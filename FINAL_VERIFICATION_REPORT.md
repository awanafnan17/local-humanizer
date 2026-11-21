# Final Verification Report (Prerelease `v0.1.0-rc.1` Ledger)

> **Overall Status:** `v0.1.0-rc.1 Prerelease Published` | `M1–M5 VERIFIED` | `M6 DEFERRED / NOT COMPLETED`  
> **Version:** `0.1.0-rc.1`  
> **Date:** `2026-11-21`  
> **Repository:** `https://github.com/awanafnan17/local-humanizer`  
> **Prerelease URL:** `https://github.com/awanafnan17/local-humanizer/releases/tag/v0.1.0-rc.1`

---

## 1. Milestone Verification Ledger

| Milestone | Status | Verified Evidence Summary |
|---|---|---|
| **M1 — Clean-Room Scaffold & Audit** | `PASS` | Upstream `StealthHumanizer` (`619feab`) cloud/detector/tunnel exclusion verified; `a9ab7a8` baseline clean; MIT `LICENSE` and `THIRD_PARTY_NOTICES.md` established. |
| **M2 — Model Benchmarking & Selection** | `PASS` | 6 GGUF models evaluated on 8 GiB Apple M2 (`benchmarks/REPORT.md`, `summary.json`, `longdoc.json`). Selected `qwen3-1.7b-q4_k_m` (`Apache-2.0`, `1,107,409,472` B, `100.0%` `38/38` invariants on `3,102`-word CPU & Metal) + optional `qwen3-4b-instruct-2507-q4_k_m` (`16 GiB`). |
| **M3 — Rust Backend & Engine** | `PASS` | `cargo clippy --all-targets` exit `0`; `69/69` unit tests passed; `3/3` `live_llama_integration` passed (`455.15s` post-M5 `lib.rs` edit, `3,766`-word E2E + auth + reaping); `2,885/2,885` independent adversarial assertions passed; `0` audit vulnerabilities. |
| **M4 — Two-Pane Desktop UI** | `PASS` | `npm run typecheck && npm run build` exit `0`; `8/8` unit tests passed; `13/13` Playwright UI groups passed (`0` console/page/outbound errors); `251/251` `wordCount` parity vs `metrics.py`. |
| **M5 — Packaging, Repo, CI/CD & Docs** | `PASS` | Local `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` B, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`) built & verified; live `.app` verified (`lsof`, `127.0.0.1:58160`, `/v1/models` `401`, single-instance, `SIGTERM` `ZERO_ORPHAN_LLAMA_SERVER`); 14-file docs authored; `ci.yml` Run `44891110914` (`PASS`); `release.yml` Run `44890685142` (`PASS` across `windows-x64` NSIS, `macos-arm64`, `macos-x64` at commit `6d52c12c2d34ef893662829a2c1fb67db13537ae`). |
| **M6 — E2E Tiers 1–5 & Release Gate** | `DEFERRED / NOT COMPLETED` | Opaque-box E2E Tiers 1–4, `>=25`-doc writing-quality suite, `30` adversarial tests, Tier-5 hardening, formal Gates A–H sign-off, and independent Victory Audit were deferred at owner direction for the `v0.1.0-rc.1` prerelease handover. |

---

## 2. Local Packaging, Prerelease `v0.1.0-rc.1`, Live `.app` & CI/CD Evidence

- **GitHub Prerelease (`v0.1.0-rc.1`):** `https://github.com/awanafnan17/local-humanizer/releases/tag/v0.1.0-rc.1` (compiled binaries built at commit `6d52c12c2d34ef893662829a2c1fb67db13537ae`; commits after `6d52c12` touch only CI unit-test fixtures, `release.yml` ad-hoc signing/tag guards, and release metadata/docs with zero runtime code changes)
- **Final Verified Prerelease `v0.1.0-rc.1` Installers (`SHA256SUMS.txt`):**
  - `LocalHumanizer-Windows-Setup.exe` (`windows-x64` NSIS from CI Run [`44890685142`](https://github.com/awanafnan17/local-humanizer/actions/runs/44890685142), `1,101,619,483` bytes, SHA-256 `1a56b2df673b81da5fdb9dfd11e55f82c7ed6be446ea4e0aa176b8777bce3807`; `7zz t` `Everything is Ok`, 46 files, GGUF SHA-256 verified)
  - `LocalHumanizer-macOS.dmg` (`macos-arm64`, `1,119,379,933` bytes, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`; ad-hoc signed via `codesign --force --deep --sign -` and verified with `codesign --verify --deep --strict` + `hdiutil verify`)
  - `LocalHumanizer-macOS-x64.dmg` (`macos-x64` bundle from CI Run `44890685142`, ad-hoc signed via `codesign --force --deep --sign -` and repackaged UDZO DMG, `1,119,728,753` bytes, SHA-256 `0e415738a658d2d3b0b34f939b5a598a7e3bc9d5fa9424b92c51a0c1ae84461a`; verified with `codesign --verify --deep --strict` + `hdiutil verify`)
  - *Note on macOS bundle signatures:* CI Run `44890685142` initially emitted macOS `.app` bundles without `Contents/_CodeSignature/CodeResources` (`release.yml` previously omitted `codesign --force --deep --sign -`). Both `v0.1.0-rc.1` macOS DMGs were ad-hoc signed and verified with `codesign --verify --deep --strict`, and `.github/workflows/release.yml` was updated to perform the same ad-hoc signing step on unsigned CI runs.
- **Code Signing:** `codesign --verify -vvvv --deep --strict` -> `valid on disk`, `satisfies its Designated Requirement` (ad-hoc signature on both `macos-arm64` and `macos-x64` `.app` bundles)
- **DMG Mount Check:** `hdiutil attach` verified `Local Humanizer.app` + `Applications -> /Applications` on both `LocalHumanizer-macOS.dmg` and `LocalHumanizer-macOS-x64.dmg`
- **Live `.app` Process & Resource Verification (`macos-arm64`):**
  - `lsof` confirmed `llama-server`, all `10` `.dylib` files, and `Qwen3-1.7B-Q4_K_M.gguf` loaded from `Local Humanizer.app/Contents/Resources/resources/`
  - Loopback socket `127.0.0.1:58160 (LISTEN)` with `/v1/models` returning HTTP `401` without bearer token (`GET /health` returned `200`)
  - Second invocation preserved single `local-humanizer` + single `llama-server` PID
  - `SIGTERM` reaped `llama-server` (`ZERO_ORPHAN_LLAMA_SERVER`, `ZERO_ORPHAN_GUI`, `ZERO_RESIDUAL_SOCKETS`)
- **GitHub Actions CI (`ci.yml`, Run [`44891110914`](https://github.com/awanafnan17/local-humanizer/actions/runs/44891110914)):**
  - `Frontend (Audit, Typecheck, Build, Unit, Playwright UI, E2E)`: `PASS` (`1m1s`)
  - `Rust Backend (Check, Clippy, Test, Audit)`: `PASS` (`3m43s`)
  - `Privacy, Notices & Git Hygiene`: `PASS` (`6s`)
- **GitHub Actions Release Matrix (`release.yml`, Run [`44890685142`](https://github.com/awanafnan17/local-humanizer/actions/runs/44890685142), commit `6d52c12c2d34ef893662829a2c1fb67db13537ae` — pre-ad-hoc-re-sign raw artifact SHA-256s):**
  - `LocalHumanizer-Windows-Setup.exe` (`windows-x64` NSIS, `16m21s`, Artifact `14701943358`, SHA-256 `1a56b2df673b81da5fdb9dfd11e55f82c7ed6be446ea4e0aa176b8777bce3807`)
  - `LocalHumanizer-macOS.dmg` (`macos-arm64` unsigned CI build, `9m40s`, Artifact `14702022270`, SHA-256 `66d0836df05362e5ec8bbd35176207313e3f2972fd50d10bce048da38caff391`)
  - `LocalHumanizer-macOS-x64.dmg` (`macos-x64` unsigned CI build, `8m38s`, Artifact `14701803381`, SHA-256 `a4f000885772a269a262ce5f2e6297ba7c07d9b72e7ab64744301feb353cf816`)

---

## 3. Explicit `DEFERRED / UNVERIFIED` Register (for `v0.1.0-rc.1`)

1. M6 opaque-box E2E Tiers 1–4 (`tests/e2e/`).
2. M6 `>=25`-document writing-quality evaluation suite (including multiple full `3,000`-word cases).
3. M6 `30` adversarial test scenarios and Tier-5 Challenger-initiated hardening.
4. M6 formal Gates A–H sign-off, Victory Audit, and stable `v0.1.0` GitHub Release.

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
