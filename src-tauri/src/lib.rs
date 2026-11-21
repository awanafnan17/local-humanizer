use tauri::{Manager, RunEvent, WindowEvent};

pub mod commands;
pub mod engine;
pub mod models;
pub mod runtime;

use commands::AppState;
use runtime::{install_panic_reap_hook, request_global_shutdown_and_reap};

fn cleanup_on_exit(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<AppState>() {
        state.download_manager.cancel_all();
        state.engine.cancel_all_active();
        if let Ok(mut ep) = state.active_endpoint.lock() {
            *ep = None;
        }
    }
    request_global_shutdown_and_reap();
}

#[cfg(unix)]
fn install_unix_signal_reap_handler(app_handle: tauri::AppHandle) {
    use std::sync::atomic::{AtomicI32, Ordering};
    static SIGNAL_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn handle_signal(_sig: libc::c_int) {
        let fd = SIGNAL_WRITE_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            let byte: u8 = 1;
            unsafe {
                let _ = libc::write(fd, (&raw const byte).cast::<libc::c_void>(), 1);
            }
        }
    }

    let mut fds = [-1i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let read_fd = fds[0];
    let write_fd = fds[1];
    unsafe {
        let flags = libc::fcntl(write_fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(write_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        let _ = libc::fcntl(read_fd, libc::F_SETFD, libc::FD_CLOEXEC);
        let _ = libc::fcntl(write_fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    SIGNAL_WRITE_FD.store(write_fd, Ordering::SeqCst);

    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_signal as *const () as usize;
        libc::sigemptyset(&raw mut sa.sa_mask);
        sa.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGTERM, &raw const sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &raw const sa, std::ptr::null_mut());
    }

    let _ = std::thread::Builder::new()
        .name("lh-signal-reaper".to_string())
        .spawn(move || {
            let mut buf = [0u8; 16];
            loop {
                let n = unsafe {
                    libc::read(read_fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
                };
                if n > 0 {
                    cleanup_on_exit(&app_handle);
                    unsafe {
                        let _ = libc::close(read_fd);
                    }
                    app_handle.exit(0);
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    std::process::exit(0);
                }
                if n == 0 {
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
        });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_reap_hook();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            #[cfg(unix)]
            install_unix_signal_reap_handler(app.handle().clone());

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();
                let _ = commands::initialize_or_switch_runtime(&app_handle, &state, None, None).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
            ) {
                cleanup_on_exit(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_status,
            commands::list_models,
            commands::select_model,
            commands::download_model,
            commands::cancel_download,
            commands::humanize_document,
            commands::cancel_humanize,
            commands::retry_chunk,
            commands::import_text_file,
            commands::save_output_file,
            commands::retry_runtime_init,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Local Humanizer application");

    app.run(|app_handle, event| match event {
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            cleanup_on_exit(app_handle);
        }
        _ => {}
    });
}
