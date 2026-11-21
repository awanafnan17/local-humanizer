# Rewriting Engine Specification

The Rust rewriting engine (`src-tauri/src/engine/`) transforms UTF-8 documents (tested up to `3,766` words live in M3 and `5,508` words in M4 UI) into paraphrased text while preserving facts, citations, numbers, quotes, code fences, and display math.

```text
Input UTF-8 Text
  │
  ▼  segmenter.rs::sanitize_input()
Sanitized Text (NUL/Bidi/ANSI/C0 stripped, CRLF -> LF, outside-fence spaces collapsed)
  │
  ▼  segmenter.rs::segment_document()
[Prose | VerbatimBlock (``` / ~~~ / $$)]
  │
  ├──► VerbatimBlock ──────────────────────────────► Reconstructor (untouched)
  │
  ▼  chunker.rs::build_work_units()
Prose Chunks (240–330w target, 350w hard cap, 1500B soft byte cap)
  │
  ▼  For each chunk:
  ├──► protect.rs::extract_invariants()  (14-step exact & non-exact invariant list)
  ├──► prompts.rs::build_prompt_bundle() (mode + intensity + options + system continuity)
  ├──► llama-server /v1/chat/completions (127.0.0.1 + Bearer, 180s timeout)
  ├──► validator.rs::sanitize_model_output() (strip <think>, preambles, echoed tags)
  ├──► validator.rs::validate_chunk()
  │       ├── PASS ────────────────────────────────► Accept & update trailing_sentences
  │       └── FLAG ──► decide_retry()
  │                     ├── EchoRetry (temp 0.72, top_p 0.90, repeat_penalty 1.12)
  │                     ├── InvariantOrTruncRetry (temp 0.45, missing-item hints)
  │                     └── CombinedRetry (temp 0.60, top_p 0.88)
  │                     └── Select best via attempt_score(); emit warnings if flagged
  ▼
reconstructor.rs::Reconstructor ──► HumanizeResponse { output_text, warnings, ... }
```

---

## 1. Input Sanitization, Word Counting & Segmentation (`segmenter.rs`)

- **Single-Pass O(n) Sanitization:** Strips NUL (`\0`), Unicode Bidi overrides (`U+202A..U+202E`, `U+2066..U+2069`), ANSI CSI/OSC escape sequences, and C0 control characters (preserving `\n` and `\t`). Normalizes `\r\n` and `\r` to `\n`. Outside fenced code (` ``` ` / `~~~`) and `$$` display math blocks, collapses runs of `>4` spaces and `>1` consecutive blank lines.
- **Word Counting (`count_words`):** Matches `src/lib/wordCount.ts` (`251/251` parity against `metrics.py`): strips `\ufeff`, splits on en/em dashes (`\u{2013}`, `\u{2014}`), strips outer Markdown/quote/bracket punctuation, counts each CJK/Kana character individually, and counts adjacent non-CJK alphanumeric runs.
- **Verbatim Isolation:** Fenced code blocks and `$$ ... $$` display math blocks become `WorkUnit::Verbatim` and pass straight to `Reconstructor` without entering LLM inference.

---

## 2. Context-Aware Chunking (`chunker.rs`)

- **Targets:** `MIN_TARGET_WORDS = 240`, `MAX_TARGET_WORDS = 330`, `HARD_CAP_WORDS = 350`, `MAX_CHUNK_BYTES = 1500` (`MAX_CHUNK_BYTES * 2 = 3000` triggers giant-paragraph splitting).
- **Safe Sentence Splitting (`split_sentences_safe`):** Avoids splitting inside URLs, DOIs, decimal numbers, abbreviations, quotes, and nobiliary APA citations (e.g., `van der Berg, 2021`, `p. 42`).
- **Oversized & Unspaced Text (`split_oversized_sentence`):** Splits giant unspaced tokens and continuous CJK runs strictly on valid UTF-8 `char` boundaries.
- **Continuity Context (`trailing_sentences`):** Extracts up to `2` trailing sentences capped at `MAX_CONTEXT_WORDS = 85` words from the **accepted previous chunk's output** and injects them strictly into the `system` prompt (`prompts.rs`) as non-echoed continuity reference.
- **Minor UI vs. Backend Parity Note:** `chunker.rs` flushes a prose chunk immediately before a `VerbatimBlock`, whereas the frontend's advisory preview estimator (`segmentParagraphChunks` in `src/lib/wordCount.ts`) skips verbatim blocks without splitting across them.

---

## 3. Modes, Intensities, Options & Prompt Framing (`prompts.rs`)

### 3.1 Modes (`RewriteMode`)
`standard`, `natural`, `academic`, `professional`, `simple`, `deep`.

### 3.2 Intensities (`Intensity`) & Base Sampling

| Intensity | Temperature | `top_p` | `top_k` | `repeat_penalty` |
|---|---:|---:|---:|---:|
| `light` | `0.40` | `0.80` | `20` | `1.06` |
| `balanced` | `0.55` | `0.85` | `20` | `1.08` |
| `strong` | `0.68` | `0.90` | `30` | `1.10` |

