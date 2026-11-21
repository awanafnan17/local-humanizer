# Local Humanizer — M2 Model Evaluation & Selection Report

**Date:** 2026-11-21  
**Milestone:** M2 (Model Benchmarking, Selection, Manifest & Notices)  
**Benchmark Harness:** `benchmarks/run_benchmarks.py` + `benchmarks/tasks.json`  
**Raw Evidence:** `benchmarks/results/*.json`, `benchmarks/results/summary.json`, `benchmarks/results/summary.csv`

---

## 1. Executive Selection Decision

| Role | Model ID | GGUF Artifact | Size | License | Decision Rationale |
|---|---|---|---|---|---|
| **Bundled Default (`is_default: true`, `is_bundled: true`)** | `qwen3-1.7b-q4_k_m` | `Qwen3-1.7B-Q4_K_M.gguf` (`unsloth/Qwen3-1.7B-GGUF`) | `1,107,409,472` B (`1,056.1 MiB`) | `Apache-2.0` | Best 8 GB CPU-only + Metal reliability, `96.7%` short-task retention, `100.0%` (`38/38`) retention on 3,102-word CPU & Metal runs, genuine rewriting (`lex_dist = 0.305–0.328`), zero `<think>` leakage with `enable_thinking: false`, fits `<2 GB` NSIS & `<2 GiB` GitHub limits. |
| **Optional High-RAM Tier (`is_default: false`, `is_bundled: false`)** | `qwen3-4b-instruct-2507-q4_k_m` | `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (`unsloth/Qwen3-4B-Instruct-2507-GGUF`) | `2,497,281,120` B (`2,381.6 MiB`) | `Apache-2.0` | Highest short-task quality (`98.2%` retention, `lex_dist = 0.529`, native non-thinking). Timed out (`>120s` `/health`) on 8 GB CPU-only due to `3,037.6 MiB` footprint / `+2,498 MB` swap; exceeds 2 GB NSIS & 2 GiB GitHub limits so delivered via SHA256-verified on-demand download for `>=16 GB` RAM hosts. |
| **Permissive 8 GB Alternative (`is_default: false`)** | `qwen2.5-1.5b-instruct-q4_k_m` | `qwen2.5-1.5b-instruct-q4_k_m.gguf` (`Qwen/Qwen2.5-1.5B-Instruct-GGUF`) | `1,117,320,736` B (`1,065.6 MiB`) | `Apache-2.0` | Fast CPU throughput (`41.88 tok/s`), low RAM footprint (`1,284.5 MiB` CPU phys), but lower invariant retention (`90.95%`), `1` truncation (`0.67` ratio on Metal `task_05`), and `2` verbatim echoes. |
| **Flagged Non-Commercial (`non_commercial_warning: true`, `is_default: false`)** | `qwen2.5-3b-instruct-q4_k_m` | `qwen2.5-3b-instruct-q4_k_m.gguf` (`Qwen/Qwen2.5-3B-Instruct-GGUF`) | `2,104,932,768` B (`2,007.4 MiB`) | `Qwen Research License Agreement` | **Disqualified from production default:** non-commercial research-only license, `>2.0 GB` NSIS limit, timed out (`>120s` `/health`) on 8 GB CPU-only (`+2,142 MB` swap), and altered both verbatim quotes in `task_03` on Metal. |
| **Evaluated Only (Not in `models.json`)** | `gemma-3-1b-it-q4_k_m` | `gemma-3-1b-it-Q4_K_M.gguf` (`ggml-org/gemma-3-1b-it-GGUF`) | `806,058,240` B (`768.7 MiB`) | `Gemma Terms of Use` | Lightest footprint (`777 MiB` CPU phys), but lower retention (`86.7–90.0%`), `3/5` verbatim echoes, requires folding `system` into `user` turn, and imposes Gemma Terms / Prohibited Use Policy flow-down. |
| **Evaluated Only (Not in `models.json`)** | `smollm2-1.7b-instruct-q4_k_m` | `SmolLM2-1.7B-Instruct-Q4_K_M.gguf` (`bartowski/SmolLM2-1.7B-Instruct-GGUF`) | `1,055,609,824` B (`1,006.7 MiB`) | `Apache-2.0` | **Rejected:** `5/5` verbatim echoes (`lex_dist = 0.0039`), stripped all Markdown backticks in `task_02` (`63.6%` retention), and leaked `"Rewritten text:"` preamble in `task_03`. |

---

## 2. Test Environment & Measurement Methodology

- **Host Hardware (`sysctl`):** Apple M2 (`Mac14,2`), `8` logical CPUs (`4` performance + `4` efficiency cores), `8,589,934,592` bytes (`8.0 GiB`) unified physical RAM.
- **OS (`sw_vers`):** `macOS 27.0` (`BuildVersion 26A428`, Darwin `arm64`).
- **Inference Runtime:** `llama.cpp` `llama-server` `1.0.0-dev (build 12600, commit 29b52d398)` at `src-tauri/resources/runtime/macos-arm64/llama-server`.
- **Server Configuration:** `--host 127.0.0.1 --port <ephemeral> --api-key <random> --ctx-size 4096 --parallel 1 --threads 4 --n-gpu-layers <0 (cpu) | 99 (metal)> --no-webui`.
- **Thinking Suppression (Qwen3 1.7B):** Server started with `--reasoning off --reasoning-budget 0 --chat-template-kwargs '{"enable_thinking":false}'` plus per-request `"chat_template_kwargs": {"enable_thinking": false}` (avoiding greedy `temp=0`; using `temperature=0.45`, `top_p=0.85`, `top_k=20`, `repeat_penalty=1.08`, `seed=42`).
- **Gemma 3 Prompt Adaptation:** `gemma-3-1b-it` GGUF Jinja template rejects non-alternating roles or `system` turns; system instructions were folded into the single `user` turn.
- **Process & Memory Telemetry:** Measured natively without `/bin/ps` (blocked in sandbox) via Darwin `ctypes` `proc_pid_rusage(pid, 4, ...)` polling (`ri_resident_size`, `ri_phys_footprint`, `ri_pageins` at 250 ms intervals), `os.wait4(pid, ...)` (`ru_maxrss`, `ru_utime`, `ru_stime`, `ru_majflt`), and `sysctl -n vm.swapusage` before/after each run.
  - *Methodological note:* `gemma-3-1b-it-q4_k_m` ran before a fix replacing `Popen.poll()` with `os.wait4(pid, os.WNOHANG)` in `stop()`, so its `peak_rss_mib` and `peak_phys_footprint_mib` are exact `proc_pid_rusage` measurements while its `cpu_utilization_pct` and `major_page_faults` appear as `0.0`/`0` in `summary.json`. All other 5 models (including `qwen3-1.7b-q4_k_m` short + longdoc) have complete `os.wait4` telemetry.
- **Hardware Coverage Caveat:** Physical Windows (`x86_64`) CPU and bare-metal Intel macOS (`x86_64`) benchmarks remain `BLOCKED / UNVERIFIED` on this `arm64` macOS machine.

---

## 3. Quantitative Short-Task Benchmark Results (5 Tasks × 6 Models × CPU/Metal)

All numbers below come verbatim from `benchmarks/results/summary.json` and `benchmarks/results/summary.csv`.

| Model ID | Mode | Tasks | Load (ms) | Prompt (tok/s) | Gen (tok/s) | Mean Wall (ms) | Peak RSS (MiB) | Peak Phys Footprint (MiB) | CPU % | Swap Delta (MB) | Major Faults | Invariant Ret. | Word Ratio | Lex Dist | `<think>` Leaks | Trunc / Echo |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `qwen3-1.7b-q4_k_m` | `cpu` | 5/5 | 12,242.0 | 150.24 | 17.98 | 22,598.2 | 1,977.34 | 1,965.39 | 279.00% | +752.62 | 139,194 | **96.67%** | 0.9865 | 0.3050 | 0 | 0 / 0 |
| `qwen3-1.7b-q4_k_m` | `metal` | 5/5 | 31,436.4* | 286.34 | 21.37* | 16,481.3 | 586.03 | 874.78 | 32.41% | +1,366.56 | 76,914 | **96.67%** | 0.9908 | 0.3283 | 0 | 0 / 0 |
| `qwen3-4b-instruct-2507-q4_k_m` | `cpu` | 0/5 | TIMEOUT (`>120s`) | — | — | — | 836.41 | 3,037.57 | 35.35% | +2,497.75 | 305,516 | — | — | — | — | — |
| `qwen3-4b-instruct-2507-q4_k_m` | `metal` | 5/5 | 17,295.1 | 178.85 | 20.38 | 17,567.7 | 913.36 | 1,102.77 | 25.49% | +750.56 | 157,354 | **98.18%** | 1.0006 | 0.5290 | 0 | 0 / 0 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `cpu` | 5/5 | 10,259.9 | 180.35 | 41.88 | 8,832.3 | 1,343.03 | 1,284.52 | 310.88% | +176.25 | 131,021 | 90.95% | 0.9411 | 0.1469 | 0 | 0 / 2 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `metal` | 5/5 | 2,307.1 | 476.67 | 48.82 | 5,925.8 | 1,244.81 | 292.11 | 87.62% | +119.62 | 69,787 | 90.95% | 0.9009 | 0.2057 | 0 | 1 / 2 |
| `qwen2.5-3b-instruct-q4_k_m` | `cpu` | 0/5 | TIMEOUT (`>120s`) | — | — | — | 1,009.75 | 2,055.00 | 24.62% | +2,142.25 | 247,688 | — | — | — | — | — |
| `qwen2.5-3b-instruct-q4_k_m` | `metal` | 5/5 | 24,512.7 | 194.33 | 18.82 | 21,075.4 | 748.55 | 344.94 | 21.13% | +305.88 | 137,486 | 93.74% | 1.0244 | 0.3126 | 0 | 0 / 0 |
| `gemma-3-1b-it-q4_k_m` | `cpu` | 5/5 | 11,001.1 | 224.35 | 30.61 | 12,003.9 | 1,414.52 | 777.16 | n/a | +437.00 | n/a | 90.00% | 1.0168 | 0.1524 | 0 | 0 / 3 |
| `gemma-3-1b-it-q4_k_m` | `metal` | 5/5 | 5,937.9 | 448.38 | 17.01 | 23,185.4 | 661.45 | 233.56 | n/a | +927.56 | n/a | 86.67% | 1.0169 | 0.1687 | 0 | 0 / 3 |
| `smollm2-1.7b-instruct-q4_k_m` | `cpu` | 5/5 | 25,133.8 | 116.40 | 18.76 | 18,974.1 | 1,442.36 | 2,492.11 | 269.70% | +1,440.50 | 132,739 | 90.51% | 1.0012 | 0.0039 | 0 | 0 / 5 |
| `smollm2-1.7b-instruct-q4_k_m` | `metal` | 5/5 | 3,086.7 | 367.74 | 44.73 | 7,592.2 | 1,960.02 | 1,434.00 | 70.61% | +997.62 | 66,686 | 90.51% | 1.0012 | 0.0039 | 0 | 0 / 5 |

*\*Note on host background swap variance & `task-45` provenance:* In an earlier benchmark pass (`task-45`, prior to adding `os.wait4()` telemetry; its standalone raw JSON was superseded by the persisted `task-93` JSONs in `benchmarks/results/`), `qwen3-1.7b-q4_k_m` measured `11.05s` load / `32.97 tok/s` (CPU) and `3.33s` load / `51.60 tok/s` (Metal) before host swap grew above 16 GiB. Independent Challenger 1 live verification on `task_01_academic_apa` (`56.15 tok/s` Metal, `23.98 tok/s` CPU) corroborates that `>50 tok/s` Metal throughput is achieved when host swap pressure is low. The table above records the final `task-93` run with complete `os.wait4()` telemetry; both runs used `seed=42` and produced identical outputs.

---

## 4. Human-Reviewed Qualitative Evaluation (1–5 Scale)

Scores below were assigned by inspecting the raw generated outputs in `benchmarks/results/*.json` across all 5 short tasks (`task_01_academic_apa`, `task_02_technical_doc`, `task_03_professional_memo`, `task_04_natural_blog`, `task_05_simple_clear`) plus `long_doc_3000`.

| Model | Meaning Pres. (1–5) | Naturalness & Fluency (1–5) | Grammar & Mechanics (1–5) | Mode / Style Adherence (1–5) | Invariant & Quote Fidelity (1–5) | Long-Doc Coherence (1–5) | Observed Defects / Notes from Raw Outputs |
|---|---:|---:|---:|---:|---:|---:|---|
| **`qwen3-4b-instruct-2507-q4_k_m`** (Metal) | **5** | **5** | **5** | **5** | **4.5** | N/A (CPU OOM/timeout) | Deepest genuine rewriting (`lex_dist=0.529`), crisp simplification on `task_05` (`ratio=0.83`), preserved both quotes verbatim in `task_03`. Only normalized ISO `2026-04-15` -> `April 15, 2026` in `task_02`. Unusable on 8 GB CPU-only (`>120s` load timeout). |
| **`qwen3-1.7b-q4_k_m`** (CPU & Metal) | **4.5** | **4.5** | **5** | **4.5** | **4.5** | **4.5** | Clean rewriting across all modes (`lex_dist=0.305–0.328`), zero preamble, zero `<think>` leaks, `100%` retention on `task_01/02/03/05` and `100%` (`38/38`) on `long_doc_3000`. Minor nuance in `task_04`: converted `3.2 hours` -> `three and a half hours` and `asked for $145` -> `offered $145`. |
| **`qwen2.5-3b-instruct-q4_k_m`** (Metal only) | **4** | **4** | **4.5** | **3.5** | **3** | N/A (CPU timeout) | **Failed verbatim quote rule** on `task_03` (rewrote `"Zero patient records may leave..."` -> `"No patient records should leave..."` and `"...prior to startup."` -> `"...before startup."`), normalized `2026-04-15`, barely edited `task_04` (`lex_dist=0.089`). Disqualified by Research-Only license + CPU timeout. |
| **`qwen2.5-1.5b-instruct-q4_k_m`** (CPU & Metal) | **4** | **3.5** | **4.5** | **3** | **3.5** | N/A | Barely changed `task_01` (`lex_dist=0.0165`) and `task_04` (`verbatim_echo_count=2`), truncated `task_05` (`ratio=0.67` on Metal, missing `Oakridge` & `Section 14.2(b)`). |
| **`gemma-3-1b-it-q4_k_m`** (CPU & Metal) | **4** | **3.5** | **4.5** | **3** | **3.5** | N/A | Echoed `3/5` tasks nearly verbatim (`task_02` `ratio=1.00`, `task_03` `ratio=1.00`); on `task_04_natural_blog` (CPU & Metal) leaked conversational preamble (`"Okay, here’s the rewritten text..."`), split `3 -> 4` paragraphs, shifted paragraph 3 from present to past tense, and dropped numeric/entity invariants (`33.3%–50.0%` retention). |
| **`smollm2-1.7b-instruct-q4_k_m`** (CPU & Metal) | **3.5** | **2.5** | **4** | **1.5** | **2.5** | N/A | Copied `task_01`, `task_04`, `task_05` 100% verbatim (`lex_dist=0.0`); wrapped `task_02` in quotes and **deleted all Markdown backticks** (`0%` code span retention); leaked `"Rewritten text:\n\n"` preamble and dropped `, 2026` in `task_03`. |

---

## 5. Full 3,102-Word Chunked Long-Document Benchmark (`qwen3-1.7b-q4_k_m`)

Evaluated on `long_doc_3000` (`3,102` source words, `28` paragraphs, `38` expected invariants across APA citations, DOIs, URLs, statistics/numbers, currencies, dates, exact quotes, inline code spans, and named entities).

### Key Pipeline Engineering Finding (M2 -> M3)
- **Initial 8-chunk run (`task-93`, 320–430w chunks + inline `__LH_PROT_###__` placeholders + user-turn `[PREVIOUS CONTEXT]`):** On 1.7B weights, inline `__LH_PROT_###__` tokens caused Attempt 1 to freeze into verbatim copying (`lex_dist=0.0`), user-turn `[PREVIOUS CONTEXT]` caused 2 duplicate paragraphs (`output_paragraphs=30` vs `28`), and 441-word chunk 1 timed out at 300s under CPU-only swap pressure.
- **Refined 11-chunk pipeline (`task-125`, 240–330w chunks + unmasked natural text + continuity reference in system prompt + leading-context guard):** Eliminated `300 s` timeouts, `<think>` leaks (`0`), truncations (`0`), 4-gram repetition flags (`0`), and duplicated context paragraphs (`28/28` paragraphs, `38/38` invariants retained) across **both** CPU-only and Metal modes. **However, per-chunk inspection revealed that chunk-level verbatim echoes persisted on `2/11` CPU chunks and `3/11` Metal chunks:**
  - **CPU (`2/11` chunks = `597 / 3,102` source words = `19.2%`):** `Chunk 1` (`310w`, `lex_dist=0.0411` Attempt 1 / `0.0128` Attempt 2 -> Attempt 1 selected) and `Chunk 9` (`287w`, **100% character-for-character verbatim copy** `lex_dist=0.0` on both attempts).
  - **Metal (`3/11` chunks = `854 / 3,102` source words = `27.5%`):** `Chunk 1` (`310w`, `lex_dist=0.0519` Attempt 1 / `0.0128` Attempt 2 -> Attempt 1 selected), `Chunk 9` (`287w`, **100% verbatim copy** `lex_dist=0.0` on both attempts), and `Chunk 11` (`257w`, **100% verbatim copy** `lex_dist=0.0` on both attempts).
  - Whole-document lexical distance (`0.2002` CPU / `0.1665` Metal) passed `document_metrics.verbatim_echo_flag = False` and masked those per-chunk echoes until per-chunk `verbatim_echo_flag` tracking (`per_chunk_verbatim_echo_count`, `per_chunk_verbatim_echo_indices`) was added.
  - **Root cause observed during the run:** On Attempt 2, the M2 script *lowered* retry temperature (`0.55 -> 0.45`) and appended `strict_retry=True` (`"Double-check every ... verbatim"`), which reinforced verbatim copying (worsening Chunk 1 from `0.0411`/`0.0519` to `0.0128` and failing to break the copy attractor on Chunks 9 & 11).

| Mode | Chunks | Retries | Load (ms) | Total All-Attempt Inference Wall (Accepted Best / Server Wall) | Selected-Attempt Mean Speed (All Attempts) | Source -> Output Words | Paragraphs | Invariant Retention | `<think>` Leaks | Doc Trunc / Repeat / Doc Echo | Per-Chunk Echoes (`lex < 0.08`) | Peak RSS / Phys Footprint (MiB) | Swap Delta (Host-Wide) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **CPU (`-ngl 0 -t 4`)** | 11/11 | 2 | 13,102.6 | **`826.6 s` (13m 47s)** (`733.5 s` best / `840.2 s` server wall) | `11.86 tok/s` (`11.80 tok/s`) | `3,102 -> 3,141` (`1.013x`) | `28 / 28` | **100.0% (`38/38`)** | 0 | 0 / 0 / 0 (`lex=0.2002`) | **2/11** (`[1, 9]`; `19.2%` words) | `1,894.0` / `3,122.7` | `+144.6 MB` |
| **Metal (`-ngl 99`)** | 11/11 | 3 | 2,050.0 | **`309.6 s` (5m 10s)** (`239.1 s` best / `313.7 s` server wall) | `21.90 tok/s` (`21.56 tok/s`) | `3,102 -> 3,145` (`1.014x`) | `28 / 28` | **100.0% (`38/38`)** | 0 | 0 / 0 / 0 (`lex=0.1665`) | **3/11** (`[1, 9, 11]`; `27.5%` words) | `1,674.1` / `2,146.1` | `+2,415.9 MB`* |

*\*Note: `total_inference_wall_ms` sums all attempts (`826,580.4 ms` CPU across 13 requests; `309,570.4 ms` Metal across 14 requests), whereas `accepted_inference_wall_ms` sums only the selected `best_attempt` per chunk (`733,495.0 ms` CPU; `239,095.1 ms` Metal) and `process_metrics.server_wall_sec` includes model load (`840.176 s` CPU; `313.71 s` Metal). Metal unified memory pageins and host-wide `vm.swapusage` delta depend on simultaneous desktop memory pressure; `qwen3-1.7b-q4_k_m` stayed comfortably below the 6.8 GB 8 GB-safe limit (`~2.15–3.12 GiB` peak footprint) with zero OOM kills and `100.0%` invariant retention across all 28 paragraphs.*

### Mandatory M3 Engineering Requirement
1. **Per-chunk echo validation (`src-tauri/src/engine/validator.rs`):** Every chunk MUST be checked individually for `lex_dist >= 0.08` (and sentence-level rewrite divergence), never masked by whole-document Jaccard.
2. **Separate echo vs. invariant retry strategies:** When `verbatim_echo_flag` triggers without missing invariants, M3 MUST **raise** temperature (e.g., `0.55 -> 0.72`, `top_p=0.90`) and inject an explicit anti-copy structural paraphrase directive rather than lowering temperature to `0.45` with `strict_retry` (also updated in `benchmarks/run_benchmarks.py`).
3. **No silent verbatim passthrough:** M3 MUST never silently return a `lex_dist = 0.0` chunk without flagging an actionable warning in `chunk_warnings` / `HumanizeResponse.warnings` so the user can inspect or trigger `retry_chunk`.

---

## 6. Packaging, Bundling & Licensing Analysis

1. **NSIS & GitHub Release Size Constraints:**
   - Windows NSIS installers have a hard **2.0 GB (`2,000,000,000` byte)** uncompressed internal payload ceiling, and GitHub Release assets have a **2.0 GiB (`2,147,483,648` byte)** per-asset limit.
   - Bundling `Qwen3-1.7B-Q4_K_M.gguf` (`1,107,409,472` B = `1.03 GiB`) + `llama.cpp` `b12600` runtime (`~25–41 MB`) + Tauri app binary/assets (`~15 MB`) totals **`~1.15–1.17 GB`**, leaving `>800 MB` headroom below both NSIS and GitHub Release limits!
   - By contrast, `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (`2,497,281,120` B = `2.33 GiB`) exceeds both limits and **must** use M3's SHA256-verified resumable on-demand downloader (`is_bundled: false`).
2. **Licensing & Notices:**
   - Default (`qwen3-1.7b-q4_k_m`), High-RAM (`qwen3-4b-instruct-2507-q4_k_m`), and 8 GB Alternative (`qwen2.5-1.5b-instruct-q4_k_m`) are all **`Apache-2.0`**. Full license text is bundled at `src-tauri/resources/notices/LICENSE-Apache-2.0.txt` and documented in Section 8 of both `THIRD_PARTY_NOTICES.md` copies.
   - `qwen2.5-3b-instruct-q4_k_m` carries the **Qwen RESEARCH LICENSE AGREEMENT** (non-commercial only) and is marked `non_commercial_warning: true`, `is_default: false`, `is_bundled: false` in `src-tauri/resources/models/models.json`.

---

## 7. Reproducibility & Verification Commands

```bash
python3 -m py_compile benchmarks/run_benchmarks.py
python3 benchmarks/run_benchmarks.py --dry-run
python3 -c "import json; d=json.load(open('benchmarks/tasks.json')); print('short:', len(d['short_tasks']), 'long_words:', len(d['long_doc_3000']['text'].split()))"
python3 -c "import json, re; m=json.load(open('src-tauri/resources/models/models.json'))['models']; print(len(m), [x['id'] for x in m if x['is_default']], all(re.fullmatch(r'[0-9a-f]{64}', x['sha256']) for x in m))"
diff -u THIRD_PARTY_NOTICES.md src-tauri/resources/notices/THIRD_PARTY_NOTICES.md
```
