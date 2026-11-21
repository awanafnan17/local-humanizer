# Release Checklist

> **Rule:** Never mark any item `PASS` without an observed, reproducible command and exit `0` verification output.

---

## 1. Source, Legal & Hygiene Gates (M1 / M5)

- [x] `PASS` — Clean-room audit of `rudra496/StealthHumanizer` (`619feab`) with zero cloud adapters, tunnels, Google Translate leaks, hosted detectors, or telemetry (`scripts/audit.sh`).
- [x] `PASS` — `npm audit --audit-level=high` (`0` vulnerabilities).
- [x] `PASS` — `cargo audit --file src-tauri/Cargo.lock` (`0` errors/vulnerabilities across `511` crates; `6` pre-existing Tauri transitive advisories documented in `SECURITY.md`).
- [x] `PASS` — `THIRD_PARTY_NOTICES.md` byte-identical to `src-tauri/resources/notices/THIRD_PARTY_NOTICES.md` (`cmp -s`), with `LICENSE-Apache-2.0.txt`, `LICENSE-llama.cpp.txt`, and `LICENSE-LLVM-OpenMP.txt` present.
- [x] `PASS` — Zero `*.gguf`, runtime binaries, `dist-release/`, `.agents/`, or private `tests/e2e/` tracked in Git (`git ls-files` guard).

## 2. Model Selection & Benchmark Gates (M2)

- [x] `PASS` — 6 candidate models evaluated on 8 GiB Apple M2 across CPU-only and Metal (`benchmarks/REPORT.md`, `benchmarks/results/summary.json`).
- [x] `PASS` — `qwen3-1.7b-q4_k_m` (`Apache-2.0`, `1,107,409,472` B, SHA-256 `b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897`) verified on `long_doc_3000` (`3,102` words, `28/28` paragraphs, `38/38` invariants) in both CPU and Metal.
- [x] `PASS` — Non-commercial model (`qwen2.5-3b-instruct-q4_k_m`) excluded from default bundling and flagged `non_commercial_warning: true`.

## 3. Backend Engine & Lifecycle Gates (M3 / M5)

- [x] `PASS` — `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` exits `0`.
- [x] `PASS` — `cargo test --manifest-path src-tauri/Cargo.toml` passes `69/69` unit tests.
- [x] `PASS` — `live_llama_integration` (`--ignored --test-threads=1`) passes `3/3` (`455.15s` post-`lib.rs` `SIGTERM` reaper edit; `3,766` words across `13/13` chunks).

## 4. Frontend UI & Branding Gates (M4 / M5)

- [x] `PASS` — `npm run typecheck && npm run build` passes cleanly.
- [x] `PASS` — `npm run test:unit` passes `8/8`.
- [x] `PASS` — `npm run test:ui` passes `13/13` Playwright groups (`0` console/page/outbound errors).

## 5. Packaging, Public Repo & CI/CD Gates (M5)

- [x] `PASS` — `bash scripts/package-macos.sh` builds `Local Humanizer.app` (`1.07 GiB`, ad-hoc `codesign --verify --deep --strict`) and `dist-release/LocalHumanizer-macOS.dmg` (`1,119,379,933` bytes, SHA-256 `53cb62afe1076b5e358b84460545969c0b70f63c13fe2eabbe5403d57c89a44d`).
- [x] `PASS` — `bash scripts/verify-checksums.sh --require-release` verifies `SHA256SUMS.txt`.
- [x] `PASS` — Live `.app` verification confirms bundled resources inside `.app/Contents/Resources/resources/`, `127.0.0.1` binding, `/v1/models` `401` bearer enforcement, single-instance behavior, and `SIGTERM` reaping (`ZERO_ORPHAN_LLAMA_SERVER`).
- [ ] `PENDING` — Public GitHub repository `awanafnan17/local-humanizer` pushed and `.github/workflows/ci.yml` passing remotely.
- [ ] `PENDING` — `.github/workflows/release.yml` (`workflow_dispatch`) cross-platform matrix (`macos-arm64`, `macos-x64`, `windows-x64` NSIS) verified.

## 6. M6 E2E & Adversarial Release Gates

- [ ] `PENDING` — Opaque-box E2E Tiers 1–4 (`tests/e2e/`).
- [ ] `PENDING` — Reproducible `>=25`-document writing-quality evaluation suite.
- [ ] `PENDING` — `30` adversarial tests + Tier-5 Challenger-initiated hardening.
- [ ] `PENDING` — Formal Gates A–H sign-off & tagged GitHub Release (`v0.1.0`).

## 7. Platform & Signing Blocks (`BLOCKED / UNVERIFIED`)

- `BLOCKED / UNVERIFIED` — Apple Developer ID code signing & Apple notarization (`0` local signing identities).
- `BLOCKED / UNVERIFIED` — Physical Windows (`x86_64`) native install, `llama-server.exe`, Job Object reaping, and uninstall QA.
- `BLOCKED / UNVERIFIED` — Bare-metal Intel macOS (`x86_64`) hardware execution QA.