- `max_tokens = clamp(ceil(chunk_words * 2.4), 256, 1400)`
- `INFERENCE_REQUEST_TIMEOUT = 180 s` (`src-tauri/src/engine/mod.rs`)

### 3.3 Options (`RewriteOptions`)
- `preserve_length: bool`
- `preserve_formatting: bool`
- `preserve_citations: bool`
- `preserve_quotes: bool`
- `language: String`: Rust validates **15** language codes (`en`, `en-US`, `en-GB`, `es`, `fr`, `de`, `it`, `pt`, `nl`, `pl`, `ja`, `zh`, `ko`, `ar`, `he`); the UI dropdown in `src/lib/constants.ts` currently exposes **13** of those (omitting `pl` and `he`).
- `voice`: `auto | first_person | third_person`.

### 3.4 Prompt Injection Boundary
Source chunk text is wrapped in `<SOURCE_TEXT_DATA>...</SOURCE_TEXT_DATA>` (dynamically suffixed `_0001`, `_0002`, ... if the input text contains that tag) with explicit system directives treating source content as untrusted text to paraphrase rather than instructions to follow.

---

## 4. 14-Step Invariant Protection (`protect.rs`)

`extract_invariants()` records expected spans without injecting brittle inline `__LH_PROT_` tokens into 1.7B prompts:

- **Enforced (`enforced_items`):**
  1. Inline code (`` `...` ``)
  2. Inline LaTeX (`$...$`)
  3. URLs (`https://...`)
  4. Email addresses
  5. DOIs (`10.xxxx/...`)
  6. Quoted strings (when `preserve_quotes = true`)
  7. Citations — APA parenthetical, IEEE `[1]`, narrative APA (when `preserve_citations = true`)
  8. Currencies & currency ranges (`$145`, `EUR 20–30`, negative currencies)
  9. Dates (ISO timestamps, `YYYY-MM-DD`, month-day-year)
  10. Numbers (semver `v1.2.3`, IPv4, scientific notation, p-values, numeric ranges, general numbers/percentages)
- **Advisory:** Named entities extracted in Step 14.
- **Lockstep Hyphen & Boundary Checking:** `collapse_wrapped_hyphens_pair`, `is_valid_non_exact_span`, and `contains_with_boundary` prevent line-wrap hyphen drift while rejecting sign flips (e.g., `-$50` -> `$50`), ISO date prefix matches, and `%` suffixes.

---

## 5. Output Sanitization, Validation & Retry Routing (`validator.rs`, `mod.rs`)

1. **Sanitization (`sanitize_model_output`):** Strips nested, entity-encoded, unclosed, and orphan `<think>` tags (preserving benign `<thinking>`), echoed `<SOURCE_TEXT_DATA...>` wrappers, conversational preambles (`"Here is the rewritten text:"`, etc.), and accidental copies of `prev_context`.
2. **Validation Thresholds (`validate_chunk`):**
   - **Length ratio:** With `preserve_length = true`, flags `< 0.70x` (truncation) or `> 1.45x` (expansion), with advisory warning outside `0.80..=1.25x` for `src_words >= 20`. With `preserve_length = false`, flags `< 0.55x` or `> 1.75x`.
   - **Paragraphs:** Flags `out_paragraphs != src_paragraphs` when `preserve_formatting = true`.
   - **Verbatim echo:** Computes weighted lexical distance (`0.45 * unigram + 0.40 * bigram + 0.15 * len_delta`); flags `lex_dist < 0.08` for `src_words >= 8` (and `lex_dist == 0.0` for `src_words >= 2`).
   - **Loops & corruption:** Flags `max_4gram_count >= 4` (above source), `dangling_end`, `unbalanced_delimiters`, `doubled_words`, and `leaked_placeholder` (`__LH_PROT_` or `[[CITE_`).
3. **Separate Retry Strategies (`decide_retry`):**
   - **Pure `EchoRetry`:** Raises temperature to `0.72` (`top_p = 0.90`, `top_k = 40`, `repeat_penalty = 1.12`) with an anti-copy structural paraphrase directive (fixing M2's finding that lowering temperature worsens copy attractors).
   - **`InvariantOrTruncRetry`:** Lowers temperature to `0.45` (`top_p = 0.85`, `top_k = 20`) and lists exact missing invariants or paragraph/length requirements.
   - **`CombinedRetry`:** Uses `temp = 0.60`, `top_p = 0.88`, `top_k = 30`, `repeat_penalty = 1.10`.
4. **Attempt Selection & Safe Fallback (`attempt_score`):** Lexicographically ranks `(non_empty, retention_ratio, no_trunc_or_loop, not_echo, para_ok, len_ok, lex_dist)` so invariant loss never defeats a clean rewrite. Empty output after retry falls back to the original chunk with an explicit 1-based `"Chunk N ..."` warning. Users can manually re-run any flagged chunk via `retry_chunk` (defaults to `EchoRetry` for chunks `>= 8` words).
