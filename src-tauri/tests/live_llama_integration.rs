use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use local_humanizer_lib::commands::{
    build_loopback_http_client, Intensity, RewriteMode, RewriteOptions, VoiceOption,
};
use local_humanizer_lib::engine::protect::{contains_with_boundary, extract_invariants};
use local_humanizer_lib::engine::segmenter::{count_paragraphs, count_words};
use local_humanizer_lib::engine::validator::{
    detect_polarity_mismatch, lexical_distance, source_output_4gram_overlap,
    token_sequence_similarity,
};
use local_humanizer_lib::engine::{humanize_document_impl, EngineCoordinator, LlamaServerClient};
use local_humanizer_lib::models::{
    find_model, load_manifest_from_path, resolve_bundled_models_dir, verify_file_sha256_sync,
    verify_runtime_layout,
};
use local_humanizer_lib::runtime::{
    resolve_runtime_dir, select_backend_mode, start_and_wait_llama_server, LlamaServerHandle,
    ServerEndpoint,
};

fn default_options() -> RewriteOptions {
    RewriteOptions {
        preserve_length: true,
        preserve_formatting: true,
        preserve_citations: true,
        preserve_quotes: true,
        language: "en".to_string(),
        voice: VoiceOption::Auto,
    }
}

fn spawn_default_live_server() -> (
    tokio::runtime::Runtime,
    LlamaServerHandle,
    local_humanizer_lib::models::ManifestModelEntry,
) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let models_dir = resolve_bundled_models_dir(None).expect("models dir");
    let manifest = load_manifest_from_path(&models_dir.join("models.json")).expect("manifest");
    let entry = find_model(&manifest, "qwen3-1.7b-q4_k_m")
        .expect("default model")
        .clone();
    let model_path = models_dir.join(&entry.filename);
    verify_file_sha256_sync(&model_path, entry.size_bytes, &entry.sha256)
        .expect("default GGUF SHA256");

    let runtime_dir = resolve_runtime_dir(None).expect("runtime dir");
    verify_runtime_layout(&runtime_dir).expect("runtime layout");

    let backend_mode = select_backend_mode(None).expect("backend mode");
    let shutdown = AtomicBool::new(false);

    let handle = rt
        .block_on(start_and_wait_llama_server(
            &runtime_dir,
            &model_path,
            &entry,
            backend_mode,
            &shutdown,
        ))
        .expect("llama-server startup");

    (rt, handle, entry)
}

fn assert_reaped_and_port_closed(pid: u32, port: u16) {
    #[cfg(unix)]
    {
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(rc, -1, "owned PID {pid} should not exist after reap");
    }
    let addr = format!("127.0.0.1:{port}");
    assert!(
        std::net::TcpStream::connect(&addr).is_err(),
        "loopback port {port} should be closed after reap"
    );
}

#[test]
#[ignore]
fn live_health_and_bearer_auth_401() {
    let (rt, mut handle, _) = spawn_default_live_server();
    let pid = handle.pid();
    let endpoint = handle.endpoint.clone();
    let client = build_loopback_http_client().expect("http client");

    rt.block_on(async {
        // Valid /health
        let ok_resp = client
            .get(format!("{}/health", endpoint.base_url()))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", endpoint.bearer_token),
            )
            .send()
            .await
            .expect("health request");
        assert_eq!(ok_resp.status(), reqwest::StatusCode::OK);
        let body = ok_resp.text().await.unwrap();
        assert!(body.contains("\"status\":\"ok\""), "unexpected health body: {body}");

        // Unauthenticated GET /v1/models -> 401/403
        let unauth_models = client
            .get(format!("{}/v1/models", endpoint.base_url()))
            .send()
            .await
            .expect("unauth models");
        assert!(
            unauth_models.status() == reqwest::StatusCode::UNAUTHORIZED
                || unauth_models.status() == reqwest::StatusCode::FORBIDDEN,
            "expected 401/403, got {}",
            unauth_models.status()
        );

        // Wrong bearer POST /v1/chat/completions -> 401/403
        let wrong_chat = client
            .post(format!("{}/v1/chat/completions", endpoint.base_url()))
            .header(reqwest::header::AUTHORIZATION, "Bearer wrong-token")
            .json(&serde_json::json!({
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 8
            }))
            .send()
            .await
            .expect("wrong token chat");
        assert!(
            wrong_chat.status() == reqwest::StatusCode::UNAUTHORIZED
                || wrong_chat.status() == reqwest::StatusCode::FORBIDDEN,
            "expected 401/403, got {}",
            wrong_chat.status()
        );

        // CORS check: evil Origin must not be echoed or wildcard-allowed
        let cors_resp = client
            .request(
                reqwest::Method::OPTIONS,
                format!("{}/v1/chat/completions", endpoint.base_url()),
            )
            .header("Origin", "https://evil.example")
            .header("Access-Control-Request-Method", "POST")
            .send()
            .await
            .expect("cors preflight");
        if let Some(acao) = cors_resp.headers().get("access-control-allow-origin") {
            let val = acao.to_str().unwrap_or("");
            assert_ne!(val, "*");
            assert_ne!(val, "https://evil.example");
        }
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, endpoint.port);
}

