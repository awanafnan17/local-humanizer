# Build & Release Guide

This document covers local development builds, runtime/model staging, macOS `.dmg` packaging, and GitHub Actions CI/CD workflows.

---

## 1. Prerequisites

- **Node.js:** `22+` (`npm 10+`)
- **Rust:** `1.100+` (`stable` toolchain, `clippy`, `cargo-audit`)
- **Tauri CLI:** `@tauri-apps/cli` (`2.12.6`, installed via `npm ci`)
- **Platform Native Toolchains:**
  - **macOS:** Xcode Command Line Tools (`codesign`, `hdiutil`, `shasum`)
  - **Windows:** MSVC Build Tools + WebView2 + NSIS (built via Tauri on `windows-latest`)
  - **Linux (CI check/test):** `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libgtk-3-dev`, `libxdo-dev`, `libssl-dev`

---

## 2. Fetching Runtime Binaries & Default Model

Large binaries (`llama-server`, dynamic libraries, `.gguf` models, `.dmg`/`.exe` installers) are **never committed to Git** (enforced by `.gitignore` and `.github/workflows/ci.yml`).

### 2.1 Fetch & Prune `llama.cpp` (`b12600`)
```bash
node scripts/fetch-runtime.mjs [--platform macos-arm64|macos-x64|win-cpu-x64|all] [--sync-notices] [--prune|--no-prune]
```
- Downloads pinned `llama.cpp` `b12600` release archives from `ggml-org/llama.cpp` and verifies exact byte size + SHA-256:
  - `macos-arm64`: `16,578,695` B (`9ced6f8cbce915968a865ded7718c2c4d14a69876bf9abf544533aec9b76e975`)
  - `macos-x64`: `12,364,702` B (`1a76c5393ebf51ee8e067de9042fe628303b976be639912a9bcf80feb71044af`)
  - `win-cpu-x64`: `25,012,254` B (`a31a14f1630408422228d02ff2a3fc41c06a6b3b3849d8d2782c37f0f6e30951`)
- With `--prune` (default), removes unused auxiliary CLI binaries (`llama-connect`, `llama-cli`, `llama-bench`, etc.) and materializes macOS `@rpath` `.1.dylib` libraries as regular files so `tauri-bundler` does not triple symlink copies inside `.app`.
- With `--sync-notices`, ensures `LICENSE-llama.cpp.txt` and `LICENSE-LLVM-OpenMP.txt` are synced into `src-tauri/resources/notices/`.

### 2.2 Fetch & Verify Bundled Default GGUF (`Qwen3-1.7B-Q4_K_M.gguf`)
```bash
node scripts/fetch-model.mjs --model qwen3-1.7b-q4_k_m
```
- Reads `src-tauri/resources/models/models.json`, downloads `Qwen3-1.7B-Q4_K_M.gguf` (`1,107,409,472` bytes) over HTTPS into `src-tauri/resources/models/`, verifies exact byte length and SHA-256 (`b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897`), and promotes atomically from `.part`.

---

## 3. Local Build, Audit & Packaging Commands

```bash
# 1. Security, license parity & hygiene checks
bash scripts/audit.sh

# 2. Verify model SHA-256, runtime layout & notices parity
bash scripts/verify-checksums.sh

# 3. Frontend typecheck, unit tests, production build & Playwright UI tests
npm run typecheck
npm run test:unit
npm run test:ui

# 4. Rust checks & tests
cargo check --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml

# 5. Live llama-server integration tests (requires runtime + default model)
cargo test --manifest-path src-tauri/Cargo.toml --test live_llama_integration -- --ignored --nocapture --test-threads=1

# 6. Full macOS .app + .dmg build + ad-hoc codesign + SHA256SUMS.txt
bash scripts/package-macos.sh
```

`scripts/package-macos.sh` invokes `CI=true npx tauri build --ci --bundles app,dmg` (passing `--skip-jenkins` to Tauri's `bundle_dmg.sh` to avoid non-interactive Finder AppleScript hangs), validates `.app/Contents/Resources/resources/{runtime,models,notices}`, applies `codesign --force --deep --sign -` + `codesign --verify --deep --strict`, creates `dist-release/LocalHumanizer-macOS.dmg`, and writes `dist-release/SHA256SUMS.txt`.

---

## 4. GitHub Actions CI & Release Workflows

- **`.github/workflows/ci.yml`** (triggers on `push` to `main`/`v*`, `pull_request` to `main`, `workflow_dispatch`):
  - **`frontend` (`ubuntu-latest`):** `npm ci` -> `npm audit --audit-level=high` -> `npm run typecheck` -> `npm run build` -> `npm run test:unit` -> ephemeral `npm install --no-save playwright && npx playwright install chromium --with-deps` -> `npm run test:ui` -> conditional M6 E2E suite (`tests/e2e/runner.py` when present).
  - **`rust` (`ubuntu-latest`):** Installs Tauri v2 Linux GTK/WebKit deps -> `cargo check --all-targets` -> `cargo clippy --all-targets` -> `cargo test` -> `cargo audit --file src-tauri/Cargo.lock`.
  - **`privacy-and-hygiene` (`ubuntu-latest`):** Rejects any tracked `dist-release/`, `node_modules/`, `src-tauri/target/`, `dist/`, `.agents/`, `*.gguf`, or `src-tauri/resources/runtime/(macos-|win-|linux-)`, checks `THIRD_PARTY_NOTICES.md` parity, and scans source for banned cloud endpoints or leaked credentials.
- **`.github/workflows/release.yml`** (triggers on `push` tags `v*` and `workflow_dispatch`):
  - **Build Matrix (`fail-fast: false`):**
    - `macos-arm64` (`macos-latest`, `aarch64-apple-darwin`, `--bundles dmg`) -> `LocalHumanizer-macOS.dmg`
    - `macos-x64` (`macos-latest`, `x86_64-apple-darwin`, `--bundles dmg`) -> `LocalHumanizer-macOS-x64.dmg`
    - `windows-x64` (`windows-latest`, `x86_64-pc-windows-msvc`, `--bundles nsis`) -> `LocalHumanizer-Windows-Setup.exe`
  - **`release` job (`refs/tags/v*` only):** Downloads built artifacts, generates and verifies `SHA256SUMS.txt`, and publishes a checksummed GitHub Release via `softprops/action-gh-release@v2`.

---

## 5. Payload Limits & Signing Caveats

- **Installer Size Headroom:** Bundled `Qwen3-1.7B-Q4_K_M.gguf` (`1.03 GiB`) + runtime (`~32 MiB` arm64 / `~25 MB` Windows) + Tauri binary (`~15 MB`) remains well under Windows NSIS's `2.0 GB` payload cap and GitHub Releases' `2.0 GiB` per-asset cap.
- **Code Signing & Notarization (`BLOCKED / UNVERIFIED`):** Without Apple Developer ID / notarization secrets or Windows Authenticode certificates, released `.dmg` and `.exe` installers are ad-hoc / unsigned.
