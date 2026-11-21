# Troubleshooting

This guide maps Local Humanizer's runtime, download, engine, and packaging diagnostics to their exact causes and fixes.

---

## 1. Startup & Readiness Errors (Red Status Pill + `Retry`)

When startup fails, the header pill turns red (`Error`) and reveals a **Retry** button (`retry_runtime_init()`).

| Error Message | Cause | Fix |
|---|---|---|
| `"Bundled llama-server runtime for '<platform>' not found"` or `"Required runtime library ... missing"` | Missing or corrupted `llama-server` or co-located `.dylib`/`.dll` files inside `$RESOURCE/resources/runtime/<platform>`. | Reinstall the application from the `.dmg` or `.exe`, or in a source build run `node scripts/fetch-runtime.mjs`. |
| `"Model file '...' size mismatch"` or `"SHA-256 mismatch"` | Corrupted or truncated `.gguf` file. | Reinstall the bundle for the default model, or open **Settings** and re-download the optional model. |
| `"llama-server failed to reach ready state within 120s"` | Host RAM exhaustion / heavy swap thrashing prevented `llama-server` from answering `GET /health` within `120 s` (common if `Qwen3 4B` is selected on an 8 GiB CPU-only machine). | Close memory-heavy applications, switch back to `Qwen3 1.7B (Recommended Default)`, and click **Retry**. |
| `"GPU acceleration (Metal) is not supported on this host architecture"` | `use_gpu` was requested on Intel macOS (`x86_64`) or Windows, where Metal is unavailable (`runtime.rs`). | Disable **Metal / GPU** in **Settings** (`CPU Only` mode). |

---

## 2. Optional Model Download Errors (`SettingsModal` / `models.rs`)

- **`"Insufficient disk space ... need X MiB (including 256 MiB buffer)"`:** Free up disk space before retrying (`Qwen3 4B Instruct 2507` requires `~2.33 GiB` plus the `256 MiB` safety buffer).
- **Interrupted Download / `416 Range Not Satisfiable`:** Click **Download** again. Partial downloads are stored in `<filename>.gguf.part`; invalid `.part` sizes or `416` responses automatically unlink `.part` and restart cleanly from byte `0`.
- **`"Another model download ('...') is already in progress"`:** Wait for the active download to finish or click **Cancel Download** first.
- **Offline Network Error During Optional Download:** Normal rewriting is 100% offline, while downloading unbundled models from Hugging Face requires temporary HTTPS access.

---

## 3. Import, Save & Rewriting Engine Warnings

- **Import Rejected:** `import_text_file` accepts only `.txt`, `.md`, and `.markdown` files up to **`2 MiB` (`2,097,152` bytes)** containing valid UTF-8 with zero NUL (`\0`) bytes. Split larger files or save as UTF-8 plain text/Markdown.
- **Save Rejected:** `save_output_file` enforces a **`16 MiB` (`16,777,216` bytes)** ceiling and writes atomically.
- **Per-Chunk Warnings in `ProgressBanner`:**
  - `"Chunk N closely echoed the source text..."` -> Click **Retry Flagged Chunk (N)** (invokes `retry_chunk` with `EchoRetry` parameters), or increase **Intensity** to **Strong** / switch **Mode** to **Deep Paraphrase**.
  - `"Chunk N altered or omitted protected item(s)"` -> Inspect the numbered chunk in the right pane or click **Retry Flagged Chunk (N)**.
  - `"Chunk N output appears shorter or cut off"` -> Click **Retry Flagged Chunk (N)** or disable **Preserve length** if heavy expansion/condensation is expected.
- **Unsupported Language Code:** The UI dropdown exposes `13` languages; the Rust backend validates `15` (`en`, `en-US`, `en-GB`, `es`, `fr`, `de`, `it`, `pt`, `nl`, `pl`, `ja`, `zh`, `ko`, `ar`, `he`).

---

## 4. macOS Gatekeeper & Windows SmartScreen Warnings

- **macOS (`"Local Humanizer.app is damaged or from an unidentified developer"`):** Local/CI macOS builds use ad-hoc code signing (`BLOCKED / UNVERIFIED` for Apple Developer ID notarization). In Finder, **Control-click -> Open** (or **System Settings -> Privacy & Security -> Open Anyway**), or strip quarantine after copying to `/Applications`:
  ```bash
  xattr -cr "/Applications/Local Humanizer.app"
  ```
- **Windows (`"Windows protected your PC"`):** Physical Windows signing/SmartScreen reputation is **`BLOCKED / UNVERIFIED`**. Click **More info -> Run anyway** after verifying the `.exe` SHA-256 against `SHA256SUMS.txt`.

---

## 5. Verifying Zero Orphan Inference Processes

After quitting Local Humanizer, verify no app-owned `llama-server` remains running:

- **macOS:** `pgrep -fl llama-server` (should return nothing) and `lsof -iTCP -sTCP:LISTEN -P -n | grep llama-server` (should return nothing).
- **Windows (PowerShell):** `Get-Process llama-server -ErrorAction SilentlyContinue`.
