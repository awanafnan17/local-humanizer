import type {
  AppStatus,
  HumanizeProgressEvent,
  HumanizeProgressState,
  ModelDownloadProgressEvent,
  RuntimeState,
} from "./types.ts";

export interface ReadinessDisplay {
  label: string;
  tone: "loading" | "ready" | "error";
  detail: string;
  canRetry: boolean;
}

export function isBusyRuntimeState(state: RuntimeState | undefined): boolean {
  return (
    state === "initializing" ||
    state === "verifying" ||
    state === "downloading" ||
    state === "starting_runtime"
  );
}

export function canStartHumanize(
  appStatus: AppStatus | null,
  inputText: string,
  isRunning: boolean,
): boolean {
  if (!appStatus || !appStatus.model_ready || isRunning) {
    return false;
  }
  return inputText.trim().length > 0;
}

export function getReadinessDisplay(
  appStatus: AppStatus | null,
  statusError: string | null,
): ReadinessDisplay {
  if (statusError) {
    return {
      label: "Error",
      tone: "error",
      detail: statusError,
      canRetry: true,
    };
  }
  if (!appStatus) {
    return {
      label: "Initializing",
      tone: "loading",
      detail: "Connecting to local runtime…",
      canRetry: false,
    };
  }

  switch (appStatus.state) {
    case "initializing":
      return {
        label: "Initializing",
        tone: "loading",
        detail: "Checking runtime environment…",
        canRetry: false,
      };
    case "verifying":
      return {
        label: "Verifying",
        tone: "loading",
        detail: "Verifying GGUF model SHA-256 integrity…",
        canRetry: false,
      };
    case "downloading":
      return {
        label: "Downloading",
        tone: "loading",
        detail: "Downloading local model artifact…",
        canRetry: false,
      };
    case "starting_runtime":
      return {
        label: "Starting Runtime",
        tone: "loading",
        detail: "Starting offline llama-server…",
        canRetry: false,
      };
    case "ready":
      if (!appStatus.model_ready) {
        return {
          label: "Starting Runtime",
          tone: "loading",
          detail: "Awaiting model health check…",
          canRetry: false,
        };
      }
      return {
        label: "Ready",
        tone: "ready",
        detail: `${appStatus.active_model_id || "Default model"} (${appStatus.backend_mode.toUpperCase()})`,
        canRetry: false,
      };
    case "error":
      return {
        label: "Error",
        tone: "error",
        detail:
          appStatus.error_message ||
          "Local inference runtime failed to initialize.",
        canRetry: true,
      };
  }
}

export function formatProgressStageLabel(
  state: HumanizeProgressState,
  chunkIndex: number,
  chunksTotal: number,
): string {
  switch (state) {
    case "segmenting":
      return "Segmenting";
    case "inferring":
      if (chunksTotal > 0) {
        const displayChunk = Math.min(Math.max(chunkIndex + 1, 1), chunksTotal);
        return `Inferring Chunk ${displayChunk} of ${chunksTotal}`;
      }
      return "Inferring";
    case "validating":
      return "Validating";
    case "retrying":
      if (chunksTotal > 0) {
        const displayChunk = Math.min(Math.max(chunkIndex + 1, 1), chunksTotal);
        return `Retrying Chunk ${displayChunk} of ${chunksTotal}`;
      }
      return "Retrying";
    case "completed":
      return "Completed";
    case "cancelled":
      return "Cancelled";
    case "error":
      return "Error";
  }
}

export interface HumanizeRunState {
  requestId: string | null;
  isRunning: boolean;
  state: HumanizeProgressState | null;
  chunkIndex: number;
  chunksTotal: number;
  partialOutput: string;
  warnings: string[];
  chunkWarningsMap: Record<number, string[]>;
  flaggedChunks: number[];
  durationMs: number | null;
  cancelled: boolean;
  errorMessage: string | null;
}

export function createInitialHumanizeRunState(): HumanizeRunState {
  return {
    requestId: null,
    isRunning: false,
    state: null,
    chunkIndex: 0,
    chunksTotal: 0,
    partialOutput: "",
    warnings: [],
    chunkWarningsMap: {},
    flaggedChunks: [],
    durationMs: null,
    cancelled: false,
    errorMessage: null,
  };
}

export function applyHumanizeProgressEvent(
  prev: HumanizeRunState,
  evt: HumanizeProgressEvent,
): HumanizeRunState {
  if (!prev.requestId || evt.request_id !== prev.requestId) {
    return prev;
  }

  const isPrevTerminal =
    prev.state === "completed" ||
    prev.state === "cancelled" ||
    prev.state === "error";
  if (isPrevTerminal) {
    return prev;
  }

  const nextChunkWarningsMap = { ...prev.chunkWarningsMap };
  const nextFlaggedSet = new Set<number>(prev.flaggedChunks);

  if (evt.chunk_warnings.length > 0) {
    nextChunkWarningsMap[evt.chunk_index] = [...evt.chunk_warnings];
    nextFlaggedSet.add(evt.chunk_index);
  }

  const isTerminal =
    evt.state === "completed" ||
    evt.state === "cancelled" ||
    evt.state === "error";

  const nextChunkIndex =
    evt.state === "inferring" && prev.state === "inferring"
      ? Math.max(prev.chunkIndex, evt.chunk_index)
      : evt.chunk_index;

  return {
    ...prev,
    isRunning: !isTerminal,
    state: evt.state,
    chunkIndex: nextChunkIndex,
    chunksTotal: evt.chunks_total,
    partialOutput: evt.partial_output || prev.partialOutput,
    chunkWarningsMap: nextChunkWarningsMap,
    flaggedChunks: Array.from(nextFlaggedSet).sort((a, b) => a - b),
    cancelled: evt.state === "cancelled" ? true : prev.cancelled,
  };
}

