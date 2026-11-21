# Changelog

All notable milestone deliverables and verified changes to **Local Humanizer** are documented in this file.

## [0.1.0-rc.1] - 2026-11-21 (Prerelease)

### M1 — Clean-Room Scaffold & Upstream Audit (`a9ab7a8`)
- Audited `rudra496/StealthHumanizer` (`619feab622625d1fdb2b02f922b48c6fd4e81214`, MIT) and excluded 100% of cloud LLM adapters, proxy tunnels, Google Translate calls, hosted AI detectors, Next.js server routes, Chrome extensions, disk governance logs, beacons, and scraped corpora.
- Initialized clean-room Tauri v2 + React 19 + TypeScript + Tailwind CSS v4 project with original branding/iconography, `scripts/fetch-runtime.mjs` (`llama.cpp b12600` SHA-256 pinned), MIT `LICENSE`, and `THIRD_PARTY_NOTICES.md`.

### M2 — Model Evaluation & Selection
- Benchmarked 6 quantized GGUF candidates (`qwen3-1.7b-q4_k_m`, `qwen3-4b-instruct-2507-q4_k_m`, `qwen2.5-1.5b-instruct-q4_k_m`, `qwen2.5-3b-instruct-q4_k_m`, `gemma-3-1b-it-q4_k_m`, `smollm2-1.7b-instruct-q4_k_m`) across CPU-only and Metal on an 8 GiB Apple M2 host (`benchmarks/REPORT.md`).
- Selected `qwen3-1.7b-q4_k_m` (`1.03 GiB`, `Apache-2.0`) as the bundled default and `qwen3-4b-instruct-2507-q4_k_m` (`2.33 GiB`, `Apache-2.0`) as the optional 16 GiB tier; flagged `qwen2.5-3b-instruct-q4_k_m` as non-commercial (`non_commercial_warning: true`).

### M3 — Rust Lifecycle, IPC & Long-Document Paraphrasing Engine
- Implemented single-instance lock (`tauri-plugin-single-instance`), launch SHA-256 verification, ephemeral `127.0.0.1` loopback binding with 256-bit bearer token, `/health` readiness gate, stderr redaction ring buffer, Unix `setsid`/process-group reaping, Windows `CREATE_NO_WINDOW` + Job Object reaping, and HTTPS-only `.part` Range downloader.
- Implemented full `>=3,000`-word paraphrasing engine (`6` modes × `3` intensities × `6` options, 14-step invariant protection, `<think>` suppression/stripping, per-chunk validation, separate `EchoRetry` vs `InvariantOrTruncRetry` routing, manual `retry_chunk`, and cancellation).
- Verified `69/69` Rust unit tests, `3/3` live `llama-server` integration tests (`3,766`-word document), and `2,885/2,885` independent adversarial probe assertions.

### M4 — Two-Pane Desktop UI & Branding
- Implemented responsive two-pane React UI (`Header`, `EditorPane`, `ControlsBar`, `ProgressBanner`, `SettingsModal`, `Toast`), keyboard shortcuts (`Cmd/Ctrl+Enter`, `Cmd/Ctrl+Shift+C`, `Cmd/Ctrl+S`, `Escape`), light/dark theme toggle, file import/save, and per-chunk retry splicing.
- Verified `8/8` frontend unit tests, `13/13` Playwright UI groups, and `251/251` `wordCount` parity.

### M5 — Packaging, Public Repo, CI/CD & 14-File Documentation Suite
- Fixed `scripts/fetch-runtime.mjs` `--platform` argument validation, added runtime pruning/macOS dylib deduplication, and created `scripts/fetch-model.mjs`, `scripts/package-macos.sh`, `scripts/verify-checksums.sh`, and `scripts/audit.sh`.
- Fixed `verify_runtime_layout` in `src-tauri/src/models.rs` to gate `libggml-metal.1.dylib` on `aarch64` macOS (compatibility with `macos-x64` runtime archive), and added Unix `SIGTERM`/`SIGINT` self-pipe reaping in `src-tauri/src/lib.rs` so external `SIGTERM` signals cleanly reap `llama-server`.
- Built and verified `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` bytes, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`), live `.app` bundled resource loading, single-instance locking, `/v1/models` `401` auth enforcement, and zero-orphan `SIGTERM` reaping.
- Added `.github/workflows/ci.yml` and `.github/workflows/release.yml`, updated `THIRD_PARTY_NOTICES.md` with complete direct/transitive Crate & npm disclosures, and authored the full 14-file documentation suite.

### Prerelease `v0.1.0-rc.1` Assets & Deferred Checks
- Published GitHub Prerelease [`v0.1.0-rc.1`](https://github.com/awanafnan17/local-humanizer/releases/tag/v0.1.0-rc.1) packaging CI Run [`44890685142`](https://github.com/awanafnan17/local-humanizer/actions/runs/44890685142) (`6d52c12c2d34ef893662829a2c1fb67db13537ae`) installers (`LocalHumanizer-Windows-Setup.exe`, `LocalHumanizer-macOS.dmg`, `LocalHumanizer-macOS-x64.dmg`) with verified `SHA256SUMS.txt`.
- **M6 E2E Verification (`DEFERRED / NOT COMPLETED`):** Opaque-box E2E Tiers 1–5, `>=25`-doc writing-quality eval, `30` adversarial tests, Gates A–H, and independent Victory Audit deferred for this prerelease.
- **Signing & Platform Hardware (`BLOCKED / UNVERIFIED`):** Apple Developer ID signing/notarization, physical Windows native runtime QA, and bare-metal Intel macOS hardware QA.
