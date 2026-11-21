# Final Verification Report (Pre-M6 Status Ledger)

> **Overall Status:** `M1–M4 VERIFIED` | `M5 IN PROGRESS (LOCAL PACKAGING VERIFIED; CI VERIFICATION PENDING)` | `M6 E2E PENDING`  
> **Version:** `0.1.0`  
> **Date:** `2026-11-21`

---

## 1. Milestone Verification Ledger

| Milestone | Status | Verified Evidence Summary |
|---|---|---|
| **M1 — Clean-Room Scaffold & Audit** | `PASS` | Upstream `StealthHumanizer` (`619feab`) cloud/detector/tunnel exclusion verified; `a9ab7a8` baseline clean; MIT `LICENSE` and `THIRD_PARTY_NOTICES.md` established. |
| **M2 — Model Benchmarking & Selection** | `PASS` | 6 GGUF models evaluated on 8 GiB Apple M2 (`benchmarks/REPORT.md`, `summary.json`, `longdoc.json`). Selected `qwen3-1.7b-q4_k_m` (`Apache-2.0`, `1,107,409,472` B, `100.0%` `38/38` invariants on `3,102`-word CPU & Metal) + optional `qwen3-4b-instruct-2507-q4_k_m` (`16 GiB`). |
| **M3 — Rust Backend & Engine** | `PASS` | `cargo clippy --all-targets` exit `0`; `69/69` unit tests passed; `3/3` `live_llama_integration` passed (`3,766`-word E2E + auth + reaping); `2,885/2,885` independent adversarial assertions passed; `0` audit vulnerabilities. |
| **M4 — Two-Pane Desktop UI** | `PASS` | `npm run typecheck && npm run build` exit `0`; `8/8` unit tests passed; `13/13` Playwright UI groups passed (`0` console/page/outbound errors); `251/251` `wordCount` parity vs `metrics.py`. |
| **M5 — Packaging, Repo, CI/CD & Docs** | `IN PROGRESS` | `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` B, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`) built & verified; live `.app` verified (`lsof`, `127.0.0.1:58160`, `/v1/models` `401`, single-instance, `SIGTERM` `ZERO_ORPHAN_LLAMA_SERVER`); 14-file docs authored. Remote GitHub push & CI/release workflow runs `PENDING`. |
| **M6 — E2E Tiers 1–5 & Release Gate** | `PENDING` | Opaque-box E2E Tiers 1–4, `>=25`-doc writing-quality suite, `30` adversarial tests, Tier-5 hardening, and Gates A–H deferred to M6. |

---

## 2. Local Packaging & Live `.app` Evidence (M5)

- **Artifact:** `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` bytes)
- **SHA-256:** `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d` (`shasum -a 256 -c dist-release/SHA256SUMS.txt` -> `OK`)
- **Code Signing:** `codesign --verify --deep --strict "src-tauri/target/release/bundle/macos/Local Humanizer.app"` -> exit `0` (ad-hoc signature)
- **DMG Mount Check:** `hdiutil attach` verified `Local Humanizer.app` + `Applications -> /Applications`
- **Live `.app` Process & Resource Verification:**
  - `lsof` confirmed `llama-server`, all `10` `.dylib` files, and `Qwen3-1.7B-Q4_K_M.gguf` loaded from `Local Humanizer.app/Contents/Resources/resources/`
  - Loopback socket `127.0.0.1:58160 (LISTEN)` with `/v1/models` returning HTTP `401` without bearer token
  - Second invocation preserved single `local-humanizer` + single `llama-server` PID
  - `SIGTERM` reaped `llama-server` (`ZERO_ORPHAN_LLAMA_SERVER`, `ZERO_ORPHAN_GUI`, `ZERO_RESIDUAL_SOCKETS`)

---

## 3. Explicit `PENDING` Register

1. Remote GitHub push (`awanafnan17/local-humanizer`), `.github/workflows/ci.yml` verification, and `.github/workflows/release.yml` dispatch verification (Step 5 of M5).
2. M6 opaque-box E2E Tiers 1–4 (`tests/e2e/`).
3. M6 `>=25`-document writing-quality evaluation suite (including multiple full `3,000`-word cases).
4. M6 `30` adversarial test scenarios and Tier-5 Challenger-initiated hardening.
5. M6 formal Gates A–H sign-off and tagged GitHub Release (`v0.1.0`).

---

## 4. Explicit `BLOCKED / UNVERIFIED` Register

1. **Apple Developer ID Code Signing & Notarization:** `0` valid signing identities on host; macOS `.app`/`.dmg` are ad-hoc signed only.
2. **Physical Windows (`x86_64`) Native Runtime QA:** No Windows OS/Wine locally; CI builds `LocalHumanizer-Windows-Setup.exe` via `windows-latest`, while physical install, `llama-server.exe`, Win32 Job Object reaping, and clean uninstall remain unverified on real Windows hardware.
3. **Bare-Metal Intel macOS (`x86_64`) Hardware QA:** Build host is Apple Silicon (`arm64`); `macos-x64` `.dmg` is cross-compiled via CI without physical Intel Mac execution.

---

## 5. Known Non-Blocking Nuances

- `src/lib/constants.ts` exposes `13` languages in the UI dropdown while `src-tauri/src/engine/prompts.rs` supports `15` (`pl` and `he` omitted from UI dropdown).
- `RuntimeState::Verifying` exists in the IPC enum while cold-start SHA-256 verification runs inside `Initializing` (and `Ready` during explicit `select_model`) before `StartingRuntime`.
- `src/lib/wordCount.ts` (`segmentParagraphChunks`) skips verbatim blocks without splitting across them, whereas `src-tauri/src/engine/chunker.rs` flushes a chunk prior to a `VerbatimBlock`.
- `cargo audit --file src-tauri/Cargo.lock` reports `6` pre-existing transitive warnings (`0` errors/vulnerabilities; `glib` `RUSTSEC-2024-0429` is Linux-GTK transitive only).