export function warningBelongsToChunk(
  warning: string,
  chunkIndex: number,
): boolean {
  const oneBased = new RegExp(`\\bchunk\\s+#?${chunkIndex + 1}\\b`, "i");
  if (oneBased.test(warning)) {
    return true;
  }
  if (chunkIndex === 0) {
    const zeroBased = /\bchunk\s*#?0\b/i;
    if (zeroBased.test(warning)) {
      return true;
    }
  }
  return false;
}

export function extractChunkWarningIndices(
  warnings: readonly string[],
  chunkWarningsMap: Record<number, string[]> = {},
  chunksTotal = 0,
): number[] {
  const indices = new Set<number>();
  for (const key of Object.keys(chunkWarningsMap)) {
    const idx = Number(key);
    if (
      Number.isInteger(idx) &&
      idx >= 0 &&
      (chunksTotal <= 0 || idx < chunksTotal) &&
      chunkWarningsMap[idx]?.length > 0
    ) {
      indices.add(idx);
    }
  }

  // M3 validator.rs emits 1-based warnings like "Chunk 1 closely echoed..." or "Chunk 1 of 3...".
  // Only explicit "chunk 0" / "chunk #0" maps to 0-based index 0.
  const chunkPattern = /\bchunk\s*#?(\d+)(?:\s+of\s+\d+)?\b/i;

  for (const warning of warnings) {
    const match = warning.match(chunkPattern);
    if (match) {
      const raw = Number(match[1]);
      if (Number.isInteger(raw) && raw >= 0) {
        const normalized = raw === 0 ? 0 : raw - 1;
        if (chunksTotal <= 0 || normalized < chunksTotal) {
          indices.add(normalized);
        }
      }
    }
  }

  return Array.from(indices).sort((a, b) => a - b);
}

/**
 * Align output text into `expectedCount` chunk slices so retrying chunk `k`
 * replaces only chunk `k` cleanly.
 */
export function alignOutputChunks(
  outputText: string,
  expectedCount: number,
): string[] {
  const trimmed = outputText.trim();
  if (!trimmed || expectedCount <= 1) {
    return trimmed ? [outputText] : [];
  }

  const paragraphs = trimmed
    .split(/\n\s*\n+/)
    .map((p) => p.trim())
    .filter((p) => p.length > 0);

  if (paragraphs.length === expectedCount) {
    return paragraphs;
  }

  if (paragraphs.length > expectedCount) {
    const result: string[] = [];
    const baseSize = Math.floor(paragraphs.length / expectedCount);
    const remainder = paragraphs.length % expectedCount;
    let cursor = 0;
    for (let i = 0; i < expectedCount; i += 1) {
      const take = baseSize + (i < remainder ? 1 : 0);
      result.push(paragraphs.slice(cursor, cursor + take).join("\n\n"));
      cursor += take;
    }
    return result;
  }

  // Fewer paragraphs than expected chunks: pad trailing empty slices only if needed
  const result = [...paragraphs];
  while (result.length < expectedCount) {
    result.push("");
  }
  return result;
}

export function spliceRetriedChunkOutput(
  outputChunks: readonly string[],
  chunkIndex: number,
  rewrittenChunk: string,
  fallbackOutput: string,
): { nextOutputText: string; nextChunks: string[] } {
  if (outputChunks.length === 0) {
    const replacement = rewrittenChunk || fallbackOutput;
    return {
      nextOutputText: replacement,
      nextChunks: replacement ? [replacement] : [],
    };
  }
  const nextChunks = [...outputChunks];
  if (chunkIndex < 0) {
    if (rewrittenChunk.length > 0) {
      nextChunks[0] = rewrittenChunk;
    }
  } else if (chunkIndex >= nextChunks.length) {
    if (rewrittenChunk.length > 0) {
      nextChunks.push(rewrittenChunk);
    }
  } else {
    nextChunks[chunkIndex] = rewrittenChunk;
  }
  const nextOutputText = nextChunks.filter((c) => c.length > 0).join("\n\n");
  return { nextOutputText, nextChunks };
}

export function computeProgressPercent(
  state: HumanizeProgressState | null,
  chunkIndex: number,
  chunksTotal: number,
): number {
  if (!state) return 0;
  if (state === "completed") return 100;
  if (state === "segmenting") return 8;
  if (state === "validating") return 92;
  if (state === "retrying") {
    if (chunksTotal > 0) {
      return Math.min(95, Math.max(15, Math.round(((chunkIndex + 1) / chunksTotal) * 85)));
    }
    return 75;
  }
  if (state === "inferring") {
    if (chunksTotal > 0) {
      const done = Math.min(chunksTotal, Math.max(1, chunkIndex + 1));
      return Math.min(88, Math.max(12, Math.round((done / chunksTotal) * 85)));
    }
    return 35;
  }
  return 0;
}

export function applyDownloadProgressEvent(
  prev: Record<string, ModelDownloadProgressEvent>,
  evt: ModelDownloadProgressEvent,
): Record<string, ModelDownloadProgressEvent> {
  return {
    ...prev,
    [evt.model_id]: evt,
  };
}
