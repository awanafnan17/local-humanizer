use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::commands::{build_loopback_http_client, generate_session_token, BackendMode};
use crate::models::{verify_runtime_layout, ManifestModelEntry};

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);
const HEALTH_POLL_TIMEOUT: Duration = Duration::from_secs(120);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const REAP_GRACE_PERIOD: Duration = Duration::from_millis(3000);
const REAP_POLL_STEP: Duration = Duration::from_millis(50);
const STDERR_MAX_LINES: usize = 32;
const STDERR_MAX_BYTES: usize = 4096;

static GLOBAL_RUNTIME_HANDLE: OnceLock<Arc<Mutex<Option<LlamaServerHandle>>>> = OnceLock::new();
static GLOBAL_SHUTDOWN_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static GLOBAL_SHUTDOWN_NOTIFY: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();

pub fn global_runtime_slot() -> &'static Arc<Mutex<Option<LlamaServerHandle>>> {
    GLOBAL_RUNTIME_HANDLE.get_or_init(|| Arc::new(Mutex::new(None)))
}

pub fn global_shutdown_flag() -> &'static Arc<AtomicBool> {
    GLOBAL_SHUTDOWN_FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

pub fn global_shutdown_notify() -> &'static Arc<tokio::sync::Notify> {
    GLOBAL_SHUTDOWN_NOTIFY.get_or_init(|| Arc::new(tokio::sync::Notify::new()))
}

pub fn host_platform_dir() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win-cpu-x64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        "linux-x64"
    }
}

pub fn resolve_runtime_dir(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let platform = host_platform_dir();
    let binary_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    let mut candidates = Vec::new();
    if let Some(res) = resource_dir {
        candidates.push(res.join("resources").join("runtime").join(platform));
        candidates.push(res.join("runtime").join(platform));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("runtime")
            .join(platform),
    );

    for cand in &candidates {
        if cand.join(binary_name).is_file() {
            return Ok(cand.clone());
        }
    }

    Err(format!(
        "Bundled llama-server runtime for '{platform}' not found (checked: {})",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub fn detect_physical_ram_bytes() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let mut memsize: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let rc = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut memsize as *mut u64).cast::<libc::c_void>(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 {
            return memsize;
        }
        0
    }

    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{
            GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok != 0 {
            return status.ullTotalPhys;
        }
        0
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
            for line in contents.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let kb_str = rest.split_whitespace().next().unwrap_or("0");
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb.saturating_mul(1024);
                    }
                }
            }
        }
        0
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        0
    }
}

pub fn supports_metal_acceleration() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

pub fn select_backend_mode(requested_gpu: Option<bool>) -> Result<BackendMode, String> {
    if supports_metal_acceleration() {
        if requested_gpu == Some(false) {
            Ok(BackendMode::Cpu)
        } else {
            Ok(BackendMode::Metal)
        }
    } else if requested_gpu == Some(true) {
        Err(
            "GPU acceleration (Metal) is not supported on this host architecture; please use CPU mode."
                .to_string(),
        )
    } else {
        Ok(BackendMode::Cpu)
    }
}

pub fn pick_ephemeral_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind ephemeral loopback port on 127.0.0.1:0: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to inspect ephemeral loopback port: {e}"))?
        .port();
    Ok(port)
}

#[derive(Debug, Clone)]
pub struct ServerEndpoint {
    pub host: String,
    pub port: u16,
    pub bearer_token: String,
    pub disable_thinking: bool,
    pub backend_mode: BackendMode,
    pub model_id: String,
}

