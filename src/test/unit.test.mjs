import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_REWRITE_OPTIONS,
  INTENSITIES,
  REWRITE_MODES,
  SUPPORTED_LANGUAGES,
  THEME_STORAGE_KEY,
  VOICE_OPTIONS,
  formatBytes,
  formatDurationMs,
} from "../lib/constants.ts";
import {
  alignOutputChunks,
  applyDownloadProgressEvent,
  applyHumanizeProgressEvent,
  canStartHumanize,
  computeProgressPercent,
  createInitialHumanizeRunState,
  extractChunkWarningIndices,
  formatProgressStageLabel,
  getReadinessDisplay,
  spliceRetriedChunkOutput,
  warningBelongsToChunk,
} from "../lib/humanizeState.ts";
import { createMockTauriRuntime } from "../lib/mockTauriIpc.ts";
import {
  isAllowedStorageKey,
  sanitizeUserFacingError,
  validateImportContent,
  validateImportFilename,
} from "../lib/validators.ts";
import {
  computeWordDeltaPercent,
  countChars,
  countWords,
  segmentParagraphChunks,
  utf8ByteLength,
} from "../lib/wordCount.ts";

test("wordCount: handles empty, dashes, markdown punctuation, accented Latin, CJK, RTL, and 3,200+ words", () => {
  assert.equal(countWords(""), 0);
  assert.equal(countWords("   \n\t  "), 0);
  assert.equal(countWords("... --- ***"), 0);

  // Dash splitting and Markdown stripping matching metrics.py
  assert.equal(countWords("well\u2014documented 2020\u20132024 **bold** `code` [1]."), 7);

  // Accented Latin + emoji + RTL + CJK
  assert.equal(countWords("Café résumé naïve"), 3);
  assert.equal(countWords("مرحبا بالعالم"), 2);
  assert.equal(countWords("日本語テスト2026"), 7); // 6 CJK/Kana chars + "2026"
  assert.equal(countChars("Hi 🚀 café"), 9);
  assert.ok(utf8ByteLength("café") > 4);

  // 3,200-word document
  const longDoc = Array.from({ length: 3200 }, (_, i) => `word${i}`).join(" ");
  assert.equal(countWords(longDoc), 3200);
});

test("wordCount: computeWordDeltaPercent and segmentParagraphChunks", () => {
  assert.equal(computeWordDeltaPercent(0, 10), null);
  assert.equal(computeWordDeltaPercent(10, 0), null);
  assert.deepEqual(computeWordDeltaPercent(100, 105), {
    deltaWords: 5,
    deltaPercent: 5,
    formatted: "+5%",
  });
  assert.deepEqual(computeWordDeltaPercent(100, 96), {
    deltaWords: -4,
    deltaPercent: -4,
    formatted: "-4%",
  });
  assert.deepEqual(computeWordDeltaPercent(50, 50), {
    deltaWords: 0,
    deltaPercent: 0,
    formatted: "0%",
  });

  const para = Array.from({ length: 180 }, () => "alpha").join(" ");
  const doc = `${para}\n\n${para}\n\n${para}`;
  const chunks = segmentParagraphChunks(doc);
  assert.equal(chunks.length, 3);

  const shortParas = [
    Array.from({ length: 40 }, () => "one").join(" "),
    Array.from({ length: 40 }, () => "two").join(" "),
    Array.from({ length: 40 }, () => "three").join(" "),
  ].join("\n\n");
  assert.equal(segmentParagraphChunks(shortParas).length, 3);
});

test("humanizeState: readiness gating and display across all runtime states", () => {
  const baseStatus = {
    state: "ready",
    active_model_id: "qwen3-1.7b-q4_k_m",
    model_ready: true,
    backend_mode: "cpu",
    ram_bytes: 8589934592,
    error_message: null,
    runtime_version: "b12600",
  };

  assert.equal(canStartHumanize(null, "hello", false), false);
  assert.equal(canStartHumanize(baseStatus, "   ", false), false);
  assert.equal(canStartHumanize(baseStatus, "hello", true), false);
  assert.equal(
    canStartHumanize({ ...baseStatus, model_ready: false, state: "starting_runtime" }, "hello", false),
    false,
  );
  assert.equal(canStartHumanize(baseStatus, "hello", false), true);

  for (const state of [
    "initializing",
    "verifying",
    "downloading",
    "starting_runtime",
  ]) {
    const info = getReadinessDisplay({ ...baseStatus, state, model_ready: false }, null);
    assert.equal(info.tone, "loading");
    assert.equal(info.canRetry, false);
  }

  const errInfo = getReadinessDisplay(
    { ...baseStatus, state: "error", model_ready: false, error_message: "GGUF checksum mismatch" },
    null,
  );
  assert.equal(errInfo.tone, "error");
  assert.equal(errInfo.canRetry, true);
  assert.match(errInfo.detail, /GGUF checksum mismatch/);

  const readyInfo = getReadinessDisplay(baseStatus, null);
  assert.equal(readyInfo.tone, "ready");
  assert.equal(readyInfo.canRetry, false);
});

