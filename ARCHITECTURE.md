# Architecture

Local Humanizer is a clean-room **Tauri v2** desktop application composed of three strictly isolated layers:

```text
┌───────────────────────────────────────────────────────────────────────────┐
│  Layer 1: Embedded Native Webview (React 19 + TypeScript + Tailwind v4)  │
│  src/App.tsx, Header, EditorPane, ControlsBar, ProgressBanner, Settings  │
│  CSP: default-src 'self'; script-src 'self'; connect-src 'self' ipc:      │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ Tauri v2 Native IPC (11 commands, 2 events)
                                ▼
┌───────────────────────────────────────────────────────────────────────────┐
│  Layer 2: Rust Backend Shell (src-tauri/src/)                             │
│  - Single-instance plugin (tauri-plugin-single-instance)                  │
│  - Lifecycle & reaping (runtime.rs: setsid/pgid + Win32 Job Object)       │
│  - Model integrity & HTTPS Range downloader (models.rs: SHA-256 + .part)  │
│  - Long-document paraphrasing engine (engine/*.rs)                        │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ Loopback HTTP (127.0.0.1:<ephemeral_port>)
                                │ Authorization: Bearer <256-bit random token>
                                │ reqwest::ClientBuilder::no_proxy()
                                ▼
┌───────────────────────────────────────────────────────────────────────────┐
│  Layer 3: Bundled Offline Inference Runtime (llama.cpp b12600)            │
│  $RESOURCE/resources/runtime/<platform>/llama-server[.exe]                │
│  --host 127.0.0.1 --no-webui --cors-origins localhost --no-cors-credentials│
│  Model: Qwen3-1.7B-Q4_K_M.gguf (bundled) / optional downloaded GGUFs      │
└───────────────────────────────────────────────────────────────────────────┘
```

---

## 1. Repository Layout

- `src/`: React 19 UI components (`Header.tsx`, `EditorPane.tsx`, `ControlsBar.tsx`, `ProgressBanner.tsx`, `SettingsModal.tsx`, `Toast.tsx`), hooks (`useAppStatus.ts`, `useHumanizer.ts`, `useKeyboardShortcuts.ts`, `useTheme.ts`), state/IPC helpers (`ipcClient.ts`, `humanizeState.ts`, `wordCount.ts`, `validators.ts`, `constants.ts`, `types.ts`, `mockTauriIpc.ts`), and tests (`src/test/unit.test.mjs`, `src/test/ui.playwright.mjs`).
- `src-tauri/`:
  - `tauri.conf.json`: Window (`1280x820`, min `960x640`), strict CSP, bundle resources (`resources/runtime`, `resources/notices`, `resources/models`), macOS `minimumSystemVersion: "12.0"`, Windows NSIS `currentUser` config.
  - `capabilities/default.json`: Minimal permissions (`core:default`, `dialog:default`) scoped to `"main"`.
  - `src/lib.rs`: App setup, single-instance focus restoration, panic reap hook, window/exit cleanup.
  - `src/commands.rs`: 11 Tauri IPC commands, file import/save validation, settings persistence.
  - `src/runtime.rs`: Platform resolution, loopback port binding, 256-bit bearer token, `/health` readiness polling, stderr ring buffer with token redaction, Unix/Windows child process reaping.
  - `src/models.rs`: `models.json` catalog loading, runtime layout validation, streaming SHA-256 verification, HTTPS-only resumable `.part` model downloader.
  - `src/engine/`: `segmenter.rs`, `chunker.rs`, `protect.rs`, `prompts.rs`, `validator.rs`, `reconstructor.rs`, `mod.rs`.
- `scripts/`: Runtime fetcher (`fetch-runtime.mjs`), model fetcher (`fetch-model.mjs`), icon generator (`generate-icons.py`), macOS packaging (`package-macos.sh`), checksum verification (`verify-checksums.sh`), and security/hygiene audit (`audit.sh`).
- `benchmarks/`: M2 reproducible benchmark harness (`run_benchmarks.py`, `tasks.json`, `REPORT.md`, `results/*.json`).

---

## 2. Locked Frontend ↔ Backend IPC Contract

Defined identically in `src-tauri/src/commands.rs` and `src/lib/types.ts`.

### 2.1 Commands (`tauri::command`)

| Command | Arguments | Return Type | Purpose |
|---|---|---|---|
| `get_app_status` | none | `AppStatus` | Current runtime state, active model ID, `backend_mode`, `ram_bytes`, error message, runtime version |
| `list_models` | none | `Vec<ModelDescriptor>` | Catalog from `models.json` enriched with `is_installed` status |
| `select_model` | `model_id: String`, `use_gpu: Option<bool>` | `AppStatus` | Verifies SHA-256/layout, restarts `llama-server`, persists `settings.json` only after healthy startup |
| `download_model` | `model_id: String` | `()` | Starts background HTTPS-only `.part` download with `Range` resume and SHA-256 verification |
| `cancel_download` | `model_id: String` | `()` | Cancels active download via `CancellationToken` before final `.gguf` promotion |
| `humanize_document` | `req: HumanizeRequest` | `HumanizeResponse` | Runs full segmentation, chunking, inference, validation, retry, and reconstruction |
| `cancel_humanize` | `request_id: String` | `()` | Cancels an in-flight rewrite request |
| `retry_chunk` | `req: RetryChunkRequest` | `RetryChunkResponse` | Retries a single flagged chunk with elevated paraphrase/echo-break settings |
| `import_text_file` | `path: String` | `String` | Reads UTF-8 `.txt`/`.md`/`.markdown` (`<= 2 MiB`, strips leading BOM, rejects NUL) |
| `save_output_file` | `path: String`, `content: String` | `()` | Writes output (`<= 16 MiB`) atomically via temp file + `sync_all()` + `rename()` |
| `retry_runtime_init` | none | `AppStatus` | Re-runs startup initialization from `"error"` state |

