use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hex;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::engine::{
    humanize_document_impl, retry_chunk_impl, EngineCoordinator, EngineError, LlamaServerClient,
};
use crate::models::{
    default_model, download_model_to_dir, find_model, list_model_descriptors,
    load_manifest_from_path, load_settings, resolve_and_verify_model_file, save_settings,
    verify_runtime_layout, AppSettings, DownloadManager, ModelPaths, ModelsManifestFile,
};
use crate::runtime::{
    detect_physical_ram_bytes, global_runtime_slot, global_shutdown_flag, reap_global_runtime_sync,
    resolve_runtime_dir, select_backend_mode, start_and_wait_into_global_slot,
    supports_metal_acceleration, ServerEndpoint,
};

pub const RUNTIME_VERSION: &str = "b12600";
pub const EVENT_MODEL_DOWNLOAD_PROGRESS: &str = "model-download-progress";
pub const EVENT_HUMANIZE_PROGRESS: &str = "humanize-progress";
pub const MAX_IMPORT_BYTES: u64 = 2 * 1024 * 1024; // 2 MiB
pub const MAX_SAVE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

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
        backend_mode: if supports_metal_acceleration() {
            BackendMode::Metal
        } else {
            BackendMode::Cpu
        },
        ram_bytes: detect_physical_ram_bytes(),
        error_message: None,
        runtime_version: RUNTIME_VERSION.to_string(),
    }
}

pub struct AppState {
    pub status: Arc<Mutex<AppStatus>>,
    pub model_paths: Mutex<Option<ModelPaths>>,
    pub runtime_dir: Mutex<Option<PathBuf>>,
    pub manifest: Mutex<Option<ModelsManifestFile>>,
    pub active_endpoint: Arc<Mutex<Option<ServerEndpoint>>>,
    pub lifecycle_lock: tokio::sync::Mutex<()>,
    pub download_manager: Arc<DownloadManager>,
    pub engine: Arc<EngineCoordinator>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(default_stub_status())),
            model_paths: Mutex::new(None),
            runtime_dir: Mutex::new(None),
            manifest: Mutex::new(None),
            active_endpoint: Arc::new(Mutex::new(None)),
            lifecycle_lock: tokio::sync::Mutex::new(()),
            download_manager: Arc::new(DownloadManager::new()),
            engine: Arc::new(EngineCoordinator::new()),
        }
    }

    pub fn update_status<F>(&self, f: F) -> AppStatus
    where
        F: FnOnce(&mut AppStatus),
    {
        let mut guard = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
        guard.clone()
    }

    pub fn get_status_snapshot(&self) -> AppStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn ensure_manifest_and_paths(
    app: &AppHandle,
    state: &AppState,
) -> Result<(ModelPaths, ModelsManifestFile), String> {
    let existing_paths = state
        .model_paths
        .lock()
        .map_err(|_| "State lock poisoned".to_string())?
        .clone();
    let paths = if let Some(p) = existing_paths {
        p
    } else {
        let resource_dir = app.path().resource_dir().ok();
        let app_local_data_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|e| format!("Failed to resolve app local data directory: {e}"))?;
        let resolved = ModelPaths::resolve(resource_dir.as_deref(), &app_local_data_dir)?;
        if let Ok(mut g) = state.model_paths.lock() {
            *g = Some(resolved.clone());
        }
        resolved
    };

    let existing_manifest = state
        .manifest
        .lock()
        .map_err(|_| "State lock poisoned".to_string())?
        .clone();
    let manifest = if let Some(m) = existing_manifest {
        m
    } else {
        let loaded = load_manifest_from_path(&paths.manifest_path())?;
        if let Ok(mut g) = state.manifest.lock() {
            *g = Some(loaded.clone());
        }
        loaded
    };

    Ok((paths, manifest))
}

