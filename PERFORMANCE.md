# Performance & Memory Benchmarks

All measurements below were collected on the reference **8 GiB** baseline machine (`Mac14,2` Apple M2, 8 logical CPUs: 4 performance + 4 efficiency, `8,589,934,592` B unified physical RAM, `macOS 27.0` Build `26A428`) running `llama-server b12600` (`--ctx-size 4096 --parallel 1 --threads 4`).

---

## 1. Telemetry Methodology

`benchmarks/run_benchmarks.py` records process metrics natively using Darwin `proc_pid_rusage` (`ri_resident_size`, `ri_phys_footprint` sampled at `250 ms` intervals), `os.wait4(pid, ...)` (`ru_maxrss`, user/system CPU time, `ru_majflt`), and host-wide `sysctl -n vm.swapusage` deltas before and after each benchmark run.

---

## 2. Short-Task Performance Summary (`benchmarks/results/summary.json`)

| Model ID | Mode | Tasks Passed | Load (ms) | Prompt (tok/s) | Gen (tok/s) | Mean Wall (ms) | Peak RSS (MiB) | Peak Phys Footprint (MiB) | CPU % | Swap Delta (MB) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `qwen3-1.7b-q4_k_m` | `cpu` | 5/5 | 12,242.0 | 150.24 | 17.98 | 22,598.2 | 1,977.34 | 1,965.39 | 279.00% | +752.62 |
| `qwen3-1.7b-q4_k_m` | `metal` | 5/5 | 31,436.4* | 286.34 | 21.37* | 16,481.3 | 586.03 | 874.78 | 32.41% | +1,366.56 |
| `qwen3-4b-instruct-2507-q4_k_m` | `cpu` | 0/5 | `>120,000` (timeout) | — | — | — | 836.41 | 3,037.57 | 35.35% | +2,497.75 |
| `qwen3-4b-instruct-2507-q4_k_m` | `metal` | 5/5 | 17,295.1 | 178.85 | 20.38 | 17,567.7 | 913.36 | 1,102.77 | 25.49% | +750.56 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `cpu` | 5/5 | 10,259.9 | 180.35 | 41.88 | 8,832.3 | 1,343.03 | 1,284.52 | 310.88% | +176.25 |
| `qwen2.5-1.5b-instruct-q4_k_m` | `metal` | 5/5 | 2,307.1 | 476.67 | 48.82 | 5,925.8 | 1,244.81 | 292.11 | 87.62% | +119.62 |
| `qwen2.5-3b-instruct-q4_k_m` | `cpu` | 0/5 | `>120,000` (timeout) | — | — | — | 1,009.75 | 2,055.00 | 24.62% | +2,142.25 |
| `qwen2.5-3b-instruct-q4_k_m` | `metal` | 5/5 | 24,512.7 | 194.33 | 18.82 | 21,075.4 | 748.55 | 344.94 | 21.13% | +305.88 |
| `gemma-3-1b-it-q4_k_m` | `cpu` | 5/5 | 11,001.1 | 224.35 | 30.61 | 12,003.9 | 1,414.52 | 777.16 | n/a | +437.00 |
| `gemma-3-1b-it-q4_k_m` | `metal` | 5/5 | 5,937.9 | 448.38 | 17.01 | 23,185.4 | 661.45 | 233.56 | n/a | +927.56 |
| `smollm2-1.7b-instruct-q4_k_m` | `cpu` | 5/5 | 25,133.8 | 116.40 | 18.76 | 18,974.1 | 1,442.36 | 2,492.11 | 269.70% | +1,440.50 |
| `smollm2-1.7b-instruct-q4_k_m` | `metal` | 5/5 | 3,086.7 | 367.74 | 44.73 | 7,592.2 | 1,960.02 | 1,434.00 | 70.61% | +997.62 |

