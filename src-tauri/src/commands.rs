use hex;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RUNTIME_VERSION: &str = "b12600";

/// Compute lowercase hex-encoded SHA-256 digest (used for GGUF & runtime integrity checks).
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Generate a 256-bit cryptographically random per-session loopback bearer token.
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Construct an isolated loopback-only HTTP client (`no_proxy`) for local llama-server IPC.
pub fn build_loopback_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| format!("failed to build loopback HTTP client: {e}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Initializing,
    Verifying,
    Downloading,
    StartingRuntime,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendMode {
    Metal,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStatus {
    pub state: RuntimeState,
    pub active_model_id: String,
    pub model_ready: bool,
    pub backend_mode: BackendMode,
    pub ram_bytes: u64,
    pub error_message: Option<String>,
    pub runtime_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: String,
    pub name: String,
    pub quant: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub license: String,
    pub license_url: String,
    pub is_default: bool,
    pub is_installed: bool,
    pub is_bundled: bool,
    pub min_recommended_ram_gb: u32,
    pub non_commercial_warning: bool,
    pub download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Downloading,
    Verifying,
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadProgressEvent {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub status: DownloadStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteMode {
    Standard,
    Natural,
    Academic,
    Professional,
    Simple,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    Light,
    Balanced,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceOption {
    Auto,
    FirstPerson,
    ThirdPerson,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewriteOptions {
    pub preserve_length: bool,
    pub preserve_formatting: bool,
    pub preserve_citations: bool,
    pub preserve_quotes: bool,
    pub language: String,
    pub voice: VoiceOption,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanizeRequest {
    pub request_id: String,
    pub text: String,
    pub mode: RewriteMode,
    pub intensity: Intensity,
    pub options: RewriteOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanizeResponse {
    pub request_id: String,
    pub output_text: String,
    pub chunks_total: usize,
    pub chunks_completed: usize,
    pub chunks_retried: usize,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanizeProgressState {
    Segmenting,
    Inferring,
    Validating,
    Retrying,
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanizeProgressEvent {
    pub request_id: String,
    pub chunk_index: usize,
    pub chunks_total: usize,
    pub state: HumanizeProgressState,
    pub partial_output: String,
    pub chunk_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryChunkRequest {
    pub request_id: String,
    pub chunk_index: usize,
    pub original_chunk: String,
    pub context_before: String,
    pub mode: RewriteMode,
    pub intensity: Intensity,
    pub options: RewriteOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryChunkResponse {
    pub request_id: String,
    pub chunk_index: usize,
    pub rewritten_chunk: String,
    pub warnings: Vec<String>,
}

pub fn default_stub_status() -> AppStatus {
    AppStatus {
        state: RuntimeState::Initializing,
        active_model_id: String::new(),
        model_ready: false,
        backend_mode: if cfg!(target_os = "macos") {
            BackendMode::Metal
        } else {
            BackendMode::Cpu
        },
        ram_bytes: 0,
        error_message: None,
        runtime_version: RUNTIME_VERSION.to_string(),
    }
}

#[tauri::command]
pub async fn get_app_status() -> Result<AppStatus, String> {
    Ok(default_stub_status())
}

#[tauri::command]
pub async fn list_models() -> Result<Vec<ModelDescriptor>, String> {
    Ok(Vec::new())
}

#[tauri::command]
pub async fn select_model(model_id: String, _use_gpu: Option<bool>) -> Result<AppStatus, String> {
    if model_id.trim().is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    Err("select_model is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn download_model(model_id: String) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    Err("download_model is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn cancel_download(model_id: String) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn humanize_document(req: HumanizeRequest) -> Result<HumanizeResponse, String> {
    if req.request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }
    Err("humanize_document is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn cancel_humanize(request_id: String) -> Result<(), String> {
    if request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn retry_chunk(req: RetryChunkRequest) -> Result<RetryChunkResponse, String> {
    if req.request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }
    Err("retry_chunk is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn import_text_file(path: String) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("path cannot be empty".to_string());
    }
    Err("import_text_file is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn save_output_file(path: String, _content: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path cannot be empty".to_string());
    }
    Err("save_output_file is not implemented in M1 scaffold; implemented in M3".to_string())
}

#[tauri::command]
pub async fn retry_runtime_init() -> Result<AppStatus, String> {
    Ok(default_stub_status())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_options() -> RewriteOptions {
        RewriteOptions {
            preserve_length: true,
            preserve_formatting: true,
            preserve_citations: true,
            preserve_quotes: true,
            language: "en".to_string(),
            voice: VoiceOption::Auto,
        }
    }

    #[test]
    fn app_status_roundtrips_with_snake_case_fields() {
        let status = AppStatus {
            state: RuntimeState::StartingRuntime,
            active_model_id: "qwen3-1.7b-q4_k_m".to_string(),
            model_ready: false,
            backend_mode: BackendMode::Metal,
            ram_bytes: 8_589_934_592,
            error_message: None,
            runtime_version: RUNTIME_VERSION.to_string(),
        };
        let json = serde_json::to_string(&status).expect("serialize AppStatus");
        assert!(json.contains("\"state\":\"starting_runtime\""));
        assert!(json.contains("\"backend_mode\":\"metal\""));
        assert!(json.contains("\"runtime_version\":\"b12600\""));
        let decoded: AppStatus = serde_json::from_str(&json).expect("deserialize AppStatus");
        assert_eq!(decoded, status);
    }

    #[test]
    fn model_descriptor_and_download_event_roundtrip() {
        let desc = ModelDescriptor {
            id: "qwen3-1.7b-q4_k_m".to_string(),
            name: "Qwen3 1.7B".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 1_200_000_000,
            sha256: "0".repeat(64),
            license: "Apache-2.0".to_string(),
            license_url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
            is_default: true,
            is_installed: false,
            is_bundled: false,
            min_recommended_ram_gb: 8,
            non_commercial_warning: false,
            download_url: "https://example.invalid/model.gguf".to_string(),
        };
        let desc_json = serde_json::to_string(&desc).expect("serialize ModelDescriptor");
        let desc_back: ModelDescriptor =
            serde_json::from_str(&desc_json).expect("deserialize ModelDescriptor");
        assert_eq!(desc_back, desc);

        let evt = ModelDownloadProgressEvent {
            model_id: desc.id.clone(),
            downloaded_bytes: 1024,
            total_bytes: 2048,
            status: DownloadStatus::Verifying,
            error: None,
        };
        let evt_json = serde_json::to_string(&evt).expect("serialize ModelDownloadProgressEvent");
        assert!(evt_json.contains("\"status\":\"verifying\""));
        let evt_back: ModelDownloadProgressEvent =
            serde_json::from_str(&evt_json).expect("deserialize ModelDownloadProgressEvent");
        assert_eq!(evt_back, evt);
    }

    #[test]
    fn humanize_and_retry_contracts_roundtrip() {
        let req = HumanizeRequest {
            request_id: "req-1".to_string(),
            text: "Hello world.".to_string(),
            mode: RewriteMode::Academic,
            intensity: Intensity::Balanced,
            options: RewriteOptions {
                voice: VoiceOption::ThirdPerson,
                ..sample_options()
            },
        };
        let req_json = serde_json::to_string(&req).expect("serialize HumanizeRequest");
        assert!(req_json.contains("\"mode\":\"academic\""));
        assert!(req_json.contains("\"intensity\":\"balanced\""));
        assert!(req_json.contains("\"voice\":\"third_person\""));
        let req_back: HumanizeRequest =
            serde_json::from_str(&req_json).expect("deserialize HumanizeRequest");
        assert_eq!(req_back, req);

        let resp = HumanizeResponse {
            request_id: "req-1".to_string(),
            output_text: "Greetings, world.".to_string(),
            chunks_total: 1,
            chunks_completed: 1,
            chunks_retried: 0,
            warnings: vec![],
            duration_ms: 42,
            cancelled: false,
        };
        let resp_json = serde_json::to_string(&resp).expect("serialize HumanizeResponse");
        let resp_back: HumanizeResponse =
            serde_json::from_str(&resp_json).expect("deserialize HumanizeResponse");
        assert_eq!(resp_back, resp);

        let progress = HumanizeProgressEvent {
            request_id: "req-1".to_string(),
            chunk_index: 0,
            chunks_total: 1,
            state: HumanizeProgressState::Inferring,
            partial_output: String::new(),
            chunk_warnings: vec![],
        };
        let progress_json =
            serde_json::to_string(&progress).expect("serialize HumanizeProgressEvent");
        assert!(progress_json.contains("\"state\":\"inferring\""));
        let progress_back: HumanizeProgressEvent =
            serde_json::from_str(&progress_json).expect("deserialize HumanizeProgressEvent");
        assert_eq!(progress_back, progress);

        let retry_req = RetryChunkRequest {
            request_id: "req-1".to_string(),
            chunk_index: 0,
            original_chunk: "Hello world.".to_string(),
            context_before: String::new(),
            mode: RewriteMode::Deep,
            intensity: Intensity::Strong,
            options: sample_options(),
        };
        let retry_req_json =
            serde_json::to_string(&retry_req).expect("serialize RetryChunkRequest");
        let retry_req_back: RetryChunkRequest =
            serde_json::from_str(&retry_req_json).expect("deserialize RetryChunkRequest");
        assert_eq!(retry_req_back, retry_req);

        let retry_resp = RetryChunkResponse {
            request_id: "req-1".to_string(),
            chunk_index: 0,
            rewritten_chunk: "Hi there.".to_string(),
            warnings: vec![],
        };
        let retry_resp_json =
            serde_json::to_string(&retry_resp).expect("serialize RetryChunkResponse");
        let retry_resp_back: RetryChunkResponse =
            serde_json::from_str(&retry_resp_json).expect("deserialize RetryChunkResponse");
        assert_eq!(retry_resp_back, retry_resp);
    }

    #[tokio::test]
    async fn all_eleven_ipc_commands_execute_cleanly_in_m1_stub() {
        let status = get_app_status().await.expect("get_app_status");
        assert_eq!(status.state, RuntimeState::Initializing);
        assert_eq!(status.runtime_version, RUNTIME_VERSION);

        let models = list_models().await.expect("list_models");
        assert!(models.is_empty());

        assert!(select_model("".to_string(), None).await.is_err());
        assert!(select_model("qwen3".to_string(), Some(false)).await.is_err());
        assert!(download_model("".to_string()).await.is_err());
        assert!(download_model("qwen3".to_string()).await.is_err());
        assert!(cancel_download("qwen3".to_string()).await.is_ok());

        let req = HumanizeRequest {
            request_id: "r1".to_string(),
            text: "Sample text".to_string(),
            mode: RewriteMode::Standard,
            intensity: Intensity::Light,
            options: sample_options(),
        };
        assert!(humanize_document(req).await.is_err());
        assert!(cancel_humanize("r1".to_string()).await.is_ok());

        let retry_req = RetryChunkRequest {
            request_id: "r1".to_string(),
            chunk_index: 0,
            original_chunk: "Sample".to_string(),
            context_before: String::new(),
            mode: RewriteMode::Natural,
            intensity: Intensity::Balanced,
            options: sample_options(),
        };
        assert!(retry_chunk(retry_req).await.is_err());
        assert!(import_text_file("/tmp/sample.txt".to_string()).await.is_err());
        assert!(save_output_file("/tmp/out.txt".to_string(), "hi".to_string()).await.is_err());

        let retried = retry_runtime_init().await.expect("retry_runtime_init");
        assert_eq!(retried.runtime_version, RUNTIME_VERSION);

        // Verify cryptographic/HTTP scaffolding helpers
        let digest = sha256_hex(b"local-humanizer");
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, sha256_hex(b"other"));

        let tok1 = generate_session_token();
        let tok2 = generate_session_token();
        assert_eq!(tok1.len(), 64);
        assert_ne!(tok1, tok2);

        assert!(build_loopback_http_client().is_ok());
    }
}