impl ServerEndpoint {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[derive(Debug)]
struct StderrRingBuffer {
    lines: VecDeque<String>,
    total_bytes: usize,
}

impl StderrRingBuffer {
    fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(STDERR_MAX_LINES),
            total_bytes: 0,
        }
    }

    fn push_sanitized(&mut self, line: &str, token: &str) {
        let mut sanitized = if !token.is_empty() && line.contains(token) {
            line.replace(token, "[REDACTED]")
        } else {
            line.to_string()
        };
        if sanitized.len() > 512 {
            let mut end = 512;
            while !sanitized.is_char_boundary(end) {
                end -= 1;
            }
            sanitized.truncate(end);
            sanitized.push_str("...");
        }
        let added = sanitized.len() + 1;
        while (self.lines.len() >= STDERR_MAX_LINES
            || self.total_bytes.saturating_add(added) > STDERR_MAX_BYTES)
            && !self.lines.is_empty()
        {
            if let Some(old) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(old.len() + 1);
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(added);
        self.lines.push_back(sanitized);
    }

    fn summary(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join(" | ")
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsJobGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsJobGuard {}
#[cfg(windows)]
unsafe impl Sync for WindowsJobGuard {}

#[cfg(windows)]
impl Drop for WindowsJobGuard {
    fn drop(&mut self) {
        self.terminate_and_close();
    }
}

#[cfg(windows)]
impl WindowsJobGuard {
    fn create_and_assign(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(format!(
                "CreateJobObjectW failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set_ok == 0 {
            let err = std::io::Error::last_os_error();
            unsafe { CloseHandle(job) };
            return Err(format!("SetInformationJobObject failed: {err}"));
        }

        let proc_handle = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let assign_ok = unsafe { AssignProcessToJobObject(job, proc_handle) };
        if assign_ok == 0 {
            let err = std::io::Error::last_os_error();
            unsafe { CloseHandle(job) };
            return Err(format!("AssignProcessToJobObject failed: {err}"));
        }

        Ok(Self { handle: job })
    }

    fn terminate_and_close(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if !self.handle.is_null() {
            unsafe {
                let _ = TerminateJobObject(self.handle, 1);
                let _ = CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[derive(Debug)]
pub struct LlamaServerHandle {
    child: Option<Child>,
    pid: u32,
    pub endpoint: ServerEndpoint,
    stderr_buf: Arc<Mutex<StderrRingBuffer>>,
    #[cfg(windows)]
    job: Option<WindowsJobGuard>,
}

impl LlamaServerHandle {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn try_check_alive(&mut self) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else {
            return Err("llama-server process handle is no longer active".to_string());
        };
        match child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => {
                let tail = self
                    .stderr_buf
                    .lock()
                    .map(|b| b.summary())
                    .unwrap_or_default();
                self.child = None;
                Err(format!(
                    "llama-server exited unexpectedly ({status}){}",
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(": {tail}")
                    }
                ))
            }
            Err(e) => Err(format!("Failed checking llama-server status: {e}")),
        }
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr_buf
            .lock()
            .map(|b| b.summary())
            .unwrap_or_default()
    }

    /// Synchronously terminate and reap ONLY this app-owned child process (and its session/job).
    pub fn reap_sync(&mut self) {
        let Some(mut child) = self.child.take() else {
            #[cfg(windows)]
            if let Some(mut job) = self.job.take() {
                job.terminate_and_close();
            }
            return;
        };

        if let Ok(Some(_)) = child.try_wait() {
            #[cfg(windows)]
            if let Some(mut job) = self.job.take() {
                job.terminate_and_close();
            }
            return;
        }

        #[cfg(unix)]
        {
            let pid = self.pid as libc::pid_t;
            // Process was spawned in its own session via setsid(); pgid == pid.
            let pg_rc = unsafe { libc::kill(-pid, libc::SIGTERM) };
            if pg_rc != 0 {
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }

            let deadline = Instant::now() + REAP_GRACE_PERIOD;
            let mut exited = false;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    exited = true;
                    break;
                }
                std::thread::sleep(REAP_POLL_STEP);
            }

            if !exited {
                let kill_pg = unsafe { libc::kill(-pid, libc::SIGKILL) };
                if kill_pg != 0 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        #[cfg(windows)]
        {
            if let Some(mut job) = self.job.take() {
                job.terminate_and_close();
            } else {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

impl Drop for LlamaServerHandle {
    fn drop(&mut self) {
        self.reap_sync();
    }
}

pub fn build_llama_server_args(
    model_path: &Path,
    port: u16,
    bearer_token: &str,
    model: &ManifestModelEntry,
    backend_mode: BackendMode,
    threads: u32,
) -> Vec<String> {
    let ngl = match backend_mode {
        BackendMode::Metal => "99",
        BackendMode::Cpu => "0",
    };
    let mut args = vec![
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--api-key".to_string(),
        bearer_token.to_string(),
        "--model".to_string(),
        model_path.to_string_lossy().into_owned(),
        "--ctx-size".to_string(),
        model.ctx_size.to_string(),
        "--parallel".to_string(),
        model.parallel_slots.max(1).to_string(),
        "--threads".to_string(),
        threads.max(1).to_string(),
        "--n-gpu-layers".to_string(),
        ngl.to_string(),
        "--no-webui".to_string(),
        "--cors-origins".to_string(),
        "localhost".to_string(),
        "--no-cors-credentials".to_string(),
    ];

    if model.disable_thinking {
        args.push("--reasoning".to_string());
        args.push("off".to_string());
        args.push("--reasoning-budget".to_string());
        args.push("0".to_string());
        args.push("--chat-template-kwargs".to_string());
        args.push("{\"enable_thinking\":false}".to_string());
    }

    args
}

pub fn spawn_llama_server_handle(
    runtime_dir: &Path,
    model_path: &Path,
    model: &ManifestModelEntry,
    backend_mode: BackendMode,
    port: u16,
) -> Result<LlamaServerHandle, String> {
    let bin_path = verify_runtime_layout(runtime_dir)?;
    let bearer_token = generate_session_token();

    let host_cpus = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let threads = model.recommended_threads.min(host_cpus).max(1);
    let args = build_llama_server_args(model_path, port, &bearer_token, model, backend_mode, threads);

    let mut cmd = Command::new(&bin_path);
    cmd.args(&args)
        .current_dir(runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(target_os = "macos")]
    {
        cmd.env("DYLD_LIBRARY_PATH", runtime_dir);
    }

    #[cfg(target_os = "linux")]
    {
        cmd.env("LD_LIBRARY_PATH", runtime_dir);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![runtime_dir.to_path_buf()];
        paths.extend(std::env::split_paths(&current_path));
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn llama-server '{}': {e}", bin_path.display()))?;
    let pid = child.id();

    #[cfg(windows)]
    let job = match WindowsJobGuard::create_and_assign(&child) {
        Ok(j) => Some(j),
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Failed to attach Windows Job Object to llama-server: {e}"
            ));
        }
    };

    let stderr_buf = Arc::new(Mutex::new(StderrRingBuffer::new()));
    if let Some(stderr) = child.stderr.take() {
        let buf_clone = Arc::clone(&stderr_buf);
        let token_clone = bearer_token.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(mut guard) = buf_clone.lock() {
                    guard.push_sanitized(&line, &token_clone);
                }
            }
        });
    }

    Ok(LlamaServerHandle {
        child: Some(child),
        pid,
        endpoint: ServerEndpoint {
            host: "127.0.0.1".to_string(),
            port,
            bearer_token,
            disable_thinking: model.disable_thinking,
            backend_mode,
            model_id: model.id.clone(),
        },
        stderr_buf,
        #[cfg(windows)]
        job,
    })
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

pub fn is_health_payload_ok(status_code: reqwest::StatusCode, body: &str) -> bool {
    if status_code != reqwest::StatusCode::OK {
        return false;
    }
    match serde_json::from_str::<HealthResponse>(body) {
        Ok(parsed) => parsed.status == "ok",
        Err(_) => false,
    }
}

pub async fn wait_until_server_ready(
    handle: &mut LlamaServerHandle,
    shutdown_flag: &AtomicBool,
    timeout: Duration,
) -> Result<(), String> {
    let client = build_loopback_http_client()?;
    let health_url = format!("{}/health", handle.endpoint.base_url());
    let auth_header = format!("Bearer {}", handle.endpoint.bearer_token);
    let deadline = Instant::now() + timeout;
    let notify = global_shutdown_notify();

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            return Err("Application shutdown requested during runtime startup".to_string());
        }

        handle.try_check_alive()?;

        let req_fut = client
            .get(&health_url)
            .header(reqwest::header::AUTHORIZATION, &auth_header)
            .timeout(HEALTH_REQUEST_TIMEOUT)
            .send();

        let send_res = tokio::select! {
            biased;
            _ = notify.notified() => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    return Err("Application shutdown requested during runtime startup".to_string());
                }
                continue;
            }
            res = req_fut => res,
        };

        if let Ok(resp) = send_res {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if is_health_payload_ok(code, &body) {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            let tail = handle.stderr_tail();
            return Err(format!(
                "llama-server failed to reach ready state within {}s{}",
                timeout.as_secs(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            ));
        }

        tokio::select! {
            biased;
            _ = notify.notified() => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    return Err("Application shutdown requested during runtime startup".to_string());
                }
            }
            _ = tokio::time::sleep(HEALTH_POLL_INTERVAL) => {}
        }
    }
}

