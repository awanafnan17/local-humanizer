use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use crate::commands::{DownloadStatus, ModelDescriptor, ModelDownloadProgressEvent};

const SHA256_BUF_SIZE: usize = 1024 * 1024; // 1 MiB streaming buffer
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(150);
const PROGRESS_EMIT_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsManifestFile {
    #[serde(default)]
    pub schema_version: String,
    pub models: Vec<ManifestModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestModelEntry {
    pub id: String,
    pub name: String,
    pub quant: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub license: String,
    pub license_url: String,
    pub is_default: bool,
    #[serde(default)]
    pub is_installed: bool,
    pub is_bundled: bool,
    pub min_recommended_ram_gb: u32,
    pub non_commercial_warning: bool,
    pub download_url: String,
    pub filename: String,
    #[serde(default)]
    pub repo_id: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub disable_thinking: bool,
    #[serde(default = "default_ctx_size")]
    pub ctx_size: u32,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_recommended_threads")]
    pub recommended_threads: u32,
    #[serde(default)]
    pub bundled_notice_files: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

fn default_ctx_size() -> u32 {
    4096
}

fn default_parallel_slots() -> u32 {
    1
}

fn default_recommended_threads() -> u32 {
    4
}

impl ManifestModelEntry {
    pub fn to_descriptor(&self, is_installed: bool) -> ModelDescriptor {
        ModelDescriptor {
            id: self.id.clone(),
            name: self.name.clone(),
            quant: self.quant.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            license: self.license.clone(),
            license_url: self.license_url.clone(),
            is_default: self.is_default,
            is_installed,
            is_bundled: self.is_bundled,
            min_recommended_ram_gb: self.min_recommended_ram_gb,
            non_commercial_warning: self.non_commercial_warning,
            download_url: self.download_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub active_model_id: Option<String>,
    #[serde(default)]
    pub use_gpu: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub bundled_models_dir: PathBuf,
    pub app_data_models_dir: PathBuf,
    pub settings_path: PathBuf,
}

impl ModelPaths {
    pub fn resolve(resource_dir: Option<&Path>, app_local_data_dir: &Path) -> Result<Self, String> {
        let bundled_models_dir = resolve_bundled_models_dir(resource_dir)?;
        let app_data_models_dir = app_local_data_dir.join("models");
        let settings_path = app_local_data_dir.join("settings.json");
        Ok(Self {
            bundled_models_dir,
            app_data_models_dir,
            settings_path,
        })
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.bundled_models_dir.join("models.json")
    }
}

pub fn resolve_bundled_models_dir(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(res) = resource_dir {
        candidates.push(res.join("resources").join("models"));
        candidates.push(res.join("models"));
    }
    let dev_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("models");
    candidates.push(dev_dir);

    for cand in &candidates {
        if cand.join("models.json").is_file() {
            return Ok(cand.clone());
        }
    }

    Err(format!(
        "Model manifest models.json not found in any resource directory (checked: {})",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub fn load_manifest_from_path(path: &Path) -> Result<ModelsManifestFile, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read model manifest {}: {e}", path.display()))?;
    let manifest: ModelsManifestFile = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse model manifest {}: {e}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &ModelsManifestFile) -> Result<(), String> {
    if manifest.models.is_empty() {
        return Err("Model manifest contains no models".to_string());
    }
    let mut default_count = 0usize;
    for entry in &manifest.models {
        if entry.id.trim().is_empty() {
            return Err("Model manifest entry has empty id".to_string());
        }
        if entry.filename.trim().is_empty()
            || entry.filename.contains('/')
            || entry.filename.contains('\\')
            || entry.filename.contains("..")
        {
            return Err(format!(
                "Model manifest entry '{}' has invalid filename '{}'",
                entry.id, entry.filename
            ));
        }
        if entry.size_bytes == 0 {
            return Err(format!(
                "Model manifest entry '{}' has zero size_bytes",
                entry.id
            ));
        }
        if entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!(
                "Model manifest entry '{}' has invalid sha256 digest",
                entry.id
            ));
        }
        validate_download_url(&entry.download_url)?;
        if entry.is_default {
            default_count += 1;
        }
    }
    if default_count != 1 {
        return Err(format!(
            "Model manifest must mark exactly one default model, found {default_count}"
        ));
    }
    Ok(())
}

pub fn validate_download_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.starts_with("https://") {
        return Ok(());
    }
    #[cfg(test)]
    if trimmed.starts_with("http://127.0.0.1:") {
        return Ok(());
    }
    Err(format!(
        "Model download URL must use HTTPS (rejected: '{trimmed}')"
    ))
}

pub fn default_model(manifest: &ModelsManifestFile) -> Result<&ManifestModelEntry, String> {
    manifest
        .models
        .iter()
        .find(|m| m.is_default)
        .or_else(|| manifest.models.first())
        .ok_or_else(|| "Model manifest contains no models".to_string())
}

pub fn find_model<'a>(
    manifest: &'a ModelsManifestFile,
    model_id: &str,
) -> Result<&'a ManifestModelEntry, String> {
    manifest
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Unknown model id '{model_id}'"))
}

pub fn load_settings(path: &Path) -> AppSettings {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create settings directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    std::fs::write(&tmp_path, json.as_bytes()).map_err(|e| {
        format!(
            "Failed to write temporary settings {}: {e}",
            tmp_path.display()
        )
    })?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| format!("Failed to persist settings {}: {e}", path.display()))?;
    Ok(())
}