test("humanizeState: progress event reducer, chunk warning extraction, and chunk splice", () => {
  let run = {
    ...createInitialHumanizeRunState(),
    requestId: "req-1",
    isRunning: true,
    state: "segmenting",
    chunksTotal: 3,
  };

  // Ignores stale request ID
  const ignored = applyHumanizeProgressEvent(run, {
    request_id: "req-other",
    chunk_index: 1,
    chunks_total: 3,
    state: "inferring",
    partial_output: "stale",
    chunk_warnings: ["bad"],
  });
  assert.equal(ignored, run);

  run = applyHumanizeProgressEvent(run, {
    request_id: "req-1",
    chunk_index: 1,
    chunks_total: 3,
    state: "inferring",
    partial_output: "Chunk 1\n\nChunk 2",
    chunk_warnings: ["Chunk 2 of 3: low lexical shift"],
  });
  assert.equal(run.chunkIndex, 1);
  assert.deepEqual(run.flaggedChunks, [1]);
  assert.equal(formatProgressStageLabel(run.state, run.chunkIndex, run.chunksTotal), "Inferring Chunk 2 of 3");
  assert.ok(computeProgressPercent(run.state, run.chunkIndex, run.chunksTotal) > 0);

  const extracted = extractChunkWarningIndices(
    ["Chunk 3 of 3: length drift"],
    run.chunkWarningsMap,
    3,
  );
  assert.deepEqual(extracted, [1, 2]);

  const aligned = alignOutputChunks("First para.\n\nSecond para.\n\nThird para.", 3);
  assert.equal(aligned.length, 3);
  const spliced = spliceRetriedChunkOutput(aligned, 1, "Revised second para.", "");
  assert.equal(spliced.nextOutputText, "First para.\n\nRevised second para.\n\nThird para.");

  const dlMap = applyDownloadProgressEvent(
    {},
    {
      model_id: "qwen3-4b-instruct-2507-q4_k_m",
      downloaded_bytes: 500,
      total_bytes: 1000,
      status: "downloading",
      error: null,
    },
  );
  assert.equal(dlMap["qwen3-4b-instruct-2507-q4_k_m"].downloaded_bytes, 500);
});

test("validators & constants: import guards, storage key whitelist, and mode/intensity metadata", () => {
  assert.equal(validateImportFilename("essay.txt").valid, true);
  assert.equal(validateImportFilename("NOTES.MD").valid, true);
  assert.equal(validateImportFilename("readme.markdown").valid, true);
  assert.equal(validateImportFilename("report.pdf").valid, false);
  assert.equal(validateImportFilename("").valid, false);

  assert.equal(validateImportContent("Clean UTF-8 prose").valid, true);
  assert.equal(validateImportContent("bad\u0000binary").valid, false);
  assert.equal(validateImportContent("x", 3 * 1024 * 1024).valid, false);

  assert.equal(isAllowedStorageKey(THEME_STORAGE_KEY), true);
  assert.equal(isAllowedStorageKey("user_document"), false);
  assert.match(sanitizeUserFacingError(new Error("line1\nline2")), /^line1 line2$/);

  assert.deepEqual(
    REWRITE_MODES.map((m) => m.id),
    ["standard", "natural", "academic", "professional", "simple", "deep"],
  );
  assert.deepEqual(
    INTENSITIES.map((i) => i.id),
    ["light", "balanced", "strong"],
  );
  assert.deepEqual(
    VOICE_OPTIONS.map((v) => v.id),
    ["auto", "first_person", "third_person"],
  );
  assert.ok(SUPPORTED_LANGUAGES.length >= 10);
  assert.equal(formatBytes(1024 * 1024), "1.00 MB");
  assert.equal(formatDurationMs(1500), "1.5s");
});