/// Poll `/health` for a `LlamaServerHandle` already registered in `slot` without holding the
/// mutex across any `.await` point.
pub async fn wait_until_slot_ready(
    slot: &Arc<Mutex<Option<LlamaServerHandle>>>,
    endpoint: &ServerEndpoint,
    shutdown_flag: &AtomicBool,
    timeout: Duration,
) -> Result<(), String> {
    let client = build_loopback_http_client()?;
    let health_url = format!("{}/health", endpoint.base_url());
    let auth_header = format!("Bearer {}", endpoint.bearer_token);
    let deadline = Instant::now() + timeout;
    let notify = global_shutdown_notify();

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            return Err("Application shutdown requested during runtime startup".to_string());
        }

        {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            let Some(handle) = guard.as_mut() else {
                return Err("llama-server handle was reaped during runtime startup".to_string());
            };
            handle.try_check_alive()?;
        }

        let req_fut = client
            .get(&health_url)
            .header(reqwest::header::AUTHORIZATION, &auth_header)
            .timeout(HEALTH_REQUEST_TIMEOUT)
            .send();

        let send_res = tokio::select! {
            biased;
            _ = notify.notified() => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    return Err("Application shutdown requested during runtime startup".to_string());
                }
                continue;
            }
            res = req_fut => res,
        };

        if let Ok(resp) = send_res {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if is_health_payload_ok(code, &body) {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            let tail = slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|h| h.stderr_tail())
                .unwrap_or_default();
            return Err(format!(
                "llama-server failed to reach ready state within {}s{}",
                timeout.as_secs(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            ));
        }

        tokio::select! {
            biased;
            _ = notify.notified() => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    return Err("Application shutdown requested during runtime startup".to_string());
                }
            }
            _ = tokio::time::sleep(HEALTH_POLL_INTERVAL) => {}
        }
    }
}

