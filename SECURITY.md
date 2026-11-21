# Security & Privacy Architecture

Local Humanizer is engineered as an **offline-first, zero-telemetry desktop application**.

---

## 1. Privacy Guarantees

- **Zero Mandatory Cloud Inference:** Normal rewriting never initiates outbound internet connections. All inference executes locally in a child `llama-server` process.
- **Zero Accounts, API Keys, or Telemetry:** No user registration, no analytics beacons, no crash-reporting uploads, and no fictional AI-detection percentages.
- **Zero Document Logging or Persistence:**
  - User documents are never written to log files or disk caches.
  - `llama-server` stderr is held strictly in an in-memory `32`-line / `4096`-byte ring buffer (`runtime.rs`) with per-line truncation (`512` bytes) and `[REDACTED]` substitution for the session bearer token.
  - Frontend `localStorage` is strictly limited to `"local-humanizer.theme"`.
  - Backend `app_local_data_dir/settings.json` persists only `active_model_id` and `use_gpu` — and writes only after verified healthy model startup (`commands.rs`).

---

## 2. Clean-Room Upstream Audit (`StealthHumanizer`)

Prior to scaffolding Local Humanizer, `https://github.com/rudra496/StealthHumanizer` was audited at commit `619feab622625d1fdb2b02f922b48c6fd4e81214` (MIT, Copyright (c) 2025-2026 Rudra Sarker). The following upstream components were **100% excluded**:

- Cloud LLM provider adapters (`lib/providers.ts`, `lib/providers-litellm.ts`)
- Maintainer proxy tunnel and deployment configs (`lib/server/rudra-free.ts`, `deploy/*`)
- Hidden Google Translate round-trip network calls (`lib/academic-inversion.ts`)
- Hosted AI detectors (`lib/gptzero.ts`, `app/api/detect/`)
- Next.js server routes, Chrome extension, governance disk logs, external image beacons, scraped OpenAlex paper abstracts, and training corpora.

Automated source scans (`scripts/audit.sh` and `.github/workflows/ci.yml`) verify zero regressions or leaked credentials (`sk-live-`, `sk-ant-`, `AIza`, `AKIA`, `ghp_`, `gho_`, etc.).

---

## 3. Loopback Authentication & Webview Hardening

- **Strict Loopback Binding:** `llama-server` is spawned with `--host 127.0.0.1 --port <ephemeral_port>` — never `0.0.0.0`.
- **Per-Session Bearer Token:** `commands.rs` generates a fresh 256-bit (64-hex-character) random secret via OS CSPRNG (`rand`) on every `llama-server` launch and passes `--api-key <token>`. Unauthenticated requests to `/health` or `/v1/chat/completions` fail with HTTP `401`.
- **No WebUI & Restricted CORS:** `--no-webui --cors-origins localhost --no-cors-credentials`.
- **Proxy Bypass:** Backend HTTP calls to `127.0.0.1` construct `reqwest::ClientBuilder::new().no_proxy()` (`commands.rs`) so system HTTP proxies never intercept local inference traffic.
- **Tauri CSP (`src-tauri/tauri.conf.json`):**
  ```text
  default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost
  ```
- **Minimal Capabilities (`src-tauri/capabilities/default.json`):** Grants only `core:default` and `dialog:default` to window `"main"`.

---

## 4. Process Isolation & Reaping

- **Single Instance:** `tauri-plugin-single-instance` prevents duplicate GUI or server spawns.
- **Child-Only Termination:**
  - **Unix:** Spawns `llama-server` inside a dedicated session (`setsid()`), sends `SIGTERM` to `-pgid` (fallback `pid`), polls `3000 ms`, escalates to `SIGKILL` if needed, and calls `child.wait()`. Verified by unit test `unix_reaper_terminates_owned_process_without_affecting_decoy` and live integration tests to spare unrelated user processes.
  - **Windows:** Spawns with `CREATE_NO_WINDOW` and attaches a Win32 Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), calling `TerminateJobObject` + `CloseHandle` + `child.wait()` on exit. *(Physical Windows execution `BLOCKED / UNVERIFIED` locally.)*
  - **Signal & Panic Hooks:** A lock-free self-pipe signal reaper (`lib.rs`) intercepts Unix `SIGTERM`/`SIGINT`, and `install_panic_reap_hook()` reaps any active `llama-server` child on unexpected Rust panics.

---

## 5. Model Integrity, HTTPS Downloads & File I/O Bounds

- **Launch Integrity Gate:** Every startup and model switch verifies exact file byte length and streaming SHA-256 (`1 MiB` buffer) against `src-tauri/resources/models/models.json`.
- **Optional Model Downloader (`models.rs`):**
  - Enforces HTTPS-only initial URLs and HTTPS-only redirect targets (max `10` redirects; `downloader_rejects_non_https_redirect_target` unit-tested).
  - Pre-checks free disk space (`256 MiB` safety buffer on Unix via `statvfs`).
  - Downloads into `<filename>.gguf.part` with HTTP `Range` (`206` + `Content-Range` bounds verification; `416` or mismatched `.part` triggers clean unlink and restart).
  - Verifies final byte size and SHA-256 **before** atomic rename `.part -> .gguf`.
- **Document Import / Save Guardrails (`commands.rs`):**
  - `import_text_file`: Restricts extensions to `.txt`, `.md`, `.markdown`; max `MAX_IMPORT_BYTES = 2 MiB` (`2,097,152` B); validates strict UTF-8 (strips leading BOM `\u{feff}`); rejects NUL bytes.
  - `save_output_file`: Max `MAX_SAVE_BYTES = 16 MiB` (`16,777,216` B); writes atomically via temp file + `sync_all()` + `rename()`.
- **Prompt Injection Defense (`prompts.rs`):** Untrusted user text is wrapped in collision-safe `<SOURCE_TEXT_DATA...>` delimiters inside the `user` role, while continuity context remains in the `system` role.

---

## 6. Dependency & TLS Security

- **TLS Stack:** `reqwest 0.12` uses `default-features = false` with `rustls-tls` (`ring`) — zero `openssl-sys` or system OpenSSL dependency.
- **`npm audit --audit-level=high`:** `0` vulnerabilities.
- **`cargo audit --file src-tauri/Cargo.lock`:** `0` errors/vulnerabilities across `511` crates; `6` pre-existing Tauri/Linux-GTK transitive warnings (`chrono 0.4.45` unmaintained, `proc-macro-error 1.0.4` unmaintained, `zbus_names 4.3.5` unmaintained, `zvariant 5.15.1` unmaintained, `zvariant_derive 5.15.1` unmaintained, `glib 0.18.5` `RUSTSEC-2024-0429` Linux-only transitive via Tauri GTK bindings).

---

## 7. Reporting a Vulnerability

If you discover a security issue (such as a loopback binding bypass, child process leak, path traversal, or secret exposure), please open a private security advisory on `https://github.com/awanafnan17/local-humanizer/security/advisories` rather than posting exploit details in public issues.