/// Streaming SHA-256 verification using a 1 MiB heap buffer inside `spawn_blocking`.
pub async fn verify_file_sha256(
    path: PathBuf,
    expected_size: u64,
    expected_sha256: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || verify_file_sha256_sync(&path, expected_size, &expected_sha256))
        .await
        .map_err(|e| format!("Integrity verification task panicked: {e}"))?
}

pub fn verify_file_sha256_sync(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    use std::io::Read;

    let meta = std::fs::metadata(path)
        .map_err(|e| format!("Model file '{}' cannot be read: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "Model path '{}' is not a regular file",
            path.display()
        ));
    }
    if meta.len() != expected_size {
        return Err(format!(
            "Model file '{}' size mismatch: expected {} bytes, got {} bytes (file may be truncated or corrupted). Re-download the model or run retry_runtime_init().",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("model.gguf"),
            expected_size,
            meta.len()
        ));
    }

    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open model file '{}': {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; SHA256_BUF_SIZE];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed reading model file '{}': {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let actual = hex::encode(hasher.finalize());
    let expected_lower = expected_sha256.to_ascii_lowercase();
    if actual != expected_lower {
        return Err(format!(
            "Model file '{}' SHA-256 mismatch (expected {}, got {}). The file may be corrupted or tampered with; re-download the model or run retry_runtime_init().",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("model.gguf"),
            expected_lower,
            actual
        ));
    }
    Ok(())
}

/// Verify model presence and cryptographic integrity locally with ZERO outbound network requests.
/// Returns `Ok(Some(path))` if valid, `Ok(None)` if not installed, or `Err(msg)` if a file is
/// present but corrupt/truncated.
pub async fn resolve_and_verify_model_file(
    entry: &ManifestModelEntry,
    paths: &ModelPaths,
) -> Result<Option<PathBuf>, String> {
    let app_data_candidate = paths.app_data_models_dir.join(&entry.filename);
    if tokio::fs::try_exists(&app_data_candidate)
        .await
        .unwrap_or(false)
    {
        verify_file_sha256(
            app_data_candidate.clone(),
            entry.size_bytes,
            entry.sha256.clone(),
        )
        .await?;
        return Ok(Some(app_data_candidate));
    }

    let bundled_candidate = paths.bundled_models_dir.join(&entry.filename);
    if tokio::fs::try_exists(&bundled_candidate)
        .await
        .unwrap_or(false)
    {
        verify_file_sha256(
            bundled_candidate.clone(),
            entry.size_bytes,
            entry.sha256.clone(),
        )
        .await?;
        return Ok(Some(bundled_candidate));
    }

    Ok(None)
}

pub async fn list_model_descriptors(
    manifest: &ModelsManifestFile,
    paths: &ModelPaths,
) -> Vec<ModelDescriptor> {
    let mut out = Vec::with_capacity(manifest.models.len());
    for entry in &manifest.models {
        let installed = matches!(resolve_and_verify_model_file(entry, paths).await, Ok(Some(_)));
        out.push(entry.to_descriptor(installed));
    }
    out
}