test("mockTauriIpc: supports all 11 IPC commands, progress events, download cancel, and chunk retry", async () => {
  const runtime = createMockTauriRuntime({
    chunkDelayMs: 5,
    downloadDelayMs: 5,
    flagChunkIndex: 0,
    virtualFiles: { "/mock/sample.md": "# Draft\n\nInitial content." },
  });

  const status = await runtime.invoke("get_app_status");
  assert.equal(status.state, "ready");
  assert.equal(status.model_ready, true);

  const models = await runtime.invoke("list_models");
  assert.equal(models.length, 4);
  const qwen25_3b = models.find((m) => m.id === "qwen2.5-3b-instruct-q4_k_m");
  assert.equal(qwen25_3b?.non_commercial_warning, true);

  const progressStates = [];
  const unlistenHumanize = runtime.listen("humanize-progress", (evt) => {
    progressStates.push(evt.state);
  });

  const res = await runtime.invoke("humanize_document", {
    req: {
      request_id: "req-unit-1",
      text: "We utilize local models in order to protect privacy.",
      mode: "standard",
      intensity: "balanced",
      options: DEFAULT_REWRITE_OPTIONS,
    },
  });
  unlistenHumanize();

  assert.equal(res.cancelled, false);
  assert.ok(res.output_text.includes("use local models"));
  assert.ok(progressStates.includes("segmenting"));
  assert.ok(progressStates.includes("inferring"));
  assert.ok(progressStates.includes("completed"));
  assert.equal(res.warnings.length, 1);

  const retried = await runtime.invoke("retry_chunk", {
    req: {
      request_id: "req-unit-1",
      chunk_index: 0,
      original_chunk: "We utilize local models.",
      context_before: "",
      mode: "academic",
      intensity: "strong",
      options: DEFAULT_REWRITE_OPTIONS,
    },
  });
  assert.equal(retried.chunk_index, 0);
  assert.equal(retried.warnings.length, 0);

  // Download + select non-bundled model
  const dlStates = [];
  const unlistenDl = runtime.listen("model-download-progress", (evt) => {
    dlStates.push(evt.status);
  });
  await runtime.invoke("download_model", { modelId: "qwen3-4b-instruct-2507-q4_k_m" });
  unlistenDl();
  assert.ok(dlStates.includes("downloading"));
  assert.ok(dlStates.includes("verifying"));
  assert.ok(dlStates.includes("completed"));

  const switched = await runtime.invoke("select_model", {
    modelId: "qwen3-4b-instruct-2507-q4_k_m",
    useGpu: true,
  });
  assert.equal(switched.active_model_id, "qwen3-4b-instruct-2507-q4_k_m");
  assert.equal(switched.backend_mode, "metal");

  const imported = await runtime.invoke("import_text_file", { path: "/mock/sample.md" });
  assert.match(imported, /Initial content/);
  await runtime.invoke("save_output_file", { path: "/mock/out.txt", content: "Saved text" });
  assert.equal(runtime.getSavedFiles()["/mock/out.txt"], "Saved text");

  const reinit = await runtime.invoke("retry_runtime_init");
  assert.equal(reinit.state, "ready");
});