#[test]
#[ignore]
fn live_spawn_cancel_and_reap_zero_orphans() {
    let (rt, mut handle, entry) = spawn_default_live_server();
    let pid = handle.pid();
    let endpoint = handle.endpoint.clone();
    let port = endpoint.port;

    let doc = (0..32)
        .map(|i| {
            format!(
                "Section {i}: On-device privacy-first writing tools evaluate local latency, memory bandwidth, and factual preservation across long technical manuscripts without external transmission."
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    assert!(count_words(&doc) >= 500);

    rt.block_on(async {
        let client = LlamaServerClient::new(endpoint).expect("client");
        let coord = Arc::new(EngineCoordinator::new());
        let cancel_coord = Arc::clone(&coord);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            cancel_coord.cancel_request("live-cancel-req");
        });

        let resp = humanize_document_impl(
            &client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-cancel-req".to_string(),
                text: doc,
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: default_options(),
            },
            entry.disable_thinking,
            |_| {},
        )
        .await
        .expect("cancelled response should be Ok");

        assert!(resp.cancelled, "expected cancelled=true");
        assert!(!resp.output_text.contains("__LH_PROT_"));
        assert!(!resp.output_text.contains("[[CITE_"));
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, port);
}

#[test]
#[ignore]
fn live_3000_word_paraphrase_e2e() {
    let (rt, mut handle, entry) = spawn_default_live_server();
    let pid = handle.pid();
    let endpoint = handle.endpoint.clone();
    let port = endpoint.port;

    let code_block = "```rust\nfn verify_local_hash(bytes: &[u8]) -> usize {\n\n    bytes.len()\n}\n```";
    let mut paras = Vec::new();
    for i in 0..32 {
        let p = match i % 4 {
            0 => format!(
                "Paragraph {i}: According to (Vance et al., 2024) and [1, 2], regional microgrids recorded $1,420 per household in verified annual savings on 2026-11-21. Detailed protocols remain archived at https://example.org/local-humanizer/spec under doi:10.1145/3580305.3599123 for independent engineering review and reproducibility across municipal utilities."
            ),
            1 => format!(
                "Paragraph {i}: Engineers observed that 94.5% of scheduled telemetry packets arrived within 18 ms across 12 substation nodes during peak winter demand. Operators emphasized \"Zero telemetry leaves the device.\" when auditing local inference pipelines for compliance with municipal data-governance charters."
            ),
            2 => format!(
                "Paragraph {i}: Independent reviewers evaluated throughput, memory pressure, and syntactic variety across multi-section technical reports exceeding three thousand words. By keeping numerical figures, dates, citations, and domain terminology intact, the rewriting stage preserved factual accuracy without sacrificing natural cadence."
            ),
            _ => format!(
                "Paragraph {i}: Furthermore, structured handover checkpoints prevented silent paragraph drops and ensured consistent tone across extended sections. Field supervisors confirmed that local workstations handled iterative revisions smoothly while maintaining strict offline isolation throughout every operational phase."
            ),
        };
        paras.push(p);
    }
    paras.insert(16, code_block.to_string());
    let mut full_paras = paras.clone();
    for i in 32..92 {
        full_paras.push(format!(
            "Extended section {i}: Municipal grid analysts documented steady voltage stability across {} feeder lines during winter testing in 2026. Maintaining accurate technical terminology, verified measurements, and consistent paragraph structure allowed engineers to compare historical baselines reliably without relying on remote cloud endpoints.",
            10 + (i % 20)
        ));
    }
    let full_doc = full_paras.join("\n\n");
    let full_src_words = count_words(&full_doc);
    let src_paragraphs = count_paragraphs(&full_doc);
    assert!(full_src_words >= 3000, "expected >=3000 words, got {full_src_words}");

    rt.block_on(async {
        let client = LlamaServerClient::new(endpoint).expect("client");
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-3000-e2e".to_string(),
                text: full_doc.clone(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: default_options(),
            },
            entry.disable_thinking,
            |_| {},
        )
        .await
        .expect("3000-word humanize");

        assert!(!resp.cancelled);
        assert!(
            !resp.failed,
            "3000-word humanize failed: {:?}, warnings={:?}",
            resp.error_message, resp.warnings
        );
        assert!(resp.chunks_total >= 90, "expected single-paragraph chunks >= 90, got {}", resp.chunks_total);
        assert_eq!(resp.chunks_completed, resp.chunks_total);

        let out_words = count_words(&resp.output_text);
        let out_paragraphs = count_paragraphs(&resp.output_text);
        let ratio = out_words as f64 / full_src_words as f64;
        assert!(
            (0.70..=1.45).contains(&ratio),
            "word ratio {ratio:.3} out of [0.70, 1.45] (src={full_src_words}, out={out_words})"
        );
        assert_eq!(
            out_paragraphs, src_paragraphs,
            "paragraph count mismatch: src={src_paragraphs}, out={out_paragraphs}"
        );
        assert!(resp.output_text.contains(code_block), "fenced code block altered");
        assert!(!resp.output_text.to_ascii_lowercase().contains("<think>"));
        assert!(!resp.output_text.to_ascii_lowercase().contains("</think>"));

        let doc_lex = lexical_distance(&full_doc, &resp.output_text);
        assert!(doc_lex >= 0.25, "3000-word output too close to source: lex_dist={doc_lex:.3}");

        let rss_kib = sample_process_rss_kib(pid);

        let required = [
            ("https://example.org/local-humanizer/spec", true),
            ("10.1145/3580305.3599123", true),
            ("(Vance et al., 2024)", true),
            ("[1, 2]", true),
            ("\"Zero telemetry leaves the device.\"", true),
            ("$1,420", false),
            ("2026-11-21", false),
            ("94.5%", false),
        ];
        let mut hits = 0usize;
        for (needle, exact) in required {
            if contains_with_boundary(needle, &resp.output_text, exact) {
                hits += 1;
            }
        }
        let retention = hits as f64 / required.len() as f64;
        assert!(
            retention >= 0.875,
            "invariant retention too low: {hits}/{} ({retention:.2}), warnings={:?}",
            required.len(),
            resp.warnings
        );
        println!(
            "live_3000_word_paraphrase_e2e OK: src_words={full_src_words}, out_words={out_words}, paras={src_paragraphs}->{out_paragraphs}, chunks={}/{}, retried={}, doc_lex={doc_lex:.3}, rss_mib={:.1}, duration_ms={}, warnings={:?}",
            resp.chunks_completed,
            resp.chunks_total,
            resp.chunks_retried,
            rss_kib as f64 / 1024.0,
            resp.duration_ms,
            resp.warnings
        );
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, port);
}

fn sample_process_rss_kib(pid: u32) -> u64 {
    #[cfg(unix)]
    {
        if let Ok(out) = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Ok(kib) = s.trim().parse::<u64>() {
                return kib;
            }
        }
    }
    0
}

