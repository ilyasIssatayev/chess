use chess_camera_recorder::server::{ServerConfig, ServerHandle};
use tauri::Manager;
struct Recorder(std::sync::Mutex<Option<ServerHandle>>);
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let resources = app.path().resource_dir()?;
            let config = if cfg!(debug_assertions) {
                ServerConfig::development()
            } else {
                ServerConfig {
                    ui: resources.join("ui"),
                    assets: resources.join("vision"),
                    camera_helper: resources.join("chess-camera"),
                    data: std::env::var_os("CHESS_RECORDER_DATA_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or(app.path().app_data_dir()?),
                }
            };
            let server = chess_camera_recorder::server::spawn(config, 0)?;
            let url = format!("http://127.0.0.1:{}/?native=1", server.port).parse()?;
            app.manage(Recorder(std::sync::Mutex::new(Some(server))));
            let window =
                tauri::WebviewWindowBuilder::new(app, "recorder", tauri::WebviewUrl::External(url))
                    .title("Chess Camera Recorder")
                    .inner_size(1360.0, 920.0)
                    .min_inner_size(900.0, 650.0)
                    .build()?;
            window.show()?;
            eprintln!("Recorder window created; visible={:?}", window.is_visible());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Desktop setup failed")
        .run(|app, event| {
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
                && label == "recorder"
            {
                app.exit(0);
            }
            #[cfg(target_os = "macos")]
            if matches!(
                event,
                tauri::RunEvent::Ready | tauri::RunEvent::Reopen { .. }
            ) && let Some(window) = app.get_webview_window("recorder")
            {
                let _ = app.show();
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(state) = app.try_state::<Recorder>()
            {
                state.0.lock().unwrap().take();
            }
        });
}