/// Spawn `llama-server` on an ephemeral `127.0.0.1` port, register it into `global_runtime_slot()`
/// IMMEDIATELY before polling `/health`, and wait for strict `200 {"status":"ok"}` readiness.
pub async fn start_and_wait_into_global_slot(
    runtime_dir: &Path,
    model_path: &Path,
    model: &ManifestModelEntry,
    backend_mode: BackendMode,
    shutdown_flag: &AtomicBool,
) -> Result<ServerEndpoint, String> {
    let max_attempts = 3u8;
    let mut last_err = String::new();

    for attempt in 1..=max_attempts {
        if shutdown_flag.load(Ordering::SeqCst) {
            reap_global_runtime_sync();
            return Err("Application shutdown requested".to_string());
        }
        let port = pick_ephemeral_loopback_port()?;
        let handle =
            spawn_llama_server_handle(runtime_dir, model_path, model, backend_mode, port)?;
        let endpoint = handle.endpoint.clone();

        {
            let mut slot = global_runtime_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(mut stale) = slot.take() {
                drop(slot);
                stale.reap_sync();
                slot = global_runtime_slot()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
            }
            *slot = Some(handle);
        }

        if shutdown_flag.load(Ordering::SeqCst) {
            reap_global_runtime_sync();
            return Err("Application shutdown requested during runtime startup".to_string());
        }

        match wait_until_slot_ready(
            global_runtime_slot(),
            &endpoint,
            shutdown_flag,
            HEALTH_POLL_TIMEOUT,
        )
        .await
        {
            Ok(()) => return Ok(endpoint),
            Err(err) => {
                let tail = global_runtime_slot()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|h| h.stderr_tail())
                    .unwrap_or_default();
                reap_global_runtime_sync();
                let is_bind_race = tail.to_ascii_lowercase().contains("address already in use")
                    || err.to_ascii_lowercase().contains("address already in use");
                last_err = err;
                if is_bind_race && attempt < max_attempts {
                    continue;
                }
                return Err(last_err);
            }
        }
    }

    Err(last_err)
}

/// Spawn `llama-server` on an ephemeral `127.0.0.1` port with bind-race retries and wait for
/// strict `200 {"status":"ok"}` `/health` readiness (using `global_runtime_slot()` during startup).
pub async fn start_and_wait_llama_server(
    runtime_dir: &Path,
    model_path: &Path,
    model: &ManifestModelEntry,
    backend_mode: BackendMode,
    shutdown_flag: &AtomicBool,
) -> Result<LlamaServerHandle, String> {
    start_and_wait_into_global_slot(runtime_dir, model_path, model, backend_mode, shutdown_flag)
        .await?;
    let handle = global_runtime_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| "llama-server handle was reaped during runtime startup".to_string())?;
    Ok(handle)
}

