use tauri::Manager;

pub mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
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
        .run(tauri::generate_context!())
        .expect("error while running Local Humanizer application");
}
