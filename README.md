# Local Humanizer

**A free, privacy-first, 100% offline AI writing and paraphrasing desktop application for macOS and Windows.**

Built with **Tauri v2 (Rust)**, **React 19 + TypeScript + Tailwind CSS v4**, and a bundled offline **`llama.cpp` (`b12600`)** inference runtime powered by quantized **GGUF** models (`Qwen3 1.7B Q4_K_M` bundled out of the box).

> **Project Status (`v0.1.0`):** Milestones M1–M4 (clean-room scaffold, 6-model benchmark evaluation, Rust lifecycle & long-document engine, and two-pane desktop UI) are verified; M5 packaging, CI/CD, and documentation are in progress / verified per [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md); formal M6 opaque-box E2E verification (`>=25`-document evaluation suite, 30 adversarial tests, Gates A–H) is **`PENDING`**. Local macOS builds use ad-hoc code signing only (Apple Developer ID signing/notarization is **`BLOCKED / UNVERIFIED`**); physical Windows native runtime QA and bare-metal Intel macOS hardware QA are **`BLOCKED / UNVERIFIED`**.

---

## Core Privacy & Design Guarantees

- **100% Offline Normal Rewriting:** All inference runs locally via a bundled `llama-server` child process (`llama.cpp b12600`) bound strictly to `127.0.0.1` on an ephemeral port with a per-session 256-bit bearer token.
- **Zero Cloud APIs, Accounts, or Telemetry:** No API keys, no paid tiers, no credit systems, no fictional AI-detection percentages, and no outbound requests during rewriting.
- **Zero Document Persistence:** Input and humanized documents are never logged to disk or persisted across sessions (`localStorage` stores only `"local-humanizer.theme"`; `settings.json` stores only `active_model_id` and `use_gpu`).
- **Process Ownership & Clean Reaping:** Single-instance enforcement (`tauri-plugin-single-instance`), SHA-256 integrity checks before launch, genuine `/health` readiness gating, and deterministic process-group / Windows Job Object reaping on close, cancel, or panic without affecting unrelated user processes.

---

## Key Capabilities

- **Long-Document Support (`3,000+` Words):** Paragraph-aware segmentation, verbatim preservation of fenced code blocks and `$$` display math, context-aware `240–330`-word chunking (`350`-word hard cap), and trailing continuity transfer (up to `2` sentences / `85` words in the `system` prompt). Tested up to `3,766` words live in M3 and `5,508` words in the M4 UI harness.
- **6 Writing Modes:** `Standard Humanize`, `Natural & Conversational`, `Academic & Formal`, `Professional`, `Simple & Clear`, and `Deep Paraphrase`.
- **3 Rewrite Intensities:** `Light`, `Balanced`, and `Strong`.
- **Fine-Grained Controls:**
  - Preserve length
  - Preserve formatting
  - Preserve citations (APA parenthetical, IEEE `[1]`, narrative APA)
  - Preserve exact quotes
  - Language selector (`13` languages exposed in the UI dropdown; `15` validated by the Rust backend including `pl` and `he`)
  - Voice (`Auto`, `First Person`, `Third Person`)
- **Invariant Protection & Auto-Retry:** 14-step extractor protects URLs, emails, DOIs, dates, currencies, numbers, percentages, semver, scientific notation, inline code, inline LaTeX, citations, and exact quotes. Per-chunk validation catches truncations, verbatim echoes (`lex_dist < 0.08`), 4-gram loops, and placeholder leaks, with separate `EchoRetry` (elevated temperature `0.72`) vs `InvariantOrTruncRetry` (`0.45`) strategies and one-click per-chunk manual retry.
- **Two-Pane Desktop Workspace:** Left **Original Text** editor (`Paste`, `Import .txt/.md/.markdown` up to `2 MiB`, `Clear`, live word/character counts) and right editable **Humanized Text** editor (`Copy` with feedback, `Select All`, `Save` up to `16 MiB`, word-count delta badge, light/dark theme, and keyboard shortcuts: `Cmd/Ctrl+Enter`, `Cmd/Ctrl+Shift+C`, `Cmd/Ctrl+S`, `Escape`).

---

## System Requirements

| Platform | Architecture | Acceleration | Minimum RAM (Default `Qwen3 1.7B`) | Recommended RAM (Optional `Qwen3 4B`) | Verification Status |
|---|---|---|---|---|---|
| **macOS 12.0+** | Apple Silicon (`arm64`) | Metal GPU (default) or CPU | 8 GiB | 16 GiB+ | Verified on Apple M2 (8 GiB), ad-hoc signed (`BLOCKED / UNVERIFIED` for Apple Developer ID notarization) |
| **macOS 12.0+** | Intel (`x86_64`) | CPU (`-ngl 0`) | 8 GiB | 16 GiB+ | Cross-compiled via CI (`BLOCKED / UNVERIFIED` on bare-metal Intel hardware) |
| **Windows 10/11** | `x86_64` | CPU (`-ngl 0`) | 8 GiB | 16 GiB+ | NSIS installer built via CI (`BLOCKED / UNVERIFIED` on physical Windows hardware) |

---

## Documentation Suite

| Document | Description |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 3-layer desktop stack, Tauri v2 IPC contract (`11` commands, `2` events), lifecycle & process reaping |
| [MODEL_SELECTION.md](MODEL_SELECTION.md) | M2 6-model benchmark evaluation, licensing analysis, and bundling rationale |
| [REWRITING_ENGINE.md](REWRITING_ENGINE.md) | Sanitization, chunking, 14-step invariant protection, prompt framing, validation & retry engine |
| [INSTALLATION.md](INSTALLATION.md) | End-user installation, SHA-256 verification, Gatekeeper / SmartScreen caveats, and first launch |
| [BUILD_AND_RELEASE.md](BUILD_AND_RELEASE.md) | Developer prerequisites, runtime/model fetchers, packaging scripts, and GitHub Actions CI/CD |
| [SECURITY.md](SECURITY.md) | Threat model, clean-room upstream audit (`StealthHumanizer`), loopback auth, CSP, and dependency audits |
| [TESTING.md](TESTING.md) | Reproducible test suites, verified M1–M5 results, and `PENDING` / `BLOCKED` registers |
| [PERFORMANCE.md](PERFORMANCE.md) | Measured CPU vs. Metal throughput, memory/swap telemetry, and `3,102`-word long-document benchmarks |
| [TROUBLESHOOTING.md](TROUBLESHOOTING.md) | Diagnostics for startup, SHA-256, 120s timeout, disk buffer, chunk warnings, and orphan checks |
| [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) | Complete OSS, runtime (`llama.cpp`, LLVM OpenMP), model, Cargo, and npm attributions |
| [CHANGELOG.md](CHANGELOG.md) | Chronological milestone & release history |
| [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) | Pre-release gate checklist across M1–M6 |
| [FINAL_VERIFICATION_REPORT.md](FINAL_VERIFICATION_REPORT.md) | Consolidated M1–M5 verification evidence ledger and M6 pending items |
| [benchmarks/REPORT.md](benchmarks/REPORT.md) | Raw M2 benchmark methodology, short-task tables, and long-document analysis |

---

## License

Local Humanizer is licensed under the **MIT License** — see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