/// Reap any currently tracked global `llama-server` synchronously (used on shutdown, window close,
/// panic hook, and before spawning a replacement server).
pub fn reap_global_runtime_sync() {
    let mut guard = global_runtime_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(mut handle) = guard.take() {
        drop(guard);
        handle.reap_sync();
    }
}

/// Set global shutdown flag, wake any in-flight `/health` pollers, and reap any tracked runtime.
pub fn request_global_shutdown_and_reap() {
    global_shutdown_flag().store(true, Ordering::SeqCst);
    global_shutdown_notify().notify_waiters();
    reap_global_runtime_sync();
}

pub fn install_panic_reap_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        request_global_shutdown_and_reap();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn detect_ram_returns_positive_on_supported_host() {
        let ram = detect_physical_ram_bytes();
        assert!(ram >= 512 * 1024 * 1024, "unexpected RAM bytes: {ram}");
    }

    #[test]
    fn backend_mode_respects_host_and_explicit_cpu_override() {
        assert_eq!(select_backend_mode(Some(false)).unwrap(), BackendMode::Cpu);
        if supports_metal_acceleration() {
            assert_eq!(select_backend_mode(None).unwrap(), BackendMode::Metal);
            assert_eq!(select_backend_mode(Some(true)).unwrap(), BackendMode::Metal);
        } else {
            assert_eq!(select_backend_mode(None).unwrap(), BackendMode::Cpu);
            assert!(select_backend_mode(Some(true)).is_err());
        }
    }

    #[test]
    fn health_payload_requires_200_and_status_ok() {
        assert!(is_health_payload_ok(
            reqwest::StatusCode::OK,
            r#"{"status":"ok"}"#
        ));
        assert!(!is_health_payload_ok(
            reqwest::StatusCode::OK,
            r#"{"status":"loading model"}"#
        ));
        assert!(!is_health_payload_ok(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"status":"ok"}"#
        ));
        assert!(!is_health_payload_ok(
            reqwest::StatusCode::OK,
            "not json"
        ));
    }

    #[test]
    fn cli_args_include_loopback_auth_cors_and_anti_thinking() {
        let model = ManifestModelEntry {
            id: "qwen3".to_string(),
            name: "Qwen3".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 100,
            sha256: "a".repeat(64),
            license: "Apache-2.0".to_string(),
            license_url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
            is_default: true,
            is_installed: true,
            is_bundled: true,
            min_recommended_ram_gb: 8,
            non_commercial_warning: false,
            download_url: "https://example.invalid/m.gguf".to_string(),
            filename: "m.gguf".to_string(),
            repo_id: String::new(),
            family: "qwen3".to_string(),
            tier: "default".to_string(),
            disable_thinking: true,
            ctx_size: 4096,
            parallel_slots: 1,
            recommended_threads: 4,
            bundled_notice_files: vec![],
            notes: String::new(),
        };
        let args = build_llama_server_args(
            Path::new("/tmp/m.gguf"),
            18080,
            "secret-token",
            &model,
            BackendMode::Metal,
            4,
        );
        assert!(args.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(args.windows(2).any(|w| w == ["--api-key", "secret-token"]));
        assert!(args.contains(&"--no-webui".to_string()));
        assert!(args.windows(2).any(|w| w == ["--cors-origins", "localhost"]));
        assert!(args.contains(&"--no-cors-credentials".to_string()));
        assert!(args.windows(2).any(|w| w == ["--n-gpu-layers", "99"]));
        assert!(args.windows(2).any(|w| w == ["--reasoning", "off"]));
        assert!(args.windows(2).any(|w| w == ["--reasoning-budget", "0"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--chat-template-kwargs", "{\"enable_thinking\":false}"]));
    }

    #[test]
    fn stderr_ring_buffer_redacts_bearer_token() {
        let mut buf = StderrRingBuffer::new();
        buf.push_sanitized("started with key deadbeef1234", "deadbeef1234");
        let s = buf.summary();
        assert!(!s.contains("deadbeef1234"));
        assert!(s.contains("[REDACTED]"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_reaper_terminates_owned_process_without_affecting_decoy() {
        use std::os::unix::process::CommandExt;

        let mut decoy = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn decoy");
        let decoy_pid = decoy.id();

        let mut owned_cmd = Command::new("sleep");
        owned_cmd.arg("30").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe {
            owned_cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let owned = owned_cmd.spawn().expect("spawn owned");
        let owned_pid = owned.id();

        let mut handle = LlamaServerHandle {
            child: Some(owned),
            pid: owned_pid,
            endpoint: ServerEndpoint {
                host: "127.0.0.1".to_string(),
                port: 0,
                bearer_token: "t".to_string(),
                disable_thinking: false,
                backend_mode: BackendMode::Cpu,
                model_id: "test".to_string(),
            },
            stderr_buf: Arc::new(Mutex::new(StderrRingBuffer::new())),
        };

        handle.reap_sync();

        // Owned process must be completely reaped (kill(pid, 0) returns ESRCH)
        let rc = unsafe { libc::kill(owned_pid as libc::pid_t, 0) };
        assert_eq!(rc, -1);

        // Decoy process must still be alive
        assert!(decoy.try_wait().unwrap().is_none());
        assert_eq!(unsafe { libc::kill(decoy_pid as libc::pid_t, 0) }, 0);
        let _ = decoy.kill();
        let _ = decoy.wait();
    }

    #[tokio::test]
    async fn wait_until_ready_rejects_loading_then_accepts_ok() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "tok-123".to_string();
        let tok_srv = token.clone();
        let srv = tokio::spawn(async move {
            let mut hits = 0u32;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                hits += 1;
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let has_auth = req.contains(&format!("Bearer {tok_srv}"));
                let (code, body) = if !has_auth {
                    ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
                } else if hits == 1 {
                    ("200 OK", r#"{"status":"loading model"}"#)
                } else {
                    ("200 OK", r#"{"status":"ok"}"#)
                };
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let dummy_child = spawn_long_lived_test_child();
        let pid = dummy_child.id();
        let mut handle = LlamaServerHandle {
            child: Some(dummy_child),
            pid,
            endpoint: ServerEndpoint {
                host: "127.0.0.1".to_string(),
                port,
                bearer_token: token,
                disable_thinking: false,
                backend_mode: BackendMode::Cpu,
                model_id: "test".to_string(),
            },
            stderr_buf: Arc::new(Mutex::new(StderrRingBuffer::new())),
            #[cfg(windows)]
            job: None,
        };
        let flag = AtomicBool::new(false);
        wait_until_server_ready(&mut handle, &flag, Duration::from_secs(5))
            .await
            .expect("should become ready on 2nd poll");
        handle.reap_sync();
        srv.abort();
    }

    fn spawn_long_lived_test_child() -> Child {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let mut cmd = Command::new("sleep");
            cmd.arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            cmd.spawn().expect("spawn unix sleep test child")
        }
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "ping -n 31 127.0.0.1 >NUL"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd.spawn().expect("spawn windows ping test child")
        }
    }

    #[tokio::test]
    async fn startup_reap_during_health_polling_terminates_child_promptly() {
        static SLOT_TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _guard = SLOT_TEST_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;

        global_shutdown_flag().store(false, Ordering::SeqCst);
        reap_global_runtime_sync();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"status":"loading model"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let child = spawn_long_lived_test_child();
        let pid = child.id();
        let endpoint = ServerEndpoint {
            host: "127.0.0.1".to_string(),
            port,
            bearer_token: "tok-reap".to_string(),
            disable_thinking: false,
            backend_mode: BackendMode::Cpu,
            model_id: "test".to_string(),
        };
        let handle = LlamaServerHandle {
            child: Some(child),
            pid,
            endpoint: endpoint.clone(),
            stderr_buf: Arc::new(Mutex::new(StderrRingBuffer::new())),
            #[cfg(windows)]
            job: None,
        };

        {
            let mut slot = global_runtime_slot().lock().unwrap();
            *slot = Some(handle);
        }

        let started = Instant::now();
        let poller = tokio::spawn(async move {
            wait_until_slot_ready(
                global_runtime_slot(),
                &endpoint,
                global_shutdown_flag(),
                Duration::from_secs(30),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(120)).await;
        request_global_shutdown_and_reap();

        let res = poller.await.expect("poller join");
        assert!(res.is_err(), "poller must abort on shutdown/reap");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "startup reap took too long: {:?}",
            started.elapsed()
        );
        assert!(
            global_runtime_slot().lock().unwrap().is_none(),
            "global slot must be empty after reap"
        );
        #[cfg(unix)]
        {
            let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
            assert_eq!(rc, -1, "setsid child PID {pid} must be reaped");
        }

        global_shutdown_flag().store(false, Ordering::SeqCst);
        srv.abort();
    }
}
