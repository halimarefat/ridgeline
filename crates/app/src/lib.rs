//! Ridgeline native application service.
//!
//! The UI talks to this crate only through a narrow JSON command interface
//! (`Runtime::rpc`). A dedicated thread ticks devices, the session
//! coordinator and recording every 50 ms on a monotonic clock, independent
//! of UI rendering, window focus or network state. AI and map-data requests
//! run as background jobs and never block the control loop.

pub mod app;
pub mod bundled;
mod h_coach;
mod h_data;
mod h_devices;
mod h_history;
mod h_profile;
mod h_routes;
mod h_session;
mod h_workouts;
pub mod jobs;
pub mod settings;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub use app::App;
use rl_device::adapter::DeviceAdapter;
use rl_net::http::HttpClient;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TICK_MS: u64 = 50;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub export_dir: PathBuf,
    pub platform_name: String,
    pub app_version: String,
}

/// Host services supplied by the desktop shell (or the dev server).
pub trait Platform: Send + Sync {
    /// Prevent system sleep while a ride is active (best effort).
    fn keep_awake(&self, _on: bool) {}
    fn secret_get(&self, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn secret_set(&self, _name: &str, _value: Option<&str>) -> Result<(), String> {
        Err("No credential store is available in this build.".into())
    }
    fn has_secure_store(&self) -> bool {
        false
    }
}

/// Platform without OS integration (tests, dev server).
pub struct NullPlatform;
impl Platform for NullPlatform {}

pub struct Runtime {
    app: Arc<Mutex<App>>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
    _lock: Option<std::fs::File>,
}

impl Runtime {
    pub fn start(cfg: Config, adapters: Vec<Box<dyn DeviceAdapter>>, http: Arc<dyn HttpClient>, platform: Arc<dyn Platform>) -> Result<Runtime, String> {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| format!("Could not create the data folder: {e}"))?;
        // Single active instance: an exclusive lock held for the process lifetime.
        let lock_path = cfg.data_dir.join("ridgeline.lock");
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path).map_err(|e| format!("Could not open the lock file: {e}"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err("Ridgeline is already running. Only one window can control the trainer at a time.".into()),
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("Could not lock the data folder: {e}")),
        }
        let app = App::new(cfg, adapters, http, platform)?;
        let app = Arc::new(Mutex::new(app));
        app.lock().unwrap().self_ref = Some(Arc::downgrade(&app));
        let stop = Arc::new(AtomicBool::new(false));
        let (a2, s2) = (app.clone(), stop.clone());
        let ticker = std::thread::Builder::new()
            .name("ridgeline-tick".into())
            .spawn(move || {
                while !s2.load(Ordering::Relaxed) {
                    if let Ok(mut a) = a2.lock() {
                        a.tick();
                    }
                    std::thread::sleep(Duration::from_millis(TICK_MS));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Runtime { app, stop, ticker: Some(ticker), _lock: Some(lock) })
    }

    /// Execute one command. Returns `{"ok":true,"result":…}` or
    /// `{"ok":false,"error":"…"}` as JSON text.
    pub fn rpc(&self, method: &str, params_json: &str) -> String {
        let reply = match self.app.lock() {
            Ok(mut a) => a.rpc(method, params_json),
            Err(_) => Err("Internal error (state lock poisoned). Please restart Ridgeline.".into()),
        };
        app::reply_json(reply)
    }

    /// True while a ride is in progress (used to guard window close / updates).
    pub fn ride_active(&self) -> bool {
        self.app.lock().map(|a| a.ride_active()).unwrap_or(false)
    }

    /// Flush and stop. A ride still in progress is stopped and saved first.
    pub fn shutdown(&mut self) {
        if let Ok(mut a) = self.app.lock() {
            a.shutdown();
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
    }

    pub fn app(&self) -> Arc<Mutex<App>> {
        self.app.clone()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