fn ensure_runtime_dir(app: &AppHandle, state: &AppState) -> Result<PathBuf, String> {
    if let Ok(g) = state.runtime_dir.lock()
        && let Some(dir) = g.as_ref()
    {
        return Ok(dir.clone());
    }
    let resource_dir = app.path().resource_dir().ok();
    let dir = resolve_runtime_dir(resource_dir.as_deref())?;
    if let Ok(mut g) = state.runtime_dir.lock() {
        *g = Some(dir.clone());
    }
    Ok(dir)
}

struct VerifiedSwitchTarget {
    entry: crate::models::ManifestModelEntry,
    backend_mode: BackendMode,
    effective_use_gpu: Option<bool>,
    model_path: PathBuf,
    settings: AppSettings,
}

async fn prepare_verified_switch_target(
    state: &AppState,
    paths: &ModelPaths,
    manifest: &ModelsManifestFile,
    runtime_dir: &Path,
    requested_model_id: Option<String>,
    requested_use_gpu: Option<Option<bool>>,
) -> Result<Result<VerifiedSwitchTarget, AppStatus>, String> {
    let has_live_endpoint = state
        .active_endpoint
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let is_explicit_switch = requested_model_id.is_some();

    let settings: AppSettings = load_settings(&paths.settings_path);
    let target_model_id = if let Some(id) = requested_model_id {
        id
    } else if let Some(saved_id) = settings.active_model_id.clone() {
        if find_model(manifest, &saved_id).is_ok() {
            saved_id
        } else {
            default_model(manifest)?.id.clone()
        }
    } else {
        default_model(manifest)?.id.clone()
    };

    let entry = find_model(manifest, &target_model_id)?.clone();

    let effective_use_gpu = match requested_use_gpu {
        Some(explicit) => explicit,
        None => settings.use_gpu,
    };
    let backend_mode = select_backend_mode(effective_use_gpu)?;

    if let Err(err) = verify_runtime_layout(runtime_dir) {
        if has_live_endpoint && is_explicit_switch {
            return Err(err);
        }
        let status = state.update_status(|s| {
            s.state = RuntimeState::Error;
            s.active_model_id = entry.id.clone();
            s.model_ready = false;
            s.backend_mode = backend_mode;
            s.error_message = Some(err);
        });
        return Ok(Err(status));
    }

    let model_path = match resolve_and_verify_model_file(&entry, paths).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            let msg = format!(
                "Model '{}' ({}) is not installed locally. Download it first or select the bundled default model.",
                entry.name, entry.id
            );
            if has_live_endpoint && is_explicit_switch {
                return Err(msg);
            }
            let status = state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.active_model_id = entry.id.clone();
                s.model_ready = false;
                s.backend_mode = backend_mode;
                s.error_message = Some(msg);
            });
            return Ok(Err(status));
        }
        Err(err) => {
            if has_live_endpoint && is_explicit_switch {
                return Err(err);
            }
            let status = state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.active_model_id = entry.id.clone();
                s.model_ready = false;
                s.backend_mode = backend_mode;
                s.error_message = Some(err);
            });
            return Ok(Err(status));
        }
    };

    Ok(Ok(VerifiedSwitchTarget {
        entry,
        backend_mode,
        effective_use_gpu,
        model_path,
        settings,
    }))
}