/// Validate that the bundled `llama-server` executable and its co-located shared libraries exist
/// and are readable/executable before attempting to spawn.
pub fn verify_runtime_layout(runtime_dir: &Path) -> Result<PathBuf, String> {
    let binary_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let bin_path = runtime_dir.join(binary_name);
    let meta = std::fs::metadata(&bin_path).map_err(|e| {
        format!(
            "Bundled inference runtime binary missing at '{}': {e}. Reinstall Local Humanizer or run retry_runtime_init().",
            bin_path.display()
        )
    })?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!(
            "Bundled inference runtime binary '{}' is invalid or empty.",
            bin_path.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "Bundled inference runtime binary '{}' is not executable.",
                bin_path.display()
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut required_dylibs = vec![
            "libllama-server-impl.dylib",
            "libllama-common.1.dylib",
            "libmtmd.1.dylib",
            "libllama.1.dylib",
            "libggml.1.dylib",
            "libggml-cpu.1.dylib",
            "libggml-blas.1.dylib",
            "libggml-rpc.1.dylib",
            "libggml-base.1.dylib",
        ];
        if cfg!(target_arch = "aarch64") {
            required_dylibs.push("libggml-metal.1.dylib");
        }
        for lib in required_dylibs {
            let p = runtime_dir.join(lib);
            let m = std::fs::metadata(&p).map_err(|e| {
                format!(
                    "Required runtime library '{}' missing or broken symlink: {e}",
                    p.display()
                )
            })?;
            if !m.is_file() || m.len() == 0 {
                return Err(format!(
                    "Required runtime library '{}' is invalid or empty.",
                    p.display()
                ));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let has_dll = std::fs::read_dir(runtime_dir)
            .map_err(|e| format!("Failed to inspect runtime dir '{}': {e}", runtime_dir.display()))?
            .filter_map(|e| e.ok())
            .any(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("dll"))
                    .unwrap_or(false)
            });
        if !has_dll {
            return Err(format!(
                "No co-located runtime DLLs found in '{}'.",
                runtime_dir.display()
            ));
        }
    }

    Ok(bin_path)
}

#[derive(Debug, Default)]
pub struct DownloadManager {
    active: Mutex<Option<ActiveDownload>>,
}

#[derive(Debug)]
struct ActiveDownload {
    model_id: String,
    cancel: CancellationToken,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(&self, model_id: &str) -> Result<DownloadGuard<'_>, String> {
        let mut guard = self
            .active
            .lock()
            .map_err(|_| "Download manager lock poisoned".to_string())?;
        if let Some(current) = guard.as_ref() {
            return Err(format!(
                "Another model download ('{}') is already in progress.",
                current.model_id
            ));
        }
        let token = CancellationToken::new();
        *guard = Some(ActiveDownload {
            model_id: model_id.to_string(),
            cancel: token.clone(),
        });
        Ok(DownloadGuard {
            manager: self,
            cancel: token,
        })
    }

    pub fn cancel(&self, model_id: &str) -> bool {
        if let Ok(guard) = self.active.lock()
            && let Some(active) = guard.as_ref()
            && active.model_id == model_id
        {
            active.cancel.cancel();
            return true;
        }
        false
    }

    pub fn cancel_all(&self) {
        if let Ok(guard) = self.active.lock()
            && let Some(active) = guard.as_ref()
        {
            active.cancel.cancel();
        }
    }
}

