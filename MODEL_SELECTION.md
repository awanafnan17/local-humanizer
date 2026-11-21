# Model Selection & Benchmark Rationale

This document summarizes the Milestone M2 evaluation of candidate small quantized GGUF models, licensing audit, packaging constraints, and final selection in `src-tauri/resources/models/models.json`.

---

## 1. Selection Summary (`models.json`)

| Role | Model ID | Filename | Exact Size | SHA-256 | License | Min RAM |
|---|---|---|---|---|---|---|
| **Bundled Default** (`is_default: true`, `is_bundled: true`) | `qwen3-1.7b-q4_k_m` | `Qwen3-1.7B-Q4_K_M.gguf` | `1,107,409,472` B (`1,056.11 MiB`) | `b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897` | `Apache-2.0` | `8 GB` |
| **Optional High-RAM Tier** (`is_bundled: false`) | `qwen3-4b-instruct-2507-q4_k_m` | `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` | `2,497,281,120` B (`2,381.59 MiB`) | `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597` | `Apache-2.0` | `16 GB` |
| **8 GB Fast Alternative** (`is_bundled: false`) | `qwen2.5-1.5b-instruct-q4_k_m` | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | `1,117,320,736` B (`1,065.56 MiB`) | `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e` | `Apache-2.0` | `8 GB` |
| **Flagged Non-Commercial** (`non_commercial_warning: true`, `is_bundled: false`) | `qwen2.5-3b-instruct-q4_k_m` | `qwen2.5-3b-instruct-q4_k_m.gguf` | `2,104,932,768` B (`2,007.42 MiB`) | `626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d` | `Qwen Research License Agreement` | `16 GB` |
| **Evaluated Only (Excluded)** | `gemma-3-1b-it-q4_k_m` | `gemma-3-1b-it-Q4_K_M.gguf` | `806,058,240` B (`768.72 MiB`) | `8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135` | `Gemma Terms of Use` | — |
| **Evaluated Only (Excluded)** | `smollm2-1.7b-instruct-q4_k_m` | `SmolLM2-1.7B-Instruct-Q4_K_M.gguf` | `1,055,609,824` B (`1,006.71 MiB`) | `77665ea4815999596525c636fbeb56ba8b080b46ae85efef4f0d986a139834d7` | `Apache-2.0` | — |

---

## 2. Quantitative Short-Task Benchmark (`benchmarks/results/summary.json`)

Measured on an **8 GiB** Apple M2 (`Mac14,2`, 8 logical CPUs, macOS `27.0`) running `llama-server b12600` (`--ctx-size 4096 --parallel 1 --threads 4 --seed 42`) across the 5 standardized tasks in `benchmarks/tasks.json`:

| Model ID | Mode | Tasks | Load (ms) | Prompt (tok/s) | Gen (tok/s) | Mean Wall (ms) | Peak RSS (MiB) | Peak Phys (MiB) | Swap Delta | Retention | Ratio | Lex Dist | `<think>` / Trunc / Echo |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `qwen3-1.7b-q4_k_m` | `cpu` | 5/5 | 12,242.0 | 150.24 | 17.98 | 22,598.2 | 1,977.34 | 1,965.39 | +752.62 MB | **96.67%** | 0.9865 | 0.3050 | 0 / 0 / 0 |
| `qwen3-1.7b-q4_k_m` | `metal` | 5/5 | 31,436.4* | 286.34 | 21.37* | 16,481.3 | 586.03 | 874.78 | +1,366.56 MB | **96.67%** | 0.9908 | 0.3283 | 0 / 0 / 0 |
| `qwen3-4b-instruct-2507-q4_k_m` | `cpu` | 0/5 | `>120s` timeout | — | — | — | 836.41 | 3,037.57 | +2,497.75 MB | — | — | — | — |
| `qwen3-4b-instruct-2507-q4_k_m` | `metal` | 5/5 | 17,295.1 | 178.85 | 20.38 | 17,567.7 | 913.36 | 1,102.77 | +750.56 MB | **98.18%** | 1.0006 | 0.5290 | 0 / 0 / 0 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `cpu` | 5/5 | 10,259.9 | 180.35 | 41.88 | 8,832.3 | 1,343.03 | 1,284.52 | +176.25 MB | 90.95% | 0.9411 | 0.1469 | 0 / 0 / 2 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `metal` | 5/5 | 2,307.1 | 476.67 | 48.82 | 5,925.8 | 1,244.81 | 292.11 | +119.62 MB | 90.95% | 0.9009 | 0.2057 | 0 / 1 / 2 |
| `qwen2.5-3b-instruct-q4_k_m` | `cpu` | 0/5 | `>120s` timeout | — | — | — | 1,009.75 | 2,055.00 | +2,142.25 MB | — | — | — | — |
| `qwen2.5-3b-instruct-q4_k_m` | `metal` | 5/5 | 24,512.7 | 194.33 | 18.82 | 21,075.4 | 748.55 | 344.94 | +305.88 MB | 93.74% | 1.0244 | 0.3126 | 0 / 0 / 0 |
| `gemma-3-1b-it-q4_k_m` | `cpu` | 5/5 | 11,001.1 | 224.35 | 30.61 | 12,003.9 | 1,414.52 | 777.16 | +437.00 MB | 90.00% | 1.0168 | 0.1524 | 0 / 0 / 3 |
| `gemma-3-1b-it-q4_k_m` | `metal` | 5/5 | 5,937.9 | 448.38 | 17.01 | 23,185.4 | 661.45 | 233.56 | +927.56 MB | 86.67% | 1.0169 | 0.1687 | 0 / 0 / 3 |
| `smollm2-1.7b-instruct-q4_k_m` | `cpu` | 5/5 | 25,133.8 | 116.40 | 18.76 | 18,974.1 | 1,442.36 | 2,492.11 | +1,440.50 MB | 90.51% | 1.0012 | 0.0039 | 0 / 0 / 5 |
| `smollm2-1.7b-instruct-q4_k_m` | `metal` | 5/5 | 3,086.7 | 367.74 | 44.73 | 7,592.2 | 1,960.02 | 1,434.00 | +997.62 MB | 90.51% | 1.0012 | 0.0039 | 0 / 0 / 5 |

