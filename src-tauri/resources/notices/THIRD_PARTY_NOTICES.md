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
| `vite` (`8.x`), `@vitejs/plugin-react` (`6.x`) | MIT | Evan You & Vite Contributors |
| `tailwindcss` (`4.x`), `@tailwindcss/vite` | MIT | Tailwind Labs, Inc. |
| `typescript` (`6.x`) | Apache-2.0 | Microsoft Corporation |
| `lucide-react` (`1.x`) | ISC | Lucide Contributors |

Additional npm transitive/build-time licenses (`package-lock.json`):
- **`MPL-2.0` (Build-Time):** `lightningcss` and platform binary packages (via `@tailwindcss/vite`; unmodified upstream source available via `https://github.com/parcel-bundler/lightningcss`).
- **`0BSD`:** `tslib`
- **`BlueOak-1.0.0` (Build-Time):** `graceful-fs`
- **`BSD-3-Clause` (Build-Time):** `source-map-js`

---

## 7. Rust Backend Crates

### 7.1 Direct Crates (`src-tauri/Cargo.toml`)

| Crate | License | Notes |
|---|---|---|
| `serde`, `serde_json` | `MIT OR Apache-2.0` | Serialization / deserialization |
| `tokio` | `MIT` | Async runtime & process management |
| `tokio-util` (`0.7.x`) | `MIT` | Cooperative `CancellationToken` lifecycle control |
| `reqwest` (`0.12.x`) | `MIT OR Apache-2.0` | Local loopback HTTP & HTTPS model download client (`rustls-tls`) |
| `ring` (via `rustls`) | `ISC` / BoringSSL-derived permissive | Pure-Rust/ASM TLS cryptography |
| `sha2`, `hex`, `rand` | `MIT OR Apache-2.0` | Integrity hashing & session token generation |
| `regex` (`1.x`) | `MIT OR Apache-2.0` | Invariant extraction & text normalization |
| `libc` (`0.2.x`) | `MIT OR Apache-2.0` | Unix `setsid`, process-group reaping, and `statvfs` |
| `windows-sys` (`0.61.x`) | `MIT OR Apache-2.0` | Windows Job Object reaping (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) & memory telemetry |

### 7.2 Transitive Cargo Licenses (`src-tauri/Cargo.lock`)

In addition to `MIT`, `Apache-2.0`, and `MIT OR Apache-2.0` crates, the compiled Rust dependency tree includes unmodified upstream crates under:
- **`MPL-2.0` (Weak Copyleft):** `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors` (unmodified upstream crates distributed via `https://crates.io`; source code for these MPL-2.0 components remains publicly available under the Mozilla Public License 2.0).
- **`Unicode-3.0`:** `icu_collections`, `icu_locale_core`, `icu_normalizer`, `icu_normalizer_data`, `icu_properties`, `icu_properties_data`, `icu_provider`, `litemap`, `potential_utf`, `tinystr`, `writeable`, `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive`, `zerotrie`, `zerovec`, `zerovec-derive`.
- **`ISC`:** `rustls-webpki`, `untrusted`.
- **`BSD-3-Clause`:** `alloc-no-stdlib`, `alloc-stdlib`, `subtle`.
- **`CDLA-Permissive-2.0`:** `webpki-roots` (root CA bundle used for HTTPS model downloads).
- **`Zlib`:** `foldhash`, `zlib-rs`.

---

## 8. Quantized GGUF Language Models

Local Humanizer distributes and supports quantized GGUF language models verified during Milestone M2 benchmarking (`benchmarks/REPORT.md` and `src-tauri/resources/models/models.json`). The full Apache License, Version 2.0 text is bundled alongside the application notices at `src-tauri/resources/notices/LICENSE-Apache-2.0.txt` (satisfying Apache-2.0 Section 4(a); upstream `Qwen/Qwen3-1.7B`, `Qwen/Qwen3-4B-Instruct-2507`, and `Qwen/Qwen2.5-1.5B-Instruct` repositories do not contain a separate `NOTICE` file).

### 8.1 Default Bundled Model
- **Model ID:** `qwen3-1.7b-q4_k_m` (`Qwen3-1.7B-Q4_K_M.gguf`)
- **Upstream Base Model:** `Qwen/Qwen3-1.7B` (Copyright Alibaba Cloud / Qwen Team)
- **Quantized GGUF Distribution:** `https://huggingface.co/unsloth/Qwen3-1.7B-GGUF`
- **License:** Apache License, Version 2.0 (`Apache-2.0`, `https://huggingface.co/Qwen/Qwen3-1.7B/blob/main/LICENSE`)
- **Exact Byte Size:** `1,107,409,472` bytes (~1.03 GiB)
- **SHA256 Digest:** `b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897`

### 8.2 Optional On-Demand Permissive Models (Not Bundled in Installer)
- **High-RAM Quality Tier (`qwen3-4b-instruct-2507-q4_k_m`):** `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (`https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF`, base model `Qwen/Qwen3-4B-Instruct-2507`), **License:** `Apache-2.0` (`https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/blob/main/LICENSE`), **Size:** `2,497,281,120` bytes (~2.33 GiB), **SHA256:** `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597`.
- **8 GB Fast Alternative (`qwen2.5-1.5b-instruct-q4_k_m`):** `qwen2.5-1.5b-instruct-q4_k_m.gguf` (`https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF`, base model `Qwen/Qwen2.5-1.5B-Instruct`), **License:** `Apache-2.0` (`https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/main/LICENSE`), **Size:** `1,117,320,736` bytes (~1.04 GiB), **SHA256:** `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`.

### 8.3 Evaluated Models Excluded from Production Distribution or Flagged Non-Commercial
- **`qwen2.5-3b-instruct-q4_k_m` (`Qwen/Qwen2.5-3B-Instruct-GGUF`, SHA256 `626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d`):** Licensed under the **Qwen RESEARCH LICENSE AGREEMENT** (`https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/blob/main/LICENSE`, Copyright (c) Alibaba Cloud). **Non-commercial / research-only.** Marked `non_commercial_warning: true`, `is_default: false`, and `is_bundled: false` in `models.json`; excluded from production default bundling.
- **`gemma-3-1b-it-q4_k_m` (`ggml-org/gemma-3-1b-it-GGUF`, SHA256 `8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135`):** Evaluated in M2 benchmarks only under the **Gemma Terms of Use** (`https://ai.google.dev/gemma/terms`). Not bundled or distributed in `models.json`.
- **`smollm2-1.7b-instruct-q4_k_m` (`bartowski/SmolLM2-1.7B-Instruct-GGUF`, SHA256 `77665ea4815999596525c636fbeb56ba8b080b46ae85efef4f0d986a139834d7`, `Apache-2.0`):** Evaluated in M2 benchmarks only; excluded from `models.json` due to verbatim-echo behavior (`mean_lexical_distance = 0.0039`).
