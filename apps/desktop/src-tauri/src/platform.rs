//! OS integration: credential store (keyring), keep-awake during rides.

use rl_app::Platform;
use std::sync::Mutex;

const SERVICE: &str = "io.github.halimarefat.ridgeline";

pub struct DesktopPlatform {
    awake: Mutex<Option<KeepAwake>>,
}

impl DesktopPlatform {
    pub fn new() -> Self {
        DesktopPlatform { awake: Mutex::new(None) }
    }
}

impl Platform for DesktopPlatform {
    fn keep_awake(&self, on: bool) {
        if let Ok(mut g) = self.awake.lock() {
            if on && g.is_none() {
                *g = KeepAwake::start();
            } else if !on {
                *g = None; // Drop releases the request.
            }
        }
    }
    fn has_secure_store(&self) -> bool {
        true
    }
    fn secret_get(&self, name: &str) -> Result<Option<String>, String> {
        let e = keyring::Entry::new(SERVICE, name).map_err(|e| e.to_string())?;
        match e.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Credential store error: {e}")),
        }
    }
    fn secret_set(&self, name: &str, value: Option<&str>) -> Result<(), String> {
        let e = keyring::Entry::new(SERVICE, name).map_err(|e| e.to_string())?;
        match value {
            Some(v) => e.set_password(v).map_err(|e| format!("Credential store error: {e}")),
            None => match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("Credential store error: {e}")),
            },
        }
    }
}

// ---------------------------------------------------------------- keep awake

#[cfg(windows)]
mod win {
    #[link(name = "kernel32")]
    extern "system" {
        pub fn SetThreadExecutionState(es_flags: u32) -> u32;
    }
    pub const ES_CONTINUOUS: u32 = 0x8000_0000;
    pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    pub const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;
}

pub struct KeepAwake {
    #[cfg(windows)]
    stop: Option<std::sync::mpsc::Sender<()>>,
    #[cfg(not(windows))]
    child: Option<std::process::Child>,
}

impl KeepAwake {
    #[cfg(windows)]
    fn start() -> Option<KeepAwake> {
        // The execution state belongs to the calling thread, so a dedicated
        // thread holds it for the duration of the ride.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || unsafe {
                win::SetThreadExecutionState(win::ES_CONTINUOUS | win::ES_SYSTEM_REQUIRED | win::ES_DISPLAY_REQUIRED);
                let _ = rx.recv();
                win::SetThreadExecutionState(win::ES_CONTINUOUS);
            })
            .ok()?;
        Some(KeepAwake { stop: Some(tx) })
    }

    #[cfg(target_os = "macos")]
    fn start() -> Option<KeepAwake> {
        let pid = std::process::id().to_string();
        let child = std::process::Command::new("/usr/bin/caffeinate").args(["-di", "-w", &pid]).spawn().ok();
        Some(KeepAwake { child })
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn start() -> Option<KeepAwake> {
        let child = std::process::Command::new("systemd-inhibit")
            .args(["--what=idle:sleep", "--who=Ridgeline", "--why=Ride in progress", "--mode=block", "sleep", "infinity"])
            .spawn()
            .ok();
        Some(KeepAwake { child })
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        #[cfg(not(windows))]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