*\*Host swap pressure note:* Under lower background desktop swap (`task-45` and Challenger 1 verification), `qwen3-1.7b-q4_k_m` reached `3.33 s` load / `51.60–56.15 tok/s` (Metal) and `11.05 s` load / `23.98–32.97 tok/s` (CPU). Also note `gemma-3-1b-it-q4_k_m` ran before `os.wait4()` was wired into `benchmarks/run_benchmarks.py`, so its `cpu_utilization_pct` and `major_page_faults` are `0.0`/`0` in `summary.json`.

---

## 3. Qualitative Evaluation & Rejection Reasons

1. **`qwen3-1.7b-q4_k_m` (Selected Default):** Achieved `4.5–5/5` across meaning preservation, naturalness, grammar, mode adherence, invariant fidelity, and long-doc coherence, with `100.0%` (`38/38`) invariant retention and `28/28` paragraphs on the `3,102`-word `long_doc_3000` benchmark in both CPU and Metal modes.
2. **`qwen3-4b-instruct-2507-q4_k_m` (Optional High-RAM Tier):** Achieved the highest short-task scores (`5/5` meaning/naturalness/grammar/style, `98.18%` retention, `lex_dist = 0.5290`), but timed out (`>120 s` `/health`) on 8 GiB CPU-only (`3,037.57 MiB` physical footprint, `+2,497.75 MB` swap) and exceeds installer size limits (`2.33 GiB`). Delivered on demand via SHA-256 resumable downloader.
3. **`qwen2.5-3b-instruct-q4_k_m` (Disqualified):** Carries the non-commercial `Qwen Research License Agreement` (`non_commercial_warning: true`), timed out on 8 GiB CPU-only, and modified verbatim quotes in `task_03` even on Metal.
4. **`qwen2.5-1.5b-instruct-q4_k_m` (Optional Fast Alternative):** Fast CPU generation (`41.88 tok/s`), but `2` verbatim echoes and `1` truncation (`0.67` ratio on Metal `task_05`).
5. **`gemma-3-1b-it-q4_k_m` (Excluded):** `3/5` verbatim echoes, leaked conversational preamble and split `3 -> 4` paragraphs in `task_04`, requires system-into-user prompt folding, and carries `Gemma Terms of Use` flow-down obligations.
6. **`smollm2-1.7b-instruct-q4_k_m` (Rejected):** `5/5` verbatim echoes (`lex_dist = 0.0039`), stripped Markdown backticks on `task_02`, and leaked `"Rewritten text:"` preamble on `task_03`.

---

## 4. Packaging & Thinking Suppression Constraints

- **Installer Size Limits:** Windows NSIS enforces a `2.0 GB` (`2,000,000,000` B) internal payload cap and GitHub Releases enforces a `2.0 GiB` (`2,147,483,648` B) per-asset limit. `Qwen3-1.7B-Q4_K_M.gguf` (`1,107,409,472` B) + `llama.cpp b12600` + Tauri shell stays safely under `~1.17 GB`, whereas `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (`2.33 GiB`) exceeds both caps.
- **Qwen3 `<think>` Suppression:** `qwen3-1.7b-q4_k_m` has `disable_thinking: true` in `models.json`. `runtime.rs` passes `--reasoning off --reasoning-budget 0 --chat-template-kwargs {"enable_thinking":false}`, `prompts.rs` reinforces `/no_think` + `chat_template_kwargs`, and `validator.rs` strips any residual `<think>` variants as a defense-in-depth fallback.

See [benchmarks/REPORT.md](benchmarks/REPORT.md) for full raw tables and task descriptions.
