# Testing & Verification Inventory

This document catalogs every reproducible test suite, command, and observed result across Milestones M1–M5, along with explicitly tracked **`PENDING`** and **`BLOCKED / UNVERIFIED`** items.

---

## 1. Verified Test Suites (M1–M5)

### 1.1 Frontend Typecheck, Unit & Playwright UI Tests (M4 / Re-verified M5)
- **Typecheck & Production Build:**
  ```bash
  npm run typecheck && npm run build
  ```
  Result: `tsc --noEmit` exit `0`; Vite production bundle built (`2150` modules).
- **Frontend Unit Tests (`src/test/unit.test.mjs`):**
  ```bash
  npm run test:unit
  ```
  Result: **`8/8` passed** (`0` failed), including `251/251` `wordCount` parity against `metrics.py`, state reducer, chunk splice, and `mockTauriIpc` contract coverage.
- **Playwright UI Suite (`src/test/ui.playwright.mjs`):**
  ```bash
  npm run test:ui
  ```
  Result: **`13/13` Playwright groups passed** (`M4-01` through `M4-19-regressions`), `0` console/page/outbound errors (executed against `mockTauriIpc.ts` browser harness).

### 1.2 Rust Backend Unit, Clippy & Integration Tests (M3 / Re-verified M5)
- **Cargo Check & Clippy:**
  ```bash
  cargo check --manifest-path src-tauri/Cargo.toml --all-targets
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
  ```
  Result: exit `0` (`0` errors; `11` pre-existing M3 clippy style warnings in `src/engine/`).
- **Rust Unit Tests (`src-tauri/src/`):**
  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml
  ```
  Result: **`69/69` unit tests passed** (`engine::` `45`, `commands::` `8`, `models::` `8`, `runtime::` `8`; `3` live integration tests ignored by default).
- **Live `llama-server` Integration (`src-tauri/tests/live_llama_integration.rs`):**
  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml --test live_llama_integration -- --ignored --nocapture --test-threads=1
  ```
  Result (M3 baseline `485.40s`, re-verified M5): **`3/3` passed** (`live_health_and_bearer_auth_401`, `live_spawn_cancel_and_reap_zero_orphans`, `live_3000_word_paraphrase_e2e` rewriting `3,766` words across `13/13` chunks with zero orphan `llama-server` processes).
- **M3 Independent Adversarial Probes:** **`2,885/2,885` assertions passed** (`2,738` engine adversarial checks including `2,592` combinatorial mode/intensity/option permutations and `3,257`-word simulation + `105` lifecycle/IPC checks + `42` UTF-8/ReDoS checks in `39 ms`).

### 1.3 Security & Integrity Audits (M1 / M5)
- **Automated Audit (`bash scripts/audit.sh`):**
  - `npm audit --audit-level=high`: `0` vulnerabilities.
  - `cargo audit --file src-tauri/Cargo.lock`: `0` errors/vulnerabilities across `511` crates (`6` pre-existing transitive warnings).
  - Byte-identical `THIRD_PARTY_NOTICES.md` & bundled license texts verified.
  - Zero banned cloud/telemetry/secret strings and zero forbidden binaries in Git.
- **Checksum Verification (`bash scripts/verify-checksums.sh`):** Verified `Qwen3-1.7B-Q4_K_M.gguf` (`1,107,409,472` bytes, SHA-256 `b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897`), macOS `arm64` runtime layout (`11` required files), and `dist-release/SHA256SUMS.txt`.

### 1.4 M5 Packaging & Live `.app` Integration Verification (macOS `arm64`)
- **Bundle & DMG (`bash scripts/package-macos.sh`):**
  - `Local Humanizer.app` (`1.07 GiB`) ad-hoc signed and verified (`codesign --verify --deep --strict`).
  - `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` bytes, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`), verified via `hdiutil attach` (contains `Local Humanizer.app` + `Applications` symlink) and `shasum -a 256 -c SHA256SUMS.txt`.
- **Live `.app` Launch, Isolation & Reaping:**
  - `lsof` confirmed `llama-server`, all `10` `.dylib` libraries, and `Qwen3-1.7B-Q4_K_M.gguf` loaded strictly from `Local Humanizer.app/Contents/Resources/resources/` (not `src-tauri/resources/`).
  - `lsof -iTCP -sTCP:LISTEN` confirmed loopback-only binding (`127.0.0.1:58160`).
  - Unauthenticated `GET /v1/models` returned HTTP `401` (`GET /health` returned `200`).
  - Second invocation of the `.app` binary preserved a single GUI process and a single `llama-server` process (`tauri-plugin-single-instance`).
  - Sending `SIGTERM` to the `.app` PID reaped `llama-server` cleanly (`ZERO_ORPHAN_LLAMA_SERVER`, `ZERO_ORPHAN_GUI`, `ZERO_RESIDUAL_SOCKETS`).

### 1.5 GitHub Actions CI & Release Workflows
- `.github/workflows/ci.yml`: `PENDING` remote GitHub Actions verification (updated after push).
- `.github/workflows/release.yml` (`workflow_dispatch`): `PENDING` remote GitHub Actions verification (updated after push).

---

## 2. `PENDING` — Milestone M6 Independent E2E Verification

- **Separate E2E Verification Track (`tests/e2e/`):** Execution of opaque-box E2E Tiers 1–4, `>=25`-document writing-quality corpus evaluation (including multiple full `3,000`-word cases), `30` adversarial scenarios, Tier-5 adversarial hardening, and formal Gates A–H release sign-off is scheduled for **M6** (`PENDING`).

---

## 3. `BLOCKED / UNVERIFIED` Hardware & Signing Gates

- **Apple Developer ID Code Signing & Notarization:** `0` Apple Developer ID identities available locally; macOS bundles are ad-hoc signed only.
- **Physical Windows (`x86_64`) Native Runtime QA:** No local Windows OS or Wine environment; Windows NSIS packaging is verified via GitHub Actions `windows-latest`, while physical Windows install, `llama-server.exe`, Win32 Job Object reaping, and uninstall remain `BLOCKED / UNVERIFIED`.
- **Bare-Metal Intel macOS (`x86_64`) Hardware QA:** Build host is Apple M2 (`arm64`); `macos-x64` `.dmg` is cross-compiled via CI without bare-metal Intel macOS execution.
