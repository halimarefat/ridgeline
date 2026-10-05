//! Settings, secrets, privacy and data management, diagnostics, about.

use crate::app::{pstr, R};
use crate::settings::Settings;
use crate::App;
use rl_json::{FromJson, ToJson, Value};

pub const SECRET_NAMES: &[&str] = &["ai_api_key"];

impl App {
    pub(crate) fn get_settings(&mut self, _p: &Value) -> R {
        let has_key = self.platform.secret_get("ai_api_key").ok().flatten().map(|k| !k.is_empty()).unwrap_or(false);
        Ok(Value::obj([
            ("settings", self.settings.to_json()),
            ("secure_store", self.platform.has_secure_store().into()),
            ("has_ai_key", has_key.into()),
            ("ai_summary_preview", self.summary_preview()),
            ("export_dir", self.cfg.export_dir.display().to_string().into()),
        ]))
    }

    pub(crate) fn save_settings(&mut self, p: &Value) -> R {
        let s = Settings::from_json(p.req("settings")?)?;
        s.validate()?;
        if self.ride_active() && s.trainer != self.settings.trainer {
            return Err("Trainer settings can't change during a ride.".into());
        }
        let demo_changed = s.demo_mode != self.settings.demo_mode;
        self.settings = s;
        self.store.write_value("settings.json", &self.settings.to_json())?;
        self.apply_trainer_settings();
        if demo_changed {
            let on = self.settings.demo_mode;
            self.enable_demo(on);
        }
        Ok(self.settings.to_json())
    }

    pub(crate) fn set_secret(&mut self, p: &Value) -> R {
        let name = pstr(p, "name", 40)?;
        if !SECRET_NAMES.contains(&name) {
            return Err("Unknown secret.".into());
        }
        let value = pstr(p, "value", 400)?.trim();
        if value.is_empty() {
            return Err("Enter a value.".into());
        }
        if !self.platform.has_secure_store() {
            return Err("No operating-system credential store is available, so keys can't be stored safely in this build.".into());
        }
        self.platform.secret_set(name, Some(value))?;
        Ok(Value::Null)
    }

    pub(crate) fn clear_secret(&mut self, p: &Value) -> R {
        let name = pstr(p, "name", 40)?;
        if !SECRET_NAMES.contains(&name) {
            return Err("Unknown secret.".into());
        }
        self.platform.secret_set(name, None)?;
        Ok(Value::Null)
    }

    pub(crate) fn storage_info(&mut self, _p: &Value) -> R {
        let mut v = self.store.usage();
        v.set("data_dir", self.cfg.data_dir.display().to_string());
        v.set("encrypted", false);
        v.set("note", "Ridgeline's local files are not encrypted; they are protected by your operating-system account. Deleting data here cannot remove anything already sent to an AI service you configured.");
        Ok(v)
    }

    pub(crate) fn delete_all_data(&mut self, p: &Value) -> R {
        if pstr(p, "confirm", 20)? != "DELETE" {
            return Err("Type DELETE to confirm.".into());
        }
        if self.ride_active() {
            return Err("Finish the ride first.".into());
        }
        self.session = None;
        self.session_meta = None;
        self.store.delete_all()?;
        let _ = self.platform.secret_set("ai_api_key", None);
        self.settings = Settings::default();
        self.completions = Value::empty_obj();
        self.route_cache.clear();
        self.reload_library();
        crate::bundled::ensure_bundled_routes(&self.store)?;
        Ok(Value::Null)
    }

    pub(crate) fn export_all_data(&mut self, _p: &Value) -> R {
        let date = self.today().to_string();
        let mut dest = self.cfg.export_dir.join(format!("ridgeline-data-{date}"));
        let mut n = 1;
        while dest.exists() && n < 100 {
            dest = self.cfg.export_dir.join(format!("ridgeline-data-{date}-{n}"));
            n += 1;
        }
        let bytes = self.store.export_all(&dest)?;
        Ok(Value::obj([("path", dest.display().to_string().into()), ("bytes", bytes.into())]))
    }

    /// Opt-in diagnostics bundle: versions, redacted settings, device
    /// capabilities and the control log. No keys, profile details, routes,
    /// locations or coach conversations.
    pub(crate) fn export_diagnostics(&mut self, _p: &Value) -> R {
        if !self.settings.diagnostics_opt_in {
            return Err("Turn on 'Allow diagnostic export' in Settings → Privacy first.".into());
        }
        let now = self.now();
        let devices: Vec<Value> = self
            .dm
            .devices
            .values()
            .map(|d| {
                Value::obj([
                    ("adapter", d.adapter_kind.into()),
                    ("state", d.state.to_json()),
                    ("roles", Value::Arr(d.roles().into_iter().map(Value::from).collect())),
                    ("manufacturer", d.info.manufacturer.clone().into()),
                    ("model", d.info.model.clone().into()),
                    ("firmware", d.info.firmware.clone().into()),
                    ("caps", d.caps().map(|c| c.to_json()).unwrap_or(Value::Null)),
                    ("parse_errors", d.parse_errors.into()),
                    ("notifications", d.notifications.into()),
                ])
            })
            .collect();
        let mut s = self.settings.to_json();
        s.set("ai", Value::obj([("provider", self.settings.ai.provider.clone().into()), ("preset", self.settings.ai.preset.clone().into()), ("model", self.settings.ai.model.clone().into())]));
        let v = Value::obj([
            ("app_version", self.cfg.app_version.clone().into()),
            ("platform", self.cfg.platform_name.clone().into()),
            ("policy_version", rl_domain::policy::POLICY_VERSION.into()),
            ("profile_version", rl_domain::route::PROFILE_VERSION.into()),
            ("uptime_ms", now.into()),
            ("settings", s),
            ("adapters", self.dm.to_json(now, false).get("adapters").cloned().unwrap_or(Value::Null)),
            ("devices", Value::Arr(devices)),
            ("control", self.dm.controller.to_json()),
            ("control_log", self.dm.controller.log_json(300)),
            ("activities", self.store.list_activities().map(|l| l.len()).unwrap_or(0).into()),
        ]);
        let path = self.write_export(&format!("ridgeline-diagnostics-{}.json", self.today()), v.to_string_pretty().as_bytes())?;
        Ok(Value::obj([("path", path.into())]))
    }

    pub(crate) fn get_about(&mut self, _p: &Value) -> R {
        Ok(Value::obj([
            ("version", self.cfg.app_version.clone().into()),
            ("platform", self.cfg.platform_name.clone().into()),
            ("policy_version", rl_domain::policy::POLICY_VERSION.into()),
            (
                "attributions",
                Value::Arr(
                    [
                        "Map tiles: OpenFreeMap © OpenMapTiles, data © OpenStreetMap contributors (ODbL)",
                        "Map rendering: MapLibre GL JS (BSD-3-Clause)",
                        "Routing: Valhalla (MIT) public server by FOSSGIS e.V.; data © OpenStreetMap contributors",
                        "Elevation: Open-Meteo.com (CC BY 4.0), Copernicus DEM GLO-90 © DLR/Airbus, provided under COPERNICUS by the EU and ESA",
                        "Desktop shell: Tauri (MIT/Apache-2.0); Bluetooth: btleplug (MIT/Apache-2.0/BSD-3-Clause)",
                        "FIT protocol: Garmin FIT SDK documentation (format reference)",
                    ]
                    .iter()
                    .map(|s| Value::from(*s))
                    .collect(),
                ),
            ),
            ("updates", "Ridgeline does not update itself. New versions are published on the project's GitHub Releases page; install them when you're not riding.".into()),
        ]))
    }
}
