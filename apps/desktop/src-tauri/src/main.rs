// Release builds on Windows: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod http;
mod platform;

use rl_app::{Config, Runtime};
use rl_device::adapter::DeviceAdapter;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

struct AppState {
    rt: Mutex<Option<Runtime>>,
    startup_error: Option<String>,
}

fn error_json(msg: &str) -> String {
    rl_json::Value::obj([("ok", false.into()), ("error", msg.into())]).to_string_compact()
}

/// The only command exposed to web content: a narrow, validated JSON
/// command interface. The UI never writes GATT bytes or storage directly.
#[tauri::command]
async fn rpc(state: tauri::State<'_, AppState>, method: String, params: String) -> Result<String, ()> {
    if method.len() > 64 {
        return Ok(error_json("Invalid command."));
    }
    let guard = match state.rt.lock() {
        Ok(g) => g,
        Err(_) => return Ok(error_json("Internal error. Please restart Ridgeline.")),
    };
    Ok(match guard.as_ref() {
        Some(rt) => rt.rpc(&method, &params),
        None => error_json(state.startup_error.as_deref().unwrap_or("Ridgeline failed to start.")),
    })
}

/// Called by the UI after the rider confirmed quitting (ride saved first).
#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let export_dir = app.path().download_dir().map(|d| d.join("Ridgeline")).unwrap_or_else(|_| data_dir.join("exports"));
            let cfg = Config {
                data_dir,
                export_dir,
                platform_name: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            let adapters: Vec<Box<dyn DeviceAdapter>> = vec![Box::new(ble::BleAdapter::start())];
            let (rt, err) = match Runtime::start(cfg, adapters, Arc::new(http::UreqClient::new()), Arc::new(platform::DesktopPlatform::new())) {
                Ok(r) => (Some(r), None),
                Err(e) => (None, Some(e)),
            };
            app.manage(AppState { rt: Mutex::new(rt), startup_error: err });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let active = state.rt.lock().ok().and_then(|g| g.as_ref().map(|r| r.ride_active())).unwrap_or(false);
                if active {
                    // Never close silently mid-ride: the UI asks the rider to
                    // stop and save first (A17).
                    api.prevent_close();
                    let _ = window.emit("ridgeline://close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![rpc, exit_app])
        .build(tauri::generate_context!())
        .expect("error while building Ridgeline");
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let state = handle.state::<AppState>();
            let rt = state.rt.lock().ok().and_then(|mut g| g.take());
            if let Some(mut rt) = rt {
                rt.shutdown();
            }
        }
    });
}