fn simple_alnum_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[test]
#[ignore]
fn live_quality_6_modes_3_intensities_citations_and_failure() {
    let (rt, mut handle, entry) = spawn_default_live_server();
    let pid = handle.pid();
    let endpoint = handle.endpoint.clone();
    let port = endpoint.port;

    let ai_passage = "Artificial intelligence has emerged as one of the most transformative technological developments in modern human history. Across industries such as healthcare diagnostics and autonomous transportation, machine learning systems are reshaping how organizations analyze complex datasets and automate operational decisions. However, rapid adoption has also sparked serious debates around algorithmic bias, workforce displacement, and the need for consistent regulatory oversight across jurisdictions.";

    let cited_neg_passage = "Between January 2022 and March 2025, clinical machine learning models reduced diagnostic turnaround times by 34.8% across 18 regional hospitals (Patel & Chen, 2024, pp. 112–118). A 2025 benchmark report emphasized that \"automated triage systems must remain auditable by attending physicians\" before emergency deployment [4]. Although annual infrastructure savings reached $2.4 million, researchers warned that dataset drift dropped model accuracy from 96.2% to 88.5% without quarterly recalibration.";

    rt.block_on(async {
        let client = LlamaServerClient::new(endpoint.clone()).expect("client");
        let coord = EngineCoordinator::new();

        // 1) All 6 modes at Balanced on the AI passage
        let modes = [
            RewriteMode::Standard,
            RewriteMode::Natural,
            RewriteMode::Academic,
            RewriteMode::Professional,
            RewriteMode::Simple,
            RewriteMode::Deep,
        ];
        let mut mode_outputs = Vec::new();
        for mode in modes {
            let resp = humanize_document_impl(
                &client,
                &coord,
                local_humanizer_lib::commands::HumanizeRequest {
                    request_id: format!("live-mode-{mode:?}"),
                    text: ai_passage.to_string(),
                    mode,
                    intensity: Intensity::Balanced,
                    options: default_options(),
                },
                entry.disable_thinking,
                |_| {},
            )
            .await
            .expect("mode request");

            assert!(!resp.failed, "mode {mode:?} failed: {:?}", resp.error_message);
            assert!(!resp.cancelled);
            assert_ne!(resp.output_text.trim(), ai_passage.trim());

            let lex = lexical_distance(ai_passage, &resp.output_text);
            let src_t = simple_alnum_tokens(ai_passage);
            let out_t = simple_alnum_tokens(&resp.output_text);
            let ov4 = source_output_4gram_overlap(&src_t, &out_t);
            let seq = token_sequence_similarity(&src_t, &out_t);
            println!(
                "MODE={mode:?}/Balanced | lex={lex:.3} 4gram={ov4:.3} seq={seq:.3} retried={} dur={}ms\nOUT: {}\n",
                resp.chunks_retried, resp.duration_ms, resp.output_text
            );
            assert!(lex >= 0.24, "mode {mode:?} lex too low: {lex:.3}");
            mode_outputs.push(resp.output_text);
        }
        // Ensure modes are not producing identical strings
        let unique_modes: std::collections::HashSet<&str> =
            mode_outputs.iter().map(String::as_str).collect();
        assert_eq!(unique_modes.len(), 6, "all 6 modes must produce distinct outputs");

        // 2) All 3 intensities on Standard mode
        for intensity in [Intensity::Light, Intensity::Balanced, Intensity::Strong] {
            let resp = humanize_document_impl(
                &client,
                &coord,
                local_humanizer_lib::commands::HumanizeRequest {
                    request_id: format!("live-int-{intensity:?}"),
                    text: ai_passage.to_string(),
                    mode: RewriteMode::Standard,
                    intensity,
                    options: default_options(),
                },
                entry.disable_thinking,
                |_| {},
            )
            .await
            .expect("intensity request");
            assert!(!resp.failed, "intensity {intensity:?} failed: {:?}", resp.error_message);
            let lex = lexical_distance(ai_passage, &resp.output_text);
            let src_t = simple_alnum_tokens(ai_passage);
            let out_t = simple_alnum_tokens(&resp.output_text);
            let ov4 = source_output_4gram_overlap(&src_t, &out_t);
            let seq = token_sequence_similarity(&src_t, &out_t);
            println!(
                "INTENSITY=Standard/{intensity:?} | lex={lex:.3} 4gram={ov4:.3} seq={seq:.3} retried={} dur={}ms\nOUT: {}\n",
                resp.chunks_retried, resp.duration_ms, resp.output_text
            );
        }

        // 3) Passage with citations, dates, numbers, exact quote, and "without quarterly recalibration" negation
        let cite_resp = humanize_document_impl(
            &client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-cite-neg".to_string(),
                text: cited_neg_passage.to_string(),
                mode: RewriteMode::Academic,
                intensity: Intensity::Balanced,
                options: default_options(),
            },
            entry.disable_thinking,
            |_| {},
        )
        .await
        .expect("citation/negation request");
        assert!(!cite_resp.failed, "citation/negation failed: {:?}", cite_resp.error_message);
        let lex = lexical_distance(cited_neg_passage, &cite_resp.output_text);
        let pol_err = detect_polarity_mismatch(cited_neg_passage, &cite_resp.output_text, &default_options());
        println!(
            "CITE+NEG (Academic/Balanced) | lex={lex:.3} retried={} pol_err={pol_err:?} warnings={:?}\nOUT: {}\n",
            cite_resp.chunks_retried, cite_resp.warnings, cite_resp.output_text
        );
        assert!(pol_err.is_none(), "polarity mismatch on cited passage: {pol_err:?}");
        let inv = extract_invariants(cited_neg_passage, &default_options());
        for (cat, val, exact) in inv.enforced_items(&default_options()) {
            assert!(
                contains_with_boundary(val, &cite_resp.output_text, exact),
                "missing {cat}:{val} in output: {}",
                cite_resp.output_text
            );
        }

        // 4) Explicit failure test on stopped/unreachable server (no silent passthrough)
        let dead_endpoint = ServerEndpoint {
            host: "127.0.0.1".to_string(),
            port: 1,
            bearer_token: "dead".to_string(),
            disable_thinking: true,
            backend_mode: endpoint.backend_mode,
            model_id: endpoint.model_id.clone(),
        };
        let dead_client = LlamaServerClient::new(dead_endpoint).expect("dead client");
        let dead_res = humanize_document_impl(
            &dead_client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-dead-server".to_string(),
                text: "Original text that must never be returned as rewritten on failure.".to_string(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: default_options(),
            },
            true,
            |_| {},
        )
        .await;
        assert!(dead_res.is_err(), "stopped/unreachable server must return explicit Err");
        let rss_kib = sample_process_rss_kib(pid);
        println!("live_quality_6_modes_3_intensities_citations_and_failure OK | peak_rss_mib={:.1}", rss_kib as f64 / 1024.0);
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, port);
}