pub async fn initialize_or_switch_runtime(
    app: &AppHandle,
    state: &AppState,
    requested_model_id: Option<String>,
    requested_use_gpu: Option<Option<bool>>,
) -> Result<AppStatus, String> {
    let _lifecycle = state.lifecycle_lock.lock().await;

    let has_live_endpoint = state
        .active_endpoint
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let is_explicit_switch = requested_model_id.is_some();

    let (paths, manifest) = match ensure_manifest_and_paths(app, state) {
        Ok(res) => res,
        Err(err) => {
            if has_live_endpoint && is_explicit_switch {
                return Err(err);
            }
            let status = state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.model_ready = false;
                s.error_message = Some(err.clone());
            });
            return Ok(status);
        }
    };

    let runtime_dir = match ensure_runtime_dir(app, state) {
        Ok(dir) => dir,
        Err(err) => {
            if has_live_endpoint && is_explicit_switch {
                return Err(err);
            }
            let status = state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.model_ready = false;
                s.error_message = Some(err);
            });
            return Ok(status);
        }
    };

    let mut target = match prepare_verified_switch_target(
        state,
        &paths,
        &manifest,
        &runtime_dir,
        requested_model_id,
        requested_use_gpu,
    )
    .await?
    {
        Ok(t) => t,
        Err(status) => return Ok(status),
    };

    // Cancel any active generation and reap previous llama-server BEFORE spawning replacement.
    state.engine.cancel_all_active();
    if let Ok(mut ep) = state.active_endpoint.lock() {
        *ep = None;
    }
    reap_global_runtime_sync();

    state.update_status(|s| {
        s.state = RuntimeState::StartingRuntime;
        s.active_model_id = target.entry.id.clone();
        s.model_ready = false;
        s.backend_mode = target.backend_mode;
        s.error_message = None;
    });

    match start_and_wait_into_global_slot(
        &runtime_dir,
        &target.model_path,
        &target.entry,
        target.backend_mode,
        global_shutdown_flag(),
    )
    .await
    {
        Ok(endpoint) => {
            // Persist preference only after verified model & healthy runtime startup (metadata only)
            target.settings.active_model_id = Some(target.entry.id.clone());
            if requested_use_gpu.is_some() {
                target.settings.use_gpu = target.effective_use_gpu;
            }
            let _ = save_settings(&paths.settings_path, &target.settings);

            if let Ok(mut ep) = state.active_endpoint.lock() {
                *ep = Some(endpoint);
            }
            let status = state.update_status(|s| {
                s.state = RuntimeState::Ready;
                s.active_model_id = target.entry.id;
                s.model_ready = true;
                s.backend_mode = target.backend_mode;
                s.error_message = None;
            });
            Ok(status)
        }
        Err(err) => {
            let status = state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.model_ready = false;
                s.error_message = Some(err);
            });
            Ok(status)
        }
    }
}

#[tauri::command]
pub async fn get_app_status(state: State<'_, AppState>) -> Result<AppStatus, String> {
    Ok(state.get_status_snapshot())
}

#[tauri::command]
pub async fn list_models(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ModelDescriptor>, String> {
    let (paths, manifest) = ensure_manifest_and_paths(&app, &state)?;
    Ok(list_model_descriptors(&manifest, &paths).await)
}

#[tauri::command]
pub async fn select_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
    use_gpu: Option<bool>,
) -> Result<AppStatus, String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    initialize_or_switch_runtime(&app, &state, Some(trimmed.to_string()), Some(use_gpu)).await
}

fn begin_download_status(state: &AppState) -> AppStatus {
    let mut guard = state.status.lock().unwrap_or_else(|e| e.into_inner());
    let prev = guard.clone();
    if !matches!(
        guard.state,
        RuntimeState::Verifying | RuntimeState::StartingRuntime
    ) {
        guard.state = RuntimeState::Downloading;
        guard.error_message = None;
    }
    prev
}

