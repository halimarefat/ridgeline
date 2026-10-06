// Release builds on Windows: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod http;
mod platform;

use rl_app::{Config, Runtime};
use rl_device::adapter::DeviceAdapter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

/// Installed-app smoke test (used by CI after installing the package):
/// `RIDGELINE_SMOKE_TEST=<report.json>` makes the app use a throwaway data
/// folder, wait until the bundled UI is running and polling the native
/// service over IPC, open demo mode, confirm simulated devices become ready,
/// write a JSON report and exit. Never set in normal use.
const SMOKE_ENV: &str = "RIDGELINE_SMOKE_TEST";
static SMOKE_STARTED: AtomicBool = AtomicBool::new(false);

fn smoke_report_path() -> Option<PathBuf> {
    std::env::var_os(SMOKE_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn run_smoke(app: tauri::AppHandle, report: PathBuf) {
    std::thread::spawn(move || {
        let call = |m: &str, p: &str| -> String {
            let st = app.state::<AppState>();
            let out = match st.rt.lock() {
                Ok(g) => g.as_ref().map(|r| r.rpc(m, p)).unwrap_or_default(),
                Err(_) => String::new(),
            };
            out
        };
        let boot = call("getBootstrap", "{}");
        let demo = call("startDemo", "{}");
        let mut ready: Vec<String> = Vec::new();
        let mut adapters = rl_json::Value::Null;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(250));
            let st = rl_json::parse(&call("getState", "{}")).unwrap_or(rl_json::Value::Null);
            let devs = st.get("result").and_then(|r| r.get("devices"));
            adapters = devs.and_then(|d| d.get("adapters")).cloned().unwrap_or(rl_json::Value::Null);
            ready = devs
                .and_then(|d| d.get("devices"))
                .and_then(|d| d.as_arr())
                .map(|a| {
                    a.iter()
                        .filter(|d| d.get("simulated").and_then(|x| x.as_bool()) == Some(true) && d.get("state").and_then(|x| x.as_str()) == Some("ready"))
                        .filter_map(|d| d.get("name").and_then(|x| x.as_str()).map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if ready.len() >= 3 {
                break;
            }
        }
        let boot_v = rl_json::parse(&boot).unwrap_or(rl_json::Value::Null);
        let ok = boot_v.get("ok").and_then(|x| x.as_bool()) == Some(true) && rl_json::parse(&demo).ok().and_then(|v| v.get("ok").and_then(|x| x.as_bool())) == Some(true) && ready.len() >= 3;
        let v = rl_json::Value::obj([
            ("ok", ok.into()),
            ("ui_ipc", true.into()),
            ("version", env!("CARGO_PKG_VERSION").into()),
            ("platform", format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH).into()),
            ("demo_devices_ready", rl_json::Value::Arr(ready.into_iter().map(rl_json::Value::from).collect())),
            ("adapters", adapters),
        ]);
        let _ = std::fs::write(&report, v.to_string_pretty());
        app.exit(if ok { 0 } else { 3 });
    });
}

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
async fn rpc(app: tauri::AppHandle, state: tauri::State<'_, AppState>, method: String, params: String) -> Result<String, ()> {
    if method.len() > 64 {
        return Ok(error_json("Invalid command."));
    }
    let out = {
        let guard = match state.rt.lock() {
            Ok(g) => g,
            Err(_) => return Ok(error_json("Internal error. Please restart Ridgeline.")),
        };
        match guard.as_ref() {
            Some(rt) => rt.rpc(&method, &params),
            None => error_json(state.startup_error.as_deref().unwrap_or("Ridgeline failed to start.")),
        }
    };
    // The UI polls getState once it has mounted: proof that the bundled UI
    // loaded and can reach the native service.
    if method == "getState" {
        if let Some(report) = smoke_report_path() {
            if !SMOKE_STARTED.swap(true, Ordering::SeqCst) {
                run_smoke(app, report);
            }
        }
    }
    Ok(out)
}

/// Called by the UI after the rider confirmed quitting (ride saved first).
#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let (data_dir, export_dir) = match smoke_report_path() {
                // Smoke test: never touch the rider's real data.
                Some(r) => {
                    let d = PathBuf::from(format!("{}.data", r.display()));
                    (d.clone(), d.join("exports"))
                }
                None => {
                    let d = app.path().app_data_dir()?;
                    let e = app.path().download_dir().map(|x| x.join("Ridgeline")).unwrap_or_else(|_| d.join("exports"));
                    (d, e)
                }
            };
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