#[test]
#[ignore]
fn live_adversarial_traps_and_3000_word_academic_submission() {
    let (rt, mut handle, entry) = spawn_default_live_server();
    let pid = handle.pid();
    let endpoint = handle.endpoint.clone();
    let port = endpoint.port;

    let trap_citations_quotes_doi = "On January 14, 2025, researchers at Northbridge Labs reported that distributed edge nodes reduced inference latency by 18.4% while trimming operating costs from $4.8 million to -$250,000 net variance (Smith & Patel, 2024, pp. 42–45). According to the audit [12], \"exact quoted statements must remain unaltered during automated processing\" as documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4.";

    let trap_negations = "Modern logistics networks cannot scale reliably without redundant fail-safes across regional hubs. Unless operators verify backup telemetry daily, automated routing will never recover from sudden fiber cuts, and only verified manual overrides can prevent cascading outages across the energy industry.";

    let trap_without_only = "Deploying clinical decision models without continuous post-market auditing violates hospital safety protocols, because these systems only maintain 94.2% sensitivity when trained on balanced cohorts from 2023–2025.";

    let chunk_5_req_without = "Achieving high analytical reliability under strictly offline constraints requires rethinking both model architecture and systems engineering. Large foundation models exceeding 70 billion parameters deliver strong zero-shot reasoning, yet their memory footprint demands dedicated multi-GPU servers costing $185,000 or more per facility. By contrast, compact quantized models running on standard workstation CPUs or unified-memory laptops reduce hardware capital expenditure to under $1,800 per clinical station. Even so, smaller models cannot compensate for weak orchestration: without deterministic input segmentation, strict factual verification, and explicit failure detection, lightweight pipelines risk silently distorting clinical summaries or dropping critical numerical thresholds.";

    let chunk_16_eval = "To evaluate whether compact local models can sustain high paraphrasing quality without cloud assistance, reviewers compared structural diversity, factual retention, and polarity accuracy across thirty technical manuscripts. Each manuscript contained domain terminology, inline citations, and multi-clause reasoning that challenged lightweight language models. Although single-pass prompts occasionally preserved source clause skeletons, iterative structural pivoting and sentence-level decomposition produced substantially more varied phrasing without altering empirical conclusions.";

    rt.block_on(async {
        let client = LlamaServerClient::new(endpoint.clone()).expect("client");
        let coord = EngineCoordinator::new();

        // 1) Isolated trap tests
        let trap_cases = [
            ("trap_citations_quotes_doi", trap_citations_quotes_doi, RewriteMode::Academic),
            ("trap_negations", trap_negations, RewriteMode::Standard),
            ("trap_without_only", trap_without_only, RewriteMode::Professional),
            ("chunk_5_req_without", chunk_5_req_without, RewriteMode::Standard),
            ("chunk_16_eval", chunk_16_eval, RewriteMode::Standard),
        ];

        for (label, text, mode) in trap_cases {
            let resp = humanize_document_impl(
                &client,
                &coord,
                local_humanizer_lib::commands::HumanizeRequest {
                    request_id: format!("live-{label}"),
                    text: text.to_string(),
                    mode,
                    intensity: Intensity::Balanced,
                    options: default_options(),
                },
                entry.disable_thinking,
                |_| {},
            )
            .await
            .expect(label);

            assert!(
                !resp.failed,
                "{label} failed: {:?}, warnings={:?}",
                resp.error_message, resp.warnings
            );
            let lex = lexical_distance(text, &resp.output_text);
            let src_t = simple_alnum_tokens(text);
            let out_t = simple_alnum_tokens(&resp.output_text);
            let ov4 = source_output_4gram_overlap(&src_t, &out_t);
            let seq = token_sequence_similarity(&src_t, &out_t);
            println!(
                "TRAP={label} ({mode:?}/Balanced) | lex={lex:.3} 4gram={ov4:.3} seq={seq:.3} retried={} dur={}ms warnings={:?}\nOUT: {}\n",
                resp.chunks_retried, resp.duration_ms, resp.warnings, resp.output_text
            );

            let inv = extract_invariants(text, &default_options());
            for (cat, val, exact) in inv.enforced_items(&default_options()) {
                assert!(
                    contains_with_boundary(val, &resp.output_text, exact),
                    "{label} missing {cat}:{val} in output: {}",
                    resp.output_text
                );
            }
            let pol_err = detect_polarity_mismatch(text, &resp.output_text, &default_options());
            assert!(
                pol_err.is_none(),
                "{label} polarity/restrictor error: {pol_err:?} | out={}",
                resp.output_text
            );
            assert!(!resp.output_text.contains("[[URL_"));
            assert!(!resp.output_text.contains("[[DOI_"));
            assert!(!resp.output_text.contains("[[EMAIL_"));
            assert!(!resp.output_text.contains("[[QUOTE_"));
        }

        // 1b) 5/5 consecutive reliability on trap_citations_quotes_doi in BOTH Standard/Balanced and Academic/Balanced
        for mode in [RewriteMode::Standard, RewriteMode::Academic] {
            for trial in 1..=5 {
                let resp = humanize_document_impl(
                    &client,
                    &coord,
                    local_humanizer_lib::commands::HumanizeRequest {
                        request_id: format!("live-5x2-{mode:?}-{trial}"),
                        text: trap_citations_quotes_doi.to_string(),
                        mode,
                        intensity: Intensity::Balanced,
                        options: default_options(),
                    },
                    entry.disable_thinking,
                    |_| {},
                )
                .await
                .expect("5x2 trial");

                assert!(
                    !resp.failed,
                    "5x2 {mode:?}/Balanced trial {trial} failed: {:?}, warnings={:?}",
                    resp.error_message, resp.warnings
                );
                assert!(
                    !resp.warnings.iter().any(|w| w.contains("omitted protected item")),
                    "5x2 {mode:?}/Balanced trial {trial} had missing invariant warning: {:?}",
                    resp.warnings
                );
                assert_eq!(
                    resp.output_text.matches("[12]").count(),
                    1,
                    "5x2 {mode:?}/Balanced trial {trial} [12] count != 1: {}",
                    resp.output_text
                );
                assert_eq!(
                    resp.output_text.matches("10.1038/s41586-024-07123-4").count(),
                    1,
                    "5x2 {mode:?}/Balanced trial {trial} DOI count != 1: {}",
                    resp.output_text
                );
                assert_eq!(
                    resp.output_text.matches("https://example.org/audits/2025").count(),
                    1,
                    "5x2 {mode:?}/Balanced trial {trial} URL count != 1: {}",
                    resp.output_text
                );
                assert!(
                    contains_with_boundary(
                        "\"exact quoted statements must remain unaltered during automated processing\"",
                        &resp.output_text,
                        true
                    ),
                    "5x2 {mode:?}/Balanced trial {trial} missing verbatim quote: {}",
                    resp.output_text
                );
                assert_eq!(
                    resp.output_text.matches("exact quoted statements must remain unaltered during automated processing").count(),
                    1,
                    "5x2 {mode:?}/Balanced trial {trial} quote count != 1: {}",
                    resp.output_text
                );
                assert!(
                    !local_humanizer_lib::engine::validator::has_identifier_subject_verb(&resp.output_text),
                    "5x2 {mode:?}/Balanced trial {trial} used DOI/URL as verb subject: {}",
                    resp.output_text
                );
                let lex = lexical_distance(trap_citations_quotes_doi, &resp.output_text);
                println!(
                    "5x2 {mode:?}/Balanced trial {trial}/5 PASS | lex={lex:.3} retried={} dur={}ms\nOUT: {}\n",
                    resp.chunks_retried, resp.duration_ms, resp.output_text
                );
            }
        }

        // 2) Full >=3,050-word multi-section academic submission with Markdown headings, code block, traps, and citations
        let headings = [
            "# Autonomous Local Paraphrasing Pipelines for Regulated Environments",
            "## 1. Introduction and Operational Context",
            "## 2. Architectural Constraints and Memory Budgets",
            "### 2.1 Quantized Inference on Unified-Memory Hosts",
            "## 3. Empirical Evaluation and Citation Integrity",
            "### 3.1 Negation and Polarity Safeguards",
            "## 4. Systems Engineering and Reproducibility",
            "## 5. Discussion and Operational Limitations",
            "## 6. Conclusion",
        ];
        let code_block = "```rust\nfn verify_chunk_integrity(words: usize, intact: bool) -> bool {\n\n    words > 0 && intact\n}\n```";

        let mut sections: Vec<String> = Vec::new();
        sections.push(headings[0].to_string());
        sections.push(headings[1].to_string());
        sections.push(chunk_5_req_without.to_string());
        sections.push(trap_citations_quotes_doi.to_string());
        sections.push(headings[2].to_string());
        sections.push(headings[3].to_string());
        sections.push(trap_negations.to_string());
        sections.push(trap_without_only.to_string());
        sections.push(headings[4].to_string());
        sections.push(chunk_16_eval.to_string());
        sections.push(headings[5].to_string());
        sections.push(code_block.to_string());

        for i in 0..60 {
            if i == 18 {
                sections.push(headings[6].to_string());
            } else if i == 38 {
                sections.push(headings[7].to_string());
            } else if i == 56 {
                sections.push(headings[8].to_string());
            }
            let para = match i % 4 {
                0 => format!(
                    "Section study {i} examined longitudinal telemetry collected between February 2023 and November 2025 across {} municipal substations (Patel & Chen, 2024, pp. 14–19). Analysts confirmed that automated batching lowered peak memory consumption by 22.5% while maintaining $1,650 in monthly operational savings per cluster. Without continuous checksum verification, distributed storage replicas cannot detect silent bit rot prior to scheduled archival.",
                    12 + (i % 9)
                ),
                1 => format!(
                    "Benchmark cohort {i} evaluated structured academic prose containing nested clauses, technical acronyms, and quantitative bounds from 2022–2025 [7, 8]. Reviewers noted that \"deterministic segmentation prevents structural drift during multi-thousand-word processing\" whenever local runtimes operate offline. Unless operators enforce strict delimiter boundaries, uncalibrated generators risk merging headings into adjacent paragraphs.",
                ),
                2 => format!(
                    "Operational review {i} tracked CPU and unified-memory utilization across workstation deployments under 8 GB RAM limits. Even when document length exceeded three thousand words, single-paragraph chunking kept token latency predictable without sacrificing grammatical fluency. Moreover, preserving exact dates such as 2026-11-21 and exact percentages such as 97.3% ensured downstream auditability.",
                ),
                _ => format!(
                    "Governance audit {i} compared manual editorial revisions against local paraphrasing pipelines across {} institutional reports. Although compact models require explicit clause-level guidance to avoid echoing source syntax, three-tier retry orchestration resolved stubborn passages without returning unedited text. Consequently, engineering teams achieved reliable offline throughput without external API dependencies.",
                    15 + (i % 10)
                ),
            };
            sections.push(para);
        }

        let full_doc = sections.join("\n\n");
        let src_words = count_words(&full_doc);
        let src_paras = count_paragraphs(&full_doc);
        assert!(src_words >= 3200, "expected >=3200 words, got {src_words}");

        let doc_resp = humanize_document_impl(
            &client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-3055-academic".to_string(),
                text: full_doc.clone(),
                mode: RewriteMode::Standard,
                intensity: Intensity::Balanced,
                options: default_options(),
            },
            entry.disable_thinking,
            |_| {},
        )
        .await
        .expect("3055-word academic submission");

        assert!(
            !doc_resp.failed,
            "3055-word academic submission failed: {:?}, warnings={:?}",
            doc_resp.error_message, doc_resp.warnings
        );
        assert!(!doc_resp.cancelled);
        assert_eq!(doc_resp.chunks_completed, doc_resp.chunks_total);

        let out_words = count_words(&doc_resp.output_text);
        let out_paras = count_paragraphs(&doc_resp.output_text);
        assert_eq!(
            out_paras, src_paras,
            "paragraph count mismatch: src={src_paras}, out={out_paras}"
        );

        for h in headings {
            assert!(
                doc_resp.output_text.contains(h),
                "heading missing or altered: {h}"
            );
        }
        assert!(doc_resp.output_text.contains(code_block), "code block altered");

        for (needle, exact) in [
            ("https://example.org/audits/2025", true),
            ("10.1038/s41586-024-07123-4", true),
            ("\"exact quoted statements must remain unaltered during automated processing\"", true),
            ("(Smith & Patel, 2024, pp. 42–45)", true),
            ("[12]", true),
            ("$4.8", false),
            ("-$250,000", false),
            ("18.4%", false),
            ("January 14, 2025", false),
            ("$185,000", false),
            ("$1,800", false),
            ("94.2%", false),
        ] {
            assert!(
                contains_with_boundary(needle, &doc_resp.output_text, exact),
                "3055-word doc missing invariant {needle}"
            );
        }

        let doc_lex = lexical_distance(&full_doc, &doc_resp.output_text);
        assert!(doc_lex >= 0.25, "doc_lex too low: {doc_lex:.3}");
        let rss_kib = sample_process_rss_kib(pid);
        println!(
            "3055_ACADEMIC_PASS: src_words={src_words}, out_words={out_words}, paras={src_paras}->{out_paras}, chunks={}/{}, retried={}, doc_lex={doc_lex:.3}, rss_mib={:.1}, duration_ms={}, warnings={:?}",
            doc_resp.chunks_completed,
            doc_resp.chunks_total,
            doc_resp.chunks_retried,
            rss_kib as f64 / 1024.0,
            doc_resp.duration_ms,
            doc_resp.warnings
        );
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, port);
}