test("regression: 1-based M3 chunk warning parsing, warningBelongsToChunk, terminal state guard, and non-destructive OOB splice", () => {
  // M3 validator.rs emits 1-based warnings without "of N"
  assert.deepEqual(
    extractChunkWarningIndices(
      ["Chunk 1 closely echoed the source text (lexical distance 0.04 < 0.08)."],
      { 0: ["Chunk 1 closely echoed the source text."] },
      3,
    ),
    [0],
  );
  assert.deepEqual(
    extractChunkWarningIndices(
      ["Chunk 2 altered or omitted protected item(s): 2026"],
      {},
      3,
    ),
    [1],
  );
  assert.deepEqual(
    extractChunkWarningIndices(["Chunk 0 legacy", "Chunk #0 legacy"], {}, 3),
    [0],
  );

  // warningBelongsToChunk must never cross-match Chunk 1 when retrying chunkIndex=1 (Chunk 2)
  assert.equal(warningBelongsToChunk("Chunk 1 closely echoed source.", 0), true);
  assert.equal(warningBelongsToChunk("Chunk 1 closely echoed source.", 1), false);
  assert.equal(warningBelongsToChunk("Chunk 2 altered protected item.", 1), true);
  assert.equal(warningBelongsToChunk("Chunk #0 error", 0), true);
  assert.equal(warningBelongsToChunk("Chunk #0 error", 1), false);
  assert.equal(warningBelongsToChunk("Chunk 10 error", 0), false);

  // Terminal state guard in applyHumanizeProgressEvent + non-regressing chunkIndex
  let run = {
    ...createInitialHumanizeRunState(),
    requestId: "req-term",
    isRunning: true,
    state: "inferring",
    chunkIndex: 2,
    chunksTotal: 3,
  };
  run = applyHumanizeProgressEvent(run, {
    request_id: "req-term",
    chunk_index: 0,
    chunks_total: 3,
    state: "inferring",
    partial_output: "chunk0",
    chunk_warnings: [],
  });
  assert.equal(run.chunkIndex, 2);

  run = applyHumanizeProgressEvent(run, {
    request_id: "req-term",
    chunk_index: 2,
    chunks_total: 3,
    state: "completed",
    partial_output: "chunk0\n\nchunk1\n\nchunk2",
    chunk_warnings: [],
  });
  assert.equal(run.state, "completed");
  assert.equal(run.isRunning, false);

  const afterLate = applyHumanizeProgressEvent(run, {
    request_id: "req-term",
    chunk_index: 0,
    chunks_total: 3,
    state: "inferring",
    partial_output: "chunk0",
    chunk_warnings: [],
  });
  assert.equal(afterLate, run);
  assert.equal(afterLate.state, "completed");
  assert.equal(afterLate.isRunning, false);

  // Non-destructive OOB splice
  const oob = spliceRetriedChunkOutput(
    ["First chunk.", "Second chunk."],
    2,
    "Retried third chunk.",
    "First chunk.\n\nSecond chunk.",
  );
  assert.equal(
    oob.nextOutputText,
    "First chunk.\n\nSecond chunk.\n\nRetried third chunk.",
  );
  assert.deepEqual(oob.nextChunks, [
    "First chunk.",
    "Second chunk.",
    "Retried third chunk.",
  ]);
});

test("regression: countWords \\ufeff parity and segmentParagraphChunks giant-paragraph & fence parity", () => {
  assert.equal(countWords("hello\u200bworld test\ufeffword"), 2);
  assert.equal(countWords("\ufeff"), 0);
  assert.equal(countWords("\ufeffhello"), 1);

  // Giant single paragraph (>180 words) without blank lines must split at sentence boundaries
  const sentences = Array.from(
    { length: 30 },
    (_, i) =>
      `Sentence ${i + 1} explains how Dr. Smith measured a 3.14 ratio across twenty reliable local tests.`,
  );
  const giantPara = sentences.join(" ");
  assert.ok(countWords(giantPara) > 400);
  const giantChunks = segmentParagraphChunks(giantPara);
  assert.ok(giantChunks.length >= 3, `Expected >=3 chunks, got ${giantChunks.length}`);
  for (const ch of giantChunks) {
    assert.ok(countWords(ch) <= 180, `Chunk exceeded 180 words: ${countWords(ch)}`);
  }

  // Fenced code blocks and $$ display math must flush prior prose and be skipped from prose chunks
  const fencedDoc = [
    "Introductory prose paragraph.",
    "",
    "```python",
    "def helper():",
    "    return 42",
    "```",
    "",
    "$$",
    "E = mc^2",
    "$$",
    "",
    "Closing prose paragraph.",
  ].join("\n");
  const fencedChunks = segmentParagraphChunks(fencedDoc);
  assert.equal(fencedChunks.length, 2);
  assert.equal(fencedChunks[0], "Introductory prose paragraph.");
  assert.equal(fencedChunks[1], "Closing prose paragraph.");

  // Markdown ATX headings (h1-h6) must be preserved as structural blocks and excluded from prose chunks
  const mdHeadingDoc = [
    "# Title",
    "",
    "## 1. Introduction and Operational Context",
    "",
    "First prose paragraph.",
    "",
    "```python",
    "# Python comment inside code",
    "x = 1",
    "```",
    "",
    "### Subsection",
    "Second prose paragraph.",
    "",
    "#hashtag remains prose.",
  ].join("\n");
  const mdChunks = segmentParagraphChunks(mdHeadingDoc);
  assert.deepEqual(mdChunks, [
    "First prose paragraph.",
    "Second prose paragraph.",
    "#hashtag remains prose.",
  ]);
});