pub struct DownloadGuard<'a> {
    manager: &'a DownloadManager,
    pub cancel: CancellationToken,
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.manager.active.lock() {
            *guard = None;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedContentRange {
    start: u64,
    end: u64,
    total: Option<u64>,
}

fn parse_content_range(header_val: &str) -> Option<ParsedContentRange> {
    let trimmed = header_val.trim();
    let rest = trimmed.strip_prefix("bytes ")?;
    let (range_part, total_part) = rest.split_once('/')?;
    let (start_str, end_str) = range_part.split_once('-')?;
    let start = start_str.trim().parse::<u64>().ok()?;
    let end = end_str.trim().parse::<u64>().ok()?;
    if end < start {
        return None;
    }
    let total = if total_part.trim() == "*" {
        None
    } else {
        let t = total_part.trim().parse::<u64>().ok()?;
        if end >= t {
            return None;
        }
        Some(t)
    };
    Some(ParsedContentRange { start, end, total })
}

#[cfg(unix)]
fn check_disk_space_bytes(dir: &Path, required_bytes: u64) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| "Invalid directory path for disk check".to_string())?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc == 0 {
        let avail = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
        let needed_with_buffer = required_bytes.saturating_add(256 * 1024 * 1024);
        if avail < needed_with_buffer {
            return Err(format!(
                "Insufficient disk space in '{}': need {} MiB (including 256 MiB buffer), only {} MiB available.",
                dir.display(),
                needed_with_buffer / (1024 * 1024),
                avail / (1024 * 1024)
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_disk_space_bytes(_dir: &Path, _required_bytes: u64) -> Result<(), String> {
    Ok(())
}

/// Resumable HTTP Range downloader writing into `<filename>.part` and verifying SHA-256 before
/// atomically renaming to `<filename>`.
pub async fn download_model_to_dir<F>(
    entry: &ManifestModelEntry,
    models_dir: &Path,
    cancel: &CancellationToken,
    mut emit_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(ModelDownloadProgressEvent),
{
    validate_download_url(&entry.download_url)?;
    tokio::fs::create_dir_all(models_dir).await.map_err(|e| {
        format!(
            "Failed to create model directory '{}': {e}",
            models_dir.display()
        )
    })?;

    let final_path = models_dir.join(&entry.filename);
    let part_path = models_dir.join(format!("{}.part", entry.filename));

    if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
        if verify_file_sha256(final_path.clone(), entry.size_bytes, entry.sha256.clone())
            .await
            .is_ok()
        {
            emit_progress(ModelDownloadProgressEvent {
                model_id: entry.id.clone(),
                downloaded_bytes: entry.size_bytes,
                total_bytes: entry.size_bytes,
                status: DownloadStatus::Completed,
                error: None,
            });
            return Ok(final_path);
        }
        let _ = tokio::fs::remove_file(&final_path).await;
    }

    let mut existing = match tokio::fs::metadata(&part_path).await {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => {
            let _ = tokio::fs::remove_file(&part_path).await;
            0
        }
        Err(_) => 0,
    };

    if existing == entry.size_bytes {
        emit_progress(ModelDownloadProgressEvent {
            model_id: entry.id.clone(),
            downloaded_bytes: existing,
            total_bytes: entry.size_bytes,
            status: DownloadStatus::Verifying,
            error: None,
        });
        if verify_file_sha256(part_path.clone(), entry.size_bytes, entry.sha256.clone())
            .await
            .is_ok()
        {
            tokio::fs::rename(&part_path, &final_path)
                .await
                .map_err(|e| format!("Failed to promote verified model file: {e}"))?;
            emit_progress(ModelDownloadProgressEvent {
                model_id: entry.id.clone(),
                downloaded_bytes: entry.size_bytes,
                total_bytes: entry.size_bytes,
                status: DownloadStatus::Completed,
                error: None,
            });
            return Ok(final_path);
        }
        let _ = tokio::fs::remove_file(&part_path).await;
        existing = 0;
    } else if existing > entry.size_bytes {
        let _ = tokio::fs::remove_file(&part_path).await;
        existing = 0;
    }

    let remaining = entry.size_bytes.saturating_sub(existing);
    if let Err(err) = check_disk_space_bytes(models_dir, remaining) {
        emit_progress(ModelDownloadProgressEvent {
            model_id: entry.id.clone(),
            downloaded_bytes: existing,
            total_bytes: entry.size_bytes,
            status: DownloadStatus::Error,
            error: Some(err.clone()),
        });
        return Err(err);
    }

    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("Too many redirects while downloading model");
        }
        if validate_download_url(attempt.url().as_str()).is_err() {
            return attempt.error("Rejected non-HTTPS redirect target during model download");
        }
        attempt.follow()
    });

    #[allow(unused_mut)]
    let mut builder = reqwest::Client::builder()
        .user_agent("LocalHumanizer/0.1.0")
        .connect_timeout(Duration::from_secs(30))
        .redirect(redirect_policy);
    #[cfg(test)]
    if entry.download_url.starts_with("http://127.0.0.1:") {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| format!("Failed to construct download HTTP client: {e}"))?;

    let mut attempt = 0u8;
    let (mut response, mut downloaded_bytes, append_mode) = loop {
        attempt += 1;
        let mut req = client.get(&entry.download_url);
        if existing > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={existing}-"));
        }

        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                emit_progress(ModelDownloadProgressEvent {
                    model_id: entry.id.clone(),
                    downloaded_bytes: existing,
                    total_bytes: entry.size_bytes,
                    status: DownloadStatus::Cancelled,
                    error: None,
                });
                return Err("Model download cancelled".to_string());
            }
            res = req.send() => {
                res.map_err(|e| {
                    let msg = format!("Model download request failed: {e}");
                    emit_progress(ModelDownloadProgressEvent {
                        model_id: entry.id.clone(),
                        downloaded_bytes: existing,
                        total_bytes: entry.size_bytes,
                        status: DownloadStatus::Error,
                        error: Some(msg.clone()),
                    });
                    msg
                })?
            }
        };

        let status = resp.status();
        if status == reqwest::StatusCode::PARTIAL_CONTENT && existing > 0 {
            let cr_header = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let parsed = parse_content_range(cr_header);
            let valid = matches!(
                parsed,
                Some(ref cr)
                    if cr.start == existing
                        && cr.total.map(|t| t == entry.size_bytes).unwrap_or(true)
            );
            if valid {
                break (resp, existing, true);
            }
            let _ = tokio::fs::remove_file(&part_path).await;
            existing = 0;
            if attempt < 2 {
                continue;
            }
            let msg = format!(
                "Server returned invalid Content-Range '{cr_header}' for partial resume"
            );
            emit_progress(ModelDownloadProgressEvent {
                model_id: entry.id.clone(),
                downloaded_bytes: 0,
                total_bytes: entry.size_bytes,
                status: DownloadStatus::Error,
                error: Some(msg.clone()),
            });
            return Err(msg);
        } else if status == reqwest::StatusCode::OK {
            if let Some(cl) = resp.content_length()
                && cl != entry.size_bytes
            {
                let msg = format!(
                    "Remote content length {cl} does not match expected {} bytes",
                    entry.size_bytes
                );
                emit_progress(ModelDownloadProgressEvent {
                    model_id: entry.id.clone(),
                    downloaded_bytes: 0,
                    total_bytes: entry.size_bytes,
                    status: DownloadStatus::Error,
                    error: Some(msg.clone()),
                });
                return Err(msg);
            }
            break (resp, 0u64, false);
        } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && existing > 0 && attempt < 2 {
            let _ = tokio::fs::remove_file(&part_path).await;
            existing = 0;
            continue;
        } else {
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                let _ = tokio::fs::remove_file(&part_path).await;
            }
            let msg = format!("Model download failed with HTTP status {status}");
            emit_progress(ModelDownloadProgressEvent {
                model_id: entry.id.clone(),
                downloaded_bytes: existing,
                total_bytes: entry.size_bytes,
                status: DownloadStatus::Error,
                error: Some(msg.clone()),
            });
            return Err(msg);
        }
    };

    let open_res = if append_mode {
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part_path)
            .await
    } else {
        tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part_path)
            .await
    };
    let file = open_res.map_err(|e| {
        let msg = format!("Failed to open partial file '{}': {e}", part_path.display());
        emit_progress(ModelDownloadProgressEvent {
            model_id: entry.id.clone(),
            downloaded_bytes,
            total_bytes: entry.size_bytes,
            status: DownloadStatus::Error,
            error: Some(msg.clone()),
        });
        msg
    })?;
    let mut writer = BufWriter::with_capacity(256 * 1024, file);

    emit_progress(ModelDownloadProgressEvent {
        model_id: entry.id.clone(),
        downloaded_bytes,
        total_bytes: entry.size_bytes,
        status: DownloadStatus::Downloading,
        error: None,
    });

    let mut last_emit = Instant::now();
    let mut last_emit_bytes = downloaded_bytes;

    loop {
        let next_chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = writer.flush().await;
                drop(writer);
                emit_progress(ModelDownloadProgressEvent {
                    model_id: entry.id.clone(),
                    downloaded_bytes,
                    total_bytes: entry.size_bytes,
                    status: DownloadStatus::Cancelled,
                    error: None,
                });
                return Err("Model download cancelled".to_string());
            }
            res = response.chunk() => res,
        };

        match next_chunk {
            Ok(Some(bytes)) => {
                if downloaded_bytes.saturating_add(bytes.len() as u64) > entry.size_bytes {
                    drop(writer);
                    let _ = tokio::fs::remove_file(&part_path).await;
                    let msg = format!(
                        "Downloaded stream exceeded expected size {} bytes",
                        entry.size_bytes
                    );
                    emit_progress(ModelDownloadProgressEvent {
                        model_id: entry.id.clone(),
                        downloaded_bytes,
                        total_bytes: entry.size_bytes,
                        status: DownloadStatus::Error,
                        error: Some(msg.clone()),
                    });
                    return Err(msg);
                }
                if let Err(e) = writer.write_all(&bytes).await {
                    let msg = format!("Failed writing partial model file: {e}");
                    emit_progress(ModelDownloadProgressEvent {
                        model_id: entry.id.clone(),
                        downloaded_bytes,
                        total_bytes: entry.size_bytes,
                        status: DownloadStatus::Error,
                        error: Some(msg.clone()),
                    });
                    return Err(msg);
                }
                downloaded_bytes += bytes.len() as u64;
                if downloaded_bytes - last_emit_bytes >= PROGRESS_EMIT_BYTES
                    || last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL
                {
                    emit_progress(ModelDownloadProgressEvent {
                        model_id: entry.id.clone(),
                        downloaded_bytes,
                        total_bytes: entry.size_bytes,
                        status: DownloadStatus::Downloading,
                        error: None,
                    });
                    last_emit = Instant::now();
                    last_emit_bytes = downloaded_bytes;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = writer.flush().await;
                let msg = format!("Network stream interrupted during download: {e}");
                emit_progress(ModelDownloadProgressEvent {
                    model_id: entry.id.clone(),
                    downloaded_bytes,
                    total_bytes: entry.size_bytes,
                    status: DownloadStatus::Error,
                    error: Some(msg.clone()),
                });
                return Err(msg);
            }
        }
    }

    writer
        .flush()
        .await
        .map_err(|e| format!("Failed flushing model file: {e}"))?;
    let file = writer.into_inner();
    file.sync_all()
        .await
        .map_err(|e| format!("Failed syncing model file to disk: {e}"))?;
    drop(file);

    if downloaded_bytes != entry.size_bytes {
        let msg = format!(
            "Download ended prematurely at {downloaded_bytes} of {} bytes; retry to resume.",
            entry.size_bytes
        );
        emit_progress(ModelDownloadProgressEvent {
            model_id: entry.id.clone(),
            downloaded_bytes,
            total_bytes: entry.size_bytes,
            status: DownloadStatus::Error,
            error: Some(msg.clone()),
        });
        return Err(msg);
    }

    emit_progress(ModelDownloadProgressEvent {
        model_id: entry.id.clone(),
        downloaded_bytes: entry.size_bytes,
        total_bytes: entry.size_bytes,
        status: DownloadStatus::Verifying,
        error: None,
    });

    if let Err(err) =
        verify_file_sha256(part_path.clone(), entry.size_bytes, entry.sha256.clone()).await
    {
        let _ = tokio::fs::remove_file(&part_path).await;
        emit_progress(ModelDownloadProgressEvent {
            model_id: entry.id.clone(),
            downloaded_bytes: entry.size_bytes,
            total_bytes: entry.size_bytes,
            status: DownloadStatus::Error,
            error: Some(err.clone()),
        });
        return Err(err);
    }

    tokio::fs::rename(&part_path, &final_path)
        .await
        .map_err(|e| format!("Failed to rename verified model file: {e}"))?;

    emit_progress(ModelDownloadProgressEvent {
        model_id: entry.id.clone(),
        downloaded_bytes: entry.size_bytes,
        total_bytes: entry.size_bytes,
        status: DownloadStatus::Completed,
        error: None,
    });

    Ok(final_path)
}

