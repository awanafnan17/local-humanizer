# Third-Party Notices

This document provides copyright notices, licensing terms, and provenance disclosures for **Local Humanizer** and the third-party software components included with or used to build it.

---

## 1. Local Humanizer

**License:** MIT License  
**Copyright:** Copyright (c) 2026 Local Humanizer Contributors  

See the root `LICENSE` file for the full MIT License text.

---

## 2. Upstream Audit & Clean-Room Provenance

Local Humanizer is an independent, greenfield clean-room Tauri v2 desktop application with no affiliation with or endorsement from upstream authors. During initial architectural planning, an upstream open-source MIT project by **Rudra Sarker** was audited at commit `619feab622625d1fdb2b02f922b48c6fd4e81214`:

- **Excluded 100%:** External cloud LLM provider adapters, maintainer proxy tunnels, third-party machine-translation endpoints, hosted detection services, Next.js server routes, browser extension code, disk governance logs, external image beacons, scraped paper corpora, and training datasets.
- **Safe MIT Algorithmic Concepts Referenced:** Region protection placeholders (citations, quotes, URLs, numbers), offline semantic fidelity and regression validation checks, sentence splitting, and readability metrics.

```text
MIT License

Copyright (c) 2025-2026 Rudra Sarker

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## 3. Bundled Offline Inference Runtime (`ggml-org/llama.cpp`)

**Component:** `llama.cpp` (`b12600` prebuilt `llama-server` and shared libraries)  
**Source:** https://github.com/ggml-org/llama.cpp  
**License:** MIT License  
**Copyright:** Copyright (c) 2023-2026 The ggml authors  

The upstream `LICENSE` file is bundled alongside the application notices at `src-tauri/resources/notices/LICENSE-llama.cpp.txt`.

```text
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## 4. LLVM OpenMP Runtime (`libomp.dll`, Windows CPU Runtime)

**Component:** LLVM OpenMP (`libomp.dll` included in `llama.cpp` `b12600` Windows CPU builds)  
**License:** Apache License v2.0 with LLVM Exceptions  
**Notice File:** Bundled at `src-tauri/resources/notices/LICENSE-LLVM-OpenMP.txt`

---

## 5. Desktop Shell Framework (Tauri v2)

- **`tauri` (`2.12.x`)**, **`tauri-build` (`2.7.x`)**, **`tauri-plugin-single-instance` (`2.5.x`)**, **`tauri-plugin-dialog` (`2.8.x`)**, **`@tauri-apps/api` (`2.12.x`)**, **`@tauri-apps/plugin-dialog` (`2.8.x`)**, **`@tauri-apps/cli` (`2.12.x`)**
- **License:** `Apache-2.0 OR MIT`
- **Copyright:** Copyright Tauri Programme within The Commons Conservancy

---

## 6. Frontend Libraries & Build Tooling

| Package | License | Copyright / Holder |
|---|---|---|
| `react`, `react-dom` (`19.x`) | MIT | Meta Platforms, Inc. and affiliates |
| `vite` (`8.x`), `@vitejs/plugin-react` | MIT | Evan You & Vite Contributors |
| `tailwindcss` (`4.x`), `@tailwindcss/vite` | MIT | Tailwind Labs, Inc. (note: build-time `lightningcss` dependency is MPL-2.0) |
| `typescript` (`6.x`) | Apache-2.0 | Microsoft Corporation |
| `lucide-react` (`1.x`) | ISC | Lucide Contributors |

---

## 7. Rust Backend Crates

| Crate | License | Notes |
|---|---|---|
| `serde`, `serde_json` | `MIT OR Apache-2.0` | Serialization / deserialization |
| `tokio` | `MIT` | Async runtime & process management |
| `reqwest` (`0.12.x`) | `MIT OR Apache-2.0` | Local loopback HTTP & model download client |
| `ring` (via `rustls`) | `ISC` / BoringSSL-derived permissive | Pure-Rust/ASM TLS cryptography |
| `sha2`, `hex`, `rand` | `MIT OR Apache-2.0` | Integrity hashing & session token generation |

---

## 8. Quantized GGUF Language Models (Pending M2 Selection)

*Section reserved for Milestone M2 model benchmarking and selection:*

- Candidate permissive models (e.g., Qwen3 1.7B / Qwen3 4B / Qwen2.5 1.5B / SmolLM2 1.7B, Apache-2.0) and any evaluated gated-terms models (such as Gemma 3, subject to Google Gemma Terms of Use and Prohibited Use Policy flow-down if distributed) will have their exact model IDs, licenses, copyright notices, and SHA256 digests recorded here upon M2 completion.
- Models carrying non-commercial license restrictions (such as Qwen2.5 3B Research-Only) are excluded from default production distribution.