fn finish_download_status(
    state: &AppState,
    prev_status: &AppStatus,
    model_id: &str,
    succeeded: bool,
) -> bool {
    let is_endpoint_live = state
        .active_endpoint
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let mut guard = state.status.lock().unwrap_or_else(|e| e.into_inner());

    if guard.state != RuntimeState::Downloading {
        return false;
    }

    if succeeded {
        if !is_endpoint_live
            && (guard.active_model_id.is_empty() || guard.active_model_id == model_id)
        {
            return true;
        }
        if is_endpoint_live {
            guard.state = RuntimeState::Ready;
            guard.error_message = None;
        } else {
            guard.state = prev_status.state;
            guard.error_message = prev_status.error_message.clone();
        }
        return false;
    }

    if is_endpoint_live {
        guard.state = RuntimeState::Ready;
        guard.error_message = None;
    } else {
        guard.state = prev_status.state;
        guard.error_message = prev_status.error_message.clone();
    }
    false
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    let (paths, manifest) = ensure_manifest_and_paths(&app, &state)?;
    let entry = find_model(&manifest, trimmed)?.clone();
    let guard = state.download_manager.try_acquire(&entry.id)?;

    let prev_status = begin_download_status(&state);

    let res = download_model_to_dir(&entry, &paths.app_data_models_dir, &guard.cancel, |ev| {
        let _ = app.emit(EVENT_MODEL_DOWNLOAD_PROGRESS, &ev);
    })
    .await;

    match res {
        Ok(_) => {
            let should_init = finish_download_status(&state, &prev_status, &entry.id, true);
            drop(guard);
            if should_init {
                let _ = initialize_or_switch_runtime(
                    &app,
                    &state,
                    Some(entry.id.clone()),
                    None,
                )
                .await;
            }
            Ok(())
        }
        Err(err) => {
            let _ = finish_download_status(&state, &prev_status, &entry.id, false);
            drop(guard);
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn cancel_download(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("model_id cannot be empty".to_string());
    }
    state.download_manager.cancel(model_id.trim());
    Ok(())
}

fn handle_transport_crash_if_dead(state: &AppState, fallback_err: String) -> String {
    let mut dead_msg = None;
    if let Ok(mut slot) = global_runtime_slot().lock()
        && let Some(handle) = slot.as_mut()
        && let Err(exit_msg) = handle.try_check_alive()
    {
        handle.reap_sync();
        *slot = None;
        dead_msg = Some(exit_msg);
    }

    if let Some(msg) = dead_msg {
        if let Ok(mut ep) = state.active_endpoint.lock() {
            *ep = None;
        }
        state.update_status(|s| {
            s.state = RuntimeState::Error;
            s.model_ready = false;
            s.error_message = Some(msg.clone());
        });
        return msg;
    }
    fallback_err
}

#[tauri::command]
pub async fn humanize_document(
    app: AppHandle,
    state: State<'_, AppState>,
    req: HumanizeRequest,
) -> Result<HumanizeResponse, String> {
    if req.request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }

    let endpoint = state
        .active_endpoint
        .lock()
        .map_err(|_| "State lock poisoned".to_string())?
        .clone();

    let Some(endpoint) = endpoint else {
        // Allow empty input to return clean 0-chunk response even before runtime check? Better: require ready unless empty.
        if req.text.trim().is_empty() {
            let dummy = crate::runtime::ServerEndpoint {
                host: "127.0.0.1".to_string(),
                port: 0,
                bearer_token: String::new(),
                disable_thinking: true,
                backend_mode: BackendMode::Cpu,
                model_id: String::new(),
            };
            let client = LlamaServerClient::new(dummy)?;
            return humanize_document_impl(&client, &state.engine, req, true, |ev| {
                let _ = app.emit(EVENT_HUMANIZE_PROGRESS, &ev);
            })
            .await
            .map_err(EngineError::into_message);
        }
        return Err(
            "Inference runtime is not ready yet. Please wait for initialization or retry runtime startup."
                .to_string(),
        );
    };

    let disable_thinking = endpoint.disable_thinking;
    let client = LlamaServerClient::new(endpoint)?;
    match humanize_document_impl(&client, &state.engine, req, disable_thinking, |ev| {
        let _ = app.emit(EVENT_HUMANIZE_PROGRESS, &ev);
    })
    .await
    {
        Ok(resp) => Ok(resp),
        Err(EngineError::Transport(msg)) => Err(handle_transport_crash_if_dead(&state, msg)),
        Err(other) => Err(other.into_message()),
    }
}

#[tauri::command]
pub async fn cancel_humanize(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<(), String> {
    if request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }
    state.engine.cancel_request(request_id.trim());
    Ok(())
}

#[tauri::command]
pub async fn retry_chunk(
    state: State<'_, AppState>,
    req: RetryChunkRequest,
) -> Result<RetryChunkResponse, String> {
    if req.request_id.trim().is_empty() {
        return Err("request_id cannot be empty".to_string());
    }
    let endpoint = state
        .active_endpoint
        .lock()
        .map_err(|_| "State lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "Inference runtime is not ready yet.".to_string())?;

    let disable_thinking = endpoint.disable_thinking;
    let client = LlamaServerClient::new(endpoint)?;
    match retry_chunk_impl(&client, &state.engine, req, disable_thinking).await {
        Ok(resp) => Ok(resp),
        Err(EngineError::Transport(msg)) => Err(handle_transport_crash_if_dead(&state, msg)),
        Err(other) => Err(other.into_message()),
    }
}

pub async fn read_import_text_file(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("Invalid file path".to_string());
    }
    let p = Path::new(trimmed);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "txt" | "md" | "markdown") {
        return Err(format!(
            "Unsupported file extension '.{ext}'. Only .txt, .md, and .markdown UTF-8 files are supported."
        ));
    }

    let meta = tokio::fs::metadata(p)
        .await
        .map_err(|e| format!("Cannot access file '{}': {e}", p.display()))?;
    if !meta.is_file() {
        return Err(format!("Path '{}' is not a regular file", p.display()));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "File '{}' exceeds the 2 MiB import limit ({} bytes).",
            p.display(),
            meta.len()
        ));
    }

    let bytes = tokio::fs::read(p)
        .await
        .map_err(|e| format!("Failed reading file '{}': {e}", p.display()))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        format!(
            "File '{}' is not valid UTF-8 text.",
            p.display()
        )
    })?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