### 2.2 Events

| Event | Payload Fields | Purpose |
|---|---|---|
| `model-download-progress` | `model_id`, `downloaded_bytes`, `total_bytes`, `status` (`"downloading" \| "verifying" \| "completed" \| "cancelled" \| "error"`), `error` | Streams model download & SHA-256 progress to Settings UI |
| `humanize-progress` | `request_id`, `chunk_index`, `chunks_total`, `state` (`"segmenting" \| "inferring" \| "validating" \| "retrying" \| "completed" \| "cancelled" \| "error"`), `partial_output`, `chunk_warnings` | Streams live chunk progress and warnings to `ProgressBanner` |

---

## 3. Runtime Lifecycle & Process Ownership

1. **Single Instance (`src-tauri/src/lib.rs`):** `tauri_plugin_single_instance::init` is registered first. Launching a second instance focuses, unminimizes, and shows the existing `"main"` window instead of spawning a second GUI or inference server.
2. **Resource Resolution (`runtime.rs`, `models.rs`):**
   - Runtime binaries resolve in order: `$RESOURCE/resources/runtime/<platform>`, `$RESOURCE/runtime/<platform>`, then compile-time `CARGO_MANIFEST_DIR/resources/runtime/<platform>` for `cargo tauri dev`.
   - Platforms: `macos-arm64`, `macos-x64`, `win-cpu-x64`.
   - `verify_runtime_layout()` checks executable bits (Unix), required co-located `.dylib`s on macOS (`10` on `aarch64` including `libggml-metal.1.dylib`; `9` on `x86_64`), and co-located `.dll` presence on Windows.
3. **Integrity Gate:** Before starting `llama-server`, `models.rs` verifies the active model's exact byte size and streaming SHA-256 (`1 MiB` buffer). Cold-start verification executes during `Initializing` (and during `select_model` when switching models) before transitioning to `StartingRuntime`.
4. **Loopback Spawn & Readiness Gate (`runtime.rs`):**
   - Reserves an ephemeral port on `127.0.0.1:0` (with up to `3` bind-race retries in `start_and_wait_into_global_slot`).
   - Generates a fresh 256-bit (64-hex-char) random bearer token (`commands.rs`).
   - Spawns `llama-server` with `--host 127.0.0.1 --port <port> --api-key <token> --model <path> --ctx-size 4096 --parallel 1 --threads <min(recommended_threads, host_cpus)> --n-gpu-layers <99_metal_or_0_cpu> --no-webui --cors-origins localhost --no-cors-credentials`, plus `--reasoning off --reasoning-budget 0 --chat-template-kwargs {"enable_thinking":false}` when `disable_thinking` is enabled.
   - Polls `GET http://127.0.0.1:<port>/health` with `Authorization: Bearer <token>` (`no_proxy()`) every `200 ms` up to `120 s` until HTTP `200` `{"status":"ok"}`.
   - Captures stderr into a bounded ring buffer (`32` lines / `4096` bytes max, lines truncated at `512` bytes, bearer token redacted as `[REDACTED]`).
5. **Deterministic Reaping:**
   - **Unix:** Child spawns in its own session via `setsid()`. Shutdown sends `SIGTERM` to `-pgid` (fallback `pid`), polls up to `3000 ms` (`50 ms` intervals), falls back to `SIGKILL`, and calls `child.wait()`.
   - **Windows:** Child spawns with `CREATE_NO_WINDOW` (`0x0800_0000`) and joins a `WindowsJobGuard` configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Shutdown calls `TerminateJobObject` + `CloseHandle` + `child.wait()`.
   - Triggered on window close, `RunEvent::ExitRequested` / `RunEvent::Exit`, Unix `SIGTERM`/`SIGINT` self-pipe signal reaper (`lib.rs`), model switching, and a process-wide panic hook (`install_panic_reap_hook()`).

---

## 4. Frontend State & Dual-Mode Client

- **`src/lib/ipcClient.ts`:** Detects native Tauri (`isTauri()`) vs browser mode, lazily importing `src/lib/mockTauriIpc.ts` only outside Tauri so production builds stay clean and Playwright UI tests can run deterministically in Chromium.
- **`src/hooks/useAppStatus.ts`:** Polls `get_app_status()` every `600 ms` while runtime/model initialization is active.
- **`src/hooks/useHumanizer.ts`:** Prevents concurrent rewrites via `inFlightRef`, tracks `activeRequestIdRef` for cancellation, reconciles per-chunk warnings via `warningBelongsToChunk`, and splices retried chunks cleanly via `spliceRetriedChunkOutput`.
- **`src/hooks/useTheme.ts`:** Manages light/dark mode using `localStorage` key `"local-humanizer.theme"` exclusively.

---

## 5. Clean-Room Provenance

Architectural planning audited `https://github.com/rudra496/StealthHumanizer` at commit `619feab622625d1fdb2b02f922b48c6fd4e81214` (MIT, Copyright (c) 2025-2026 Rudra Sarker). All cloud providers, proxy tunnels, Google Translate leaks, hosted detectors, Next.js routes, browser extensions, disk logs, image beacons, and scraped datasets were excluded 100%; only pure offline MIT algorithmic concepts (region protection, validation guards, sentence splitting, readability metrics) were adapted with attribution in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
