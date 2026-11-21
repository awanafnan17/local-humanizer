use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use local_humanizer_lib::commands::{
    build_loopback_http_client, Intensity, RewriteMode, RewriteOptions, VoiceOption,
};
use local_humanizer_lib::engine::protect::contains_with_boundary;
use local_humanizer_lib::engine::segmenter::count_words;
use local_humanizer_lib::engine::{humanize_document_impl, EngineCoordinator, LlamaServerClient};
use local_humanizer_lib::models::{
    find_model, load_manifest_from_path, resolve_bundled_models_dir, verify_file_sha256_sync,
    verify_runtime_layout,
};
use local_humanizer_lib::runtime::{
    resolve_runtime_dir, select_backend_mode, start_and_wait_llama_server, LlamaServerHandle,
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
    assert!(full_src_words >= 3000, "expected >=3000 words, got {full_src_words}");

    rt.block_on(async {
        let client = LlamaServerClient::new(endpoint).expect("client");
        let coord = EngineCoordinator::new();
        let resp = humanize_document_impl(
            &client,
            &coord,
            local_humanizer_lib::commands::HumanizeRequest {
                request_id: "live-3000-e2e".to_string(),
                text: full_doc,
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
        assert!(resp.chunks_total >= 8, "chunks_total={}", resp.chunks_total);
        assert_eq!(resp.chunks_completed, resp.chunks_total);

        let out_words = count_words(&resp.output_text);
        let ratio = out_words as f64 / full_src_words as f64;
        assert!(
            (0.70..=1.45).contains(&ratio),
            "word ratio {ratio:.3} out of [0.70, 1.45] (src={full_src_words}, out={out_words})"
        );
        assert!(resp.output_text.contains(code_block), "fenced code block altered");
        assert!(!resp.output_text.to_ascii_lowercase().contains("<think>"));
        assert!(!resp.output_text.to_ascii_lowercase().contains("</think>"));

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
            "live_3000_word_paraphrase_e2e OK: src_words={full_src_words}, out_words={out_words}, chunks={}/{}, retried={}, duration_ms={}, warnings={:?}",
            resp.chunks_completed, resp.chunks_total, resp.chunks_retried, resp.duration_ms, resp.warnings
        );
    });

    handle.reap_sync();
    assert_reaped_and_port_closed(pid, port);
}