pub async fn write_output_text_file(path: &str, content: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("Invalid output file path".to_string());
    }
    if content.len() > MAX_SAVE_BYTES {
        return Err(format!(
            "Output content exceeds maximum save size ({} bytes).",
            MAX_SAVE_BYTES
        ));
    }
    let p = Path::new(trimmed);
    let parent = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| "Output path must include a parent directory".to_string())?;
    let parent_meta = tokio::fs::metadata(parent)
        .await
        .map_err(|e| format!("Output directory '{}' does not exist: {e}", parent.display()))?;
    if !parent_meta.is_dir() {
        return Err(format!(
            "Output parent '{}' is not a directory",
            parent.display()
        ));
    }

    let tmp_name = format!(
        ".{}.{}.{}.tmp",
        p.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output"),
        std::process::id(),
        &generate_session_token()[..12]
    );
    let tmp_path = parent.join(tmp_name);
    let mut f = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create output file '{}': {e}", p.display()))?;
    let write_res: Result<(), String> = async {
        use tokio::io::AsyncWriteExt;
        f.write_all(content.as_bytes())
            .await
            .map_err(|e| format!("Failed writing output file '{}': {e}", p.display()))?;
        f.sync_all()
            .await
            .map_err(|e| format!("Failed syncing output file '{}': {e}", p.display()))?;
        drop(f);
        tokio::fs::rename(&tmp_path, p)
            .await
            .map_err(|e| format!("Failed finalizing output file '{}': {e}", p.display()))?;
        Ok(())
    }
    .await;
    if write_res.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
    write_res
}

#[tauri::command]
pub async fn import_text_file(path: String) -> Result<String, String> {
    read_import_text_file(&path).await
}

#[tauri::command]
pub async fn save_output_file(path: String, content: String) -> Result<(), String> {
    write_output_text_file(&path, &content).await
}