pub type SharedDownloadProgressLog = Arc<Mutex<Vec<ModelDownloadProgressEvent>>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{generate_session_token, sha256_hex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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

    fn sample_entry(url: String, data: &[u8]) -> ManifestModelEntry {
        ManifestModelEntry {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: data.len() as u64,
            sha256: sha256_hex(data),
            license: "Apache-2.0".to_string(),
            license_url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
            is_default: true,
            is_installed: false,
            is_bundled: false,
            min_recommended_ram_gb: 8,
            non_commercial_warning: false,
            download_url: url,
            filename: "test-model.gguf".to_string(),
            repo_id: "test/repo".to_string(),
            family: "qwen3".to_string(),
            tier: "default".to_string(),
            disable_thinking: true,
            ctx_size: 4096,
            parallel_slots: 1,
            recommended_threads: 4,
            bundled_notice_files: vec![],
            notes: String::new(),
        }
    }

    async fn spawn_mock_range_server(
        payload: Vec<u8>,
        force_200_ignore_range: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let data = payload.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let range_start = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                        .and_then(|l| l.split_once("bytes="))
                        .and_then(|(_, r)| r.trim().strip_suffix('-'))
                        .and_then(|s| s.parse::<usize>().ok());

                    if let Some(start) = range_start
                        && !force_200_ignore_range
                    {
                        if start >= data.len() {
                            let hdr = format!(
                                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                data.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes()).await;
                        } else {
                            let slice = &data[start..];
                            let hdr = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                start,
                                data.len() - 1,
                                data.len(),
                                slice.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes()).await;
                            let _ = sock.write_all(slice).await;
                        }
                    } else {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(&data).await;
                    }
                });
            }
        });
        (format!("http://127.0.0.1:{}/model.gguf", addr.port()), handle)
    }

    #[test]
    fn loads_real_workspace_models_json_and_validates() {
        let dir = resolve_bundled_models_dir(None).expect("dev models dir");
        let manifest = load_manifest_from_path(&dir.join("models.json")).expect("valid models.json");
        assert_eq!(manifest.models.len(), 4);
        let def = default_model(&manifest).expect("default model");
        assert_eq!(def.id, "qwen3-1.7b-q4_k_m");
        assert!(def.disable_thinking);
        let high = find_model(&manifest, "qwen3-4b-instruct-2507-q4_k_m").expect("4b");
        assert!(!high.disable_thinking);
        let non_comm = find_model(&manifest, "qwen2.5-3b-instruct-q4_k_m").expect("3b");
        assert!(non_comm.non_commercial_warning);
    }

    #[test]
    fn rejects_non_https_download_url_outside_loopback_test() {
        assert!(validate_download_url("https://huggingface.co/model.gguf").is_ok());
        assert!(validate_download_url("http://example.com/model.gguf").is_err());
        assert!(validate_download_url("file:///tmp/model.gguf").is_err());
    }

    #[tokio::test]
    async fn verify_file_sha256_detects_corrupt_and_truncated_files() {
        let tmp = TestDir::new("lh-sha");
        let p = tmp.path().join("sample.gguf");
        let payload = vec![42u8; 2 * 1024 * 1024 + 17];
        std::fs::write(&p, &payload).unwrap();
        let hash = sha256_hex(&payload);

        assert!(verify_file_sha256(p.clone(), payload.len() as u64, hash.clone()).await.is_ok());
        let trunc_err = verify_file_sha256(p.clone(), payload.len() as u64 + 10, hash.clone())
            .await
            .unwrap_err();
        assert!(trunc_err.contains("size mismatch"));

        let bad_hash = "0".repeat(64);
        let hash_err = verify_file_sha256(p, payload.len() as u64, bad_hash)
            .await
            .unwrap_err();
        assert!(hash_err.contains("SHA-256 mismatch"));
    }

    #[tokio::test]
    async fn range_downloader_resumes_206_handles_200_reset_and_rejects_bad_sha256() {
        let payload: Vec<u8> = (0..(300 * 1024)).map(|i| (i % 251) as u8).collect();
        let (url, srv) = spawn_mock_range_server(payload.clone(), false).await;

        // 1) Partial .part resume via 206
        let tmp = TestDir::new("lh-sha");
        let entry = sample_entry(url.clone(), &payload);
        let part_path = tmp.path().join("test-model.gguf.part");
        std::fs::write(&part_path, &payload[..120_000]).unwrap();

        let mut events = Vec::new();
        let cancel = CancellationToken::new();
        let final_path = download_model_to_dir(&entry, tmp.path(), &cancel, |ev| events.push(ev))
            .await
            .expect("resumed 206 download");
        assert_eq!(std::fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
        assert_eq!(events.last().unwrap().status, DownloadStatus::Completed);

        // 2) Oversized/corrupt .part cleaned & reset; 416 or 200 fallback works
        let tmp2 = TestDir::new("lh-sha2");
        let part2 = tmp2.path().join("test-model.gguf.part");
        std::fs::write(&part2, vec![9u8; payload.len() + 500]).unwrap();
        let (url_200, srv2) = spawn_mock_range_server(payload.clone(), true).await;
        let entry2 = sample_entry(url_200, &payload);
        let final2 = download_model_to_dir(&entry2, tmp2.path(), &cancel, |_| {})
            .await
            .expect("200 reset download");
        assert_eq!(std::fs::read(&final2).unwrap(), payload);

        // 3) SHA-256 mismatch removes .part and never creates final file
        let tmp3 = TestDir::new("lh-sha3");
        let mut bad_entry = sample_entry(url.clone(), &payload);
        bad_entry.sha256 = "f".repeat(64);
        let mut err_events = Vec::new();
        let res = download_model_to_dir(&bad_entry, tmp3.path(), &cancel, |ev| err_events.push(ev))
            .await;
        assert!(res.is_err());
        assert!(!tmp3.path().join("test-model.gguf").exists());
        assert!(!tmp3.path().join("test-model.gguf.part").exists());
        assert_eq!(err_events.last().unwrap().status, DownloadStatus::Error);

        srv.abort();
        srv2.abort();
    }

    #[tokio::test]
    async fn range_downloader_cancellation_prevents_final_promotion() {
        let payload: Vec<u8> = (0..(128 * 1024)).map(|i| (i % 199) as u8).collect();
        let (url, srv) = spawn_mock_range_server(payload.clone(), false).await;
        let tmp = TestDir::new("lh-sha");
        let entry = sample_entry(url, &payload);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let mut events = Vec::new();
        let res = download_model_to_dir(&entry, tmp.path(), &cancel, |ev| events.push(ev)).await;
        assert!(res.is_err());
        assert!(!tmp.path().join("test-model.gguf").exists());
        assert_eq!(events.last().unwrap().status, DownloadStatus::Cancelled);
        srv.abort();
    }

    #[test]
    fn parse_content_range_validates_bounds() {
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some(ParsedContentRange {
                start: 100,
                end: 199,
                total: Some(200)
            })
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some(ParsedContentRange {
                start: 100,
                end: 199,
                total: None
            })
        );
        assert_eq!(parse_content_range("bytes 200-100/300"), None);
        assert_eq!(parse_content_range("bytes 100-200/200"), None);
        assert_eq!(parse_content_range("bytes 100-250/200"), None);
        assert_eq!(parse_content_range("invalid"), None);
    }

    #[tokio::test]
    async fn downloader_recovers_from_bad_full_part_416_and_wrong_content_range() {
        let payload: Vec<u8> = (0..(96 * 1024)).map(|i| (i % 233) as u8).collect();
        let cancel = CancellationToken::new();

        // 1) .part == size_bytes with bad SHA-256 deletes .part and re-downloads cleanly
        let (url_ok, srv_ok) = spawn_mock_range_server(payload.clone(), false).await;
        let tmp1 = TestDir::new("lh-part-badsha");
        let part1 = tmp1.path().join("test-model.gguf.part");
        std::fs::write(&part1, vec![0xAAu8; payload.len()]).unwrap();
        let entry1 = sample_entry(url_ok, &payload);
        let final1 = download_model_to_dir(&entry1, tmp1.path(), &cancel, |_| {})
            .await
            .expect("should discard bad full .part and download valid file");
        assert_eq!(std::fs::read(&final1).unwrap(), payload);
        srv_ok.abort();

        // 2) 416 Range Not Satisfiable on existing < size_bytes falls back to full 200 download
        let listener_416 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_416 = listener_416.local_addr().unwrap().port();
        let data_416 = payload.clone();
        let srv_416 = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener_416.accept().await else {
                    break;
                };
                let data = data_416.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    if req.to_ascii_lowercase().contains("range:") {
                        let hdr = format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                    } else {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(&data).await;
                    }
                });
            }
        });
        let tmp2 = TestDir::new("lh-416");
        std::fs::write(tmp2.path().join("test-model.gguf.part"), &payload[..1024]).unwrap();
        let entry2 = sample_entry(format!("http://127.0.0.1:{port_416}/model.gguf"), &payload);
        let final2 = download_model_to_dir(&entry2, tmp2.path(), &cancel, |_| {})
            .await
            .expect("416 should unlink .part and retry full download");
        assert_eq!(std::fs::read(&final2).unwrap(), payload);
        srv_416.abort();

        // 3) 206 with mismatched Content-Range unlinks .part and retries full download
        let listener_cr = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_cr = listener_cr.local_addr().unwrap().port();
        let data_cr = payload.clone();
        let srv_cr = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener_cr.accept().await else {
                    break;
                };
                let data = data_cr.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    if req.to_ascii_lowercase().contains("range:") {
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len() - 1,
                            data.len(),
                            data.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(&data).await;
                    } else {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(&data).await;
                    }
                });
            }
        });
        let tmp3 = TestDir::new("lh-badcr");
        std::fs::write(tmp3.path().join("test-model.gguf.part"), &payload[..2048]).unwrap();
        let entry3 = sample_entry(format!("http://127.0.0.1:{port_cr}/model.gguf"), &payload);
        let final3 = download_model_to_dir(&entry3, tmp3.path(), &cancel, |_| {})
            .await
            .expect("mismatched Content-Range should reset .part and retry");
        assert_eq!(std::fs::read(&final3).unwrap(), payload);
        srv_cr.abort();
    }

    #[tokio::test]
    async fn downloader_rejects_non_https_redirect_target() {
        let payload = b"model-bytes".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let hdr = "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/evil.gguf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = sock.write_all(hdr.as_bytes()).await;
            }
        });

        let tmp = TestDir::new("lh-redir");
        let entry = sample_entry(format!("http://127.0.0.1:{port}/model.gguf"), &payload);
        let cancel = CancellationToken::new();
        let res = download_model_to_dir(&entry, tmp.path(), &cancel, |_| {}).await;
        assert!(res.is_err());
        srv.abort();
    }
}