*\*Host swap variance:* When host desktop memory pressure was low (`task-45` and Challenger 1 verification), `qwen3-1.7b-q4_k_m` achieved `3.33 s` load / `51.60–56.15 tok/s` (Metal) and `11.05 s` load / `23.98–32.97 tok/s` (CPU).

### Why 3B and 4B Models Fail on 8 GiB CPU-Only
Both `qwen3-4b-instruct-2507-q4_k_m` (`3,037.57 MiB` physical footprint, `+2,497.75 MB` swap, `305,516` major page faults) and `qwen2.5-3b-instruct-q4_k_m` (`2,055.00 MiB` physical footprint, `+2,142.25 MB` swap, `247,688` major faults) exhausted memory headroom during CPU-only `mmap`/tensor initialization and exceeded the `120 s` `/health` readiness timeout. By contrast, `qwen3-1.7b-q4_k_m` stayed comfortably below the `8 GiB` boundary (`~0.87–1.97 GiB` short-task footprint) with zero OOM terminations.

---

## 3. Full 3,102-Word Document Benchmark (`long_doc_3000`, `qwen3-1.7b-q4_k_m`)

Evaluated on `long_doc_3000` (`3,102` source words, `28` paragraphs, `38` invariants) across `11` chunks:

| Metric | CPU (`-ngl 0 -t 4`) | Metal (`-ngl 99`) |
|---|---:|---:|
| Chunks Completed | `11 / 11` (`2` retries, `13` calls) | `11 / 11` (`3` retries, `14` calls) |
| Model Load Latency | `13,102.6 ms` | `2,050.0 ms` |
| Accepted-Best Inference Sum (`accepted_inference_wall_ms`) | `733.5 s` (`733,495.0 ms`) | `239.1 s` (`239,095.1 ms`) |
| All-Attempt Inference Wall (`total_inference_wall_ms`) | `826.6 s` (`13m 47s`) | `309.6 s` (`5m 10s`) |
| End-to-End Server Wall (`process_metrics.server_wall_sec`) | `840.2 s` | `313.7 s` |
| Selected-Attempt Speed (All Attempts) | `11.86 tok/s` (`11.80 tok/s`) | `21.90 tok/s` (`21.56 tok/s`) |
| Source -> Output Words | `3,102 -> 3,141` (`1.0126x`) | `3,102 -> 3,145` (`1.0139x`) |
| Paragraphs Preserved | `28 / 28` | `28 / 28` |
| Invariants Retained | `38 / 38` (`100.0%`) | `38 / 38` (`100.0%`) |
| `<think>` Leaks / Truncations / 4-gram Loops | `0 / 0 / 0` | `0 / 0 / 0` |
| Peak RSS / Physical Footprint | `1,894.02 MiB` / `3,122.68 MiB` | `1,674.08 MiB` / `2,146.08 MiB` |
| CPU Utilization / Host Swap Delta | `229.4%` / `+144.62 MB` | `69.53%` / `+2,415.94 MB` |

- **M2 -> M3 Optimization:** M2's `longdoc` run exposed `2/11` CPU and `3/11` Metal per-chunk echoes (`Chunk 1`, `Chunk 9`, `Chunk 11`) caused by lowering retry temperature (`0.55 -> 0.45`). M3 fixed this by routing pure echoes to `EchoRetry` (`temp = 0.72`, `top_p = 0.90`, `repeat_penalty = 1.12`) with an anti-copy structural directive.
- **Live M3 Rust Engine E2E (`live_llama_integration`):** Rewrote a `3,766`-word document (`3,766 -> 3,929` words across `13/13` chunks with preserved Rust code fence and `>=87.5%` invariants) alongside auth and cancel/reap tests in `485.40 s` total.

---

## 4. Platform Performance Caveats

- **Windows `x86_64` CPU & Intel macOS `x86_64`:** Real hardware throughput, RAM footprint, and `3,000`-word timings on Windows and bare-metal Intel macOS remain **`BLOCKED / UNVERIFIED`** without physical target hardware.