#[tauri::command]
pub async fn retry_runtime_init(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppStatus, String> {
    initialize_or_switch_runtime(&app, &state, None, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("{prefix}-{}", &generate_session_token()[..12]));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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
    async fn import_and_save_file_enforce_limits_extensions_utf8_bom_and_atomic_write() {
        let tmp = TestDir::new("lh-cmd");
        let txt_path = tmp.path().join("doc.md");
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice("Hello café 🚀".as_bytes());
        std::fs::write(&txt_path, &with_bom).unwrap();

        let imported = read_import_text_file(txt_path.to_str().unwrap())
            .await
            .expect("valid md import");
        assert_eq!(imported, "Hello café 🚀");

        // Unsupported extension rejected
        let bad_ext = tmp.path().join("doc.exe");
        std::fs::write(&bad_ext, b"binary").unwrap();
        assert!(read_import_text_file(bad_ext.to_str().unwrap()).await.is_err());

        // Invalid UTF-8 rejected
        let bad_utf8 = tmp.path().join("bad.txt");
        std::fs::write(&bad_utf8, [0xFF, 0xFE, 0x00]).unwrap();
        assert!(read_import_text_file(bad_utf8.to_str().unwrap()).await.is_err());

        // Oversized > 2 MiB rejected
        let big = tmp.path().join("big.txt");
        std::fs::write(&big, vec![b'a'; (MAX_IMPORT_BYTES as usize) + 10]).unwrap();
        assert!(read_import_text_file(big.to_str().unwrap()).await.is_err());

        // Atomic save works and rejects missing parent
        let out_path = tmp.path().join("saved.txt");
        write_output_text_file(out_path.to_str().unwrap(), "Saved text 🚀")
            .await
            .expect("atomic save");
        assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "Saved text 🚀");

        let bad_parent = tmp.path().join("missing_dir").join("saved.txt");
        assert!(write_output_text_file(bad_parent.to_str().unwrap(), "hi")
            .await
            .is_err());

        // Failed rename (target is an existing directory) cleans up its temporary file
        let existing_dir_target = tmp.path().join("existing_dir.txt");
        std::fs::create_dir(&existing_dir_target).unwrap();
        assert!(write_output_text_file(existing_dir_target.to_str().unwrap(), "hi")
            .await
            .is_err());
        let leftover_tmps: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftover_tmps.is_empty(),
            "temporary file left behind after rename failure: {:?}",
            leftover_tmps
        );
    }

    #[test]
    fn crypto_and_http_helpers_work() {
        let digest = sha256_hex(b"local-humanizer");
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, sha256_hex(b"other"));

        let tok1 = generate_session_token();
        let tok2 = generate_session_token();
        assert_eq!(tok1.len(), 64);
        assert_ne!(tok1, tok2);

        assert!(build_loopback_http_client().is_ok());
    }

    #[test]
    fn download_status_preserves_prior_error_and_does_not_clobber_starting_runtime() {
        let state = AppState::new();
        state.update_status(|s| {
            s.state = RuntimeState::Error;
            s.error_message = Some("Initial model missing".to_string());
        });

        let prev = begin_download_status(&state);
        assert_eq!(state.get_status_snapshot().state, RuntimeState::Downloading);
        assert_eq!(state.get_status_snapshot().error_message, None);

        // On download failure when !is_endpoint_live, both Error state and original error_message are restored
        let should_init = finish_download_status(&state, &prev, "qwen3-4b", false);
        assert!(!should_init);
        let snap = state.get_status_snapshot();
        assert_eq!(snap.state, RuntimeState::Error);
        assert_eq!(snap.error_message.as_deref(), Some("Initial model missing"));

        // Concurrent StartingRuntime transition must not be clobbered by download completion/error
        let prev2 = begin_download_status(&state);
        state.update_status(|s| {
            s.state = RuntimeState::StartingRuntime;
            s.error_message = None;
        });
        let should_init2 = finish_download_status(&state, &prev2, "qwen3-4b", true);
        assert!(!should_init2);
        assert_eq!(state.get_status_snapshot().state, RuntimeState::StartingRuntime);
    }

    #[tokio::test]
    async fn select_model_missing_file_preserves_settings_and_live_endpoint() {
        let tmp = TestDir::new("lh-switch");
        let bundled = tmp.path().join("bundled");
        let app_data = tmp.path().join("app_data");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();

        let paths = ModelPaths {
            bundled_models_dir: bundled,
            app_data_models_dir: app_data.clone(),
            settings_path: tmp.path().join("settings.json"),
        };
        save_settings(
            &paths.settings_path,
            &AppSettings {
                active_model_id: Some("qwen3-1.7b-q4_k_m".to_string()),
                use_gpu: None,
            },
        )
        .unwrap();

        let manifest_dir = crate::models::resolve_bundled_models_dir(None).unwrap();
        let manifest = load_manifest_from_path(&manifest_dir.join("models.json")).unwrap();
        let runtime_dir = resolve_runtime_dir(None).unwrap();

        let state = AppState::new();
        {
            let mut ep = state.active_endpoint.lock().unwrap();
            *ep = Some(ServerEndpoint {
                host: "127.0.0.1".to_string(),
                port: 18080,
                bearer_token: "tok".to_string(),
                disable_thinking: true,
                backend_mode: BackendMode::Cpu,
                model_id: "qwen3-1.7b-q4_k_m".to_string(),
            });
        }
        state.update_status(|s| {
            s.state = RuntimeState::Ready;
            s.active_model_id = "qwen3-1.7b-q4_k_m".to_string();
            s.model_ready = true;
        });

        let res = prepare_verified_switch_target(
            &state,
            &paths,
            &manifest,
            &runtime_dir,
            Some("qwen3-4b-instruct-2507-q4_k_m".to_string()),
            None,
        )
        .await;

        assert!(res.is_err(), "switching to uninstalled model must fail");
        assert_eq!(
            load_settings(&paths.settings_path).active_model_id.as_deref(),
            Some("qwen3-1.7b-q4_k_m"),
            "settings.json must NOT be overwritten before model verification"
        );
        assert!(
            state.active_endpoint.lock().unwrap().is_some(),
            "currently healthy endpoint must NOT be cleared"
        );
        assert_eq!(state.get_status_snapshot().state, RuntimeState::Ready);
    }

    #[test]
    fn finish_download_status_does_not_clobber_concurrent_select_model_error() {
        for succeeded in [true, false] {
            let state = AppState::new();
            {
                let mut ep = state.active_endpoint.lock().unwrap();
                *ep = Some(ServerEndpoint {
                    host: "127.0.0.1".to_string(),
                    port: 18080,
                    bearer_token: "tok".to_string(),
                    disable_thinking: true,
                    backend_mode: BackendMode::Cpu,
                    model_id: "qwen3-1.7b-q4_k_m".to_string(),
                });
            }
            state.update_status(|s| {
                s.state = RuntimeState::Ready;
                s.active_model_id = "qwen3-1.7b-q4_k_m".to_string();
                s.model_ready = true;
                s.error_message = None;
            });

            let prev = begin_download_status(&state);
            assert_eq!(state.get_status_snapshot().state, RuntimeState::Downloading);

            // Simulate concurrent select_model clearing endpoint -> StartingRuntime -> Error
            {
                let mut ep = state.active_endpoint.lock().unwrap();
                *ep = None;
            }
            state.update_status(|s| {
                s.state = RuntimeState::StartingRuntime;
                s.model_ready = false;
                s.error_message = None;
            });
            state.update_status(|s| {
                s.state = RuntimeState::Error;
                s.model_ready = false;
                s.error_message = Some("Failed to start switched model".to_string());
            });

            let should_init = finish_download_status(&state, &prev, "qwen3-4b-instruct-2507-q4_k_m", succeeded);
            assert!(!should_init);

            let snap = state.get_status_snapshot();
            assert_eq!(snap.state, RuntimeState::Error);
            assert!(!snap.model_ready);
            assert_eq!(
                snap.error_message.as_deref(),
                Some("Failed to start switched model")
            );
        }
    }
}
