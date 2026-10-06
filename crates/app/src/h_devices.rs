//! Device commands: scan, connect, roles/sources, control, simulator.

use crate::app::{pstr, R};
use crate::App;
use rl_device::telemetry::{Metric, Service, SourceKey};
use rl_json::Value;

impl App {
    pub(crate) fn scan_devices(&mut self, p: &Value) -> R {
        let now = self.now();
        let include_sim = p.bool_or("include_simulator", self.settings.demo_mode);
        let idxs: Vec<usize> = (0..self.dm.adapters.len()).filter(|i| include_sim || Some(*i) != self.sim_index).collect();
        if idxs.is_empty() {
            return Err("No Bluetooth adapter is available in this build. Use demo mode to try simulated devices.".into());
        }
        let mut errs = Vec::new();
        let mut any = false;
        for i in idxs {
            match self.dm.start_scan(Some(i), now) {
                Ok(()) => any = true,
                Err(e) => errs.push(e),
            }
        }
        if any {
            Ok(Value::obj([("scanning", true.into()), ("warnings", Value::Arr(errs.into_iter().map(Value::from).collect()))]))
        } else {
            Err(errs.join(" "))
        }
    }

    pub(crate) fn connect_device(&mut self, p: &Value) -> R {
        let key = pstr(p, "key", 200)?.to_string();
        let now = self.now();
        self.dm.connect(&key, now)?;
        self.dm.telemetry.set_stale_ms(&key, self.settings.trainer.stale_ms);
        self.remember_device(&key);
        Ok(Value::Null)
    }

    pub(crate) fn disconnect_device(&mut self, p: &Value) -> R {
        let key = pstr(p, "key", 200)?;
        if self.ride_active() && self.dm.trainer_key.as_deref() == Some(key) {
            return Err("Stop the ride before disconnecting the trainer.".into());
        }
        let now = self.now();
        self.dm.disconnect(key, now);
        Ok(Value::Null)
    }

    pub(crate) fn forget_device(&mut self, p: &Value) -> R {
        let key = pstr(p, "key", 200)?.to_string();
        if self.ride_active() {
            return Err("Stop the ride before forgetting devices.".into());
        }
        let now = self.now();
        self.dm.forget(&key, now);
        let mut prefs = self.store.read_value("devices.json")?.unwrap_or_else(|| Value::Arr(vec![]));
        if let Value::Arr(a) = &mut prefs {
            a.retain(|d| d.get("key").and_then(|k| k.as_str()) != Some(key.as_str()));
        }
        self.store.write_value("devices.json", &prefs)?;
        Ok(Value::Null)
    }

    /// Remember a device by name + services (+ platform id as a hint only;
    /// BLE identifiers are not stable across platforms).
    fn remember_device(&mut self, key: &str) {
        let Some(d) = self.dm.devices.get(key) else { return };
        let entry = Value::obj([
            ("key", key.into()),
            ("name", d.name.clone().into()),
            ("adapter", d.adapter_kind.into()),
            ("roles", Value::Arr(d.roles().into_iter().map(Value::from).collect())),
            ("last_used_utc", rl_domain::time::now_utc_ms().into()),
        ]);
        let mut prefs = self.store.read_value("devices.json").ok().flatten().unwrap_or_else(|| Value::Arr(vec![]));
        if let Value::Arr(a) = &mut prefs {
            a.retain(|d| d.get("key").and_then(|k| k.as_str()) != Some(key));
            a.push(entry);
            if a.len() > 20 {
                a.remove(0);
            }
        }
        let _ = self.store.write_value("devices.json", &prefs);
    }

    pub(crate) fn set_trainer(&mut self, p: &Value) -> R {
        if self.ride_active() {
            return Err("The trainer cannot be changed during a ride.".into());
        }
        let key = p.opt_str("key")?.map(|s| s.to_string());
        let now = self.now();
        self.dm.set_trainer(key.as_deref(), now)?;
        Ok(Value::Null)
    }

    pub(crate) fn assign_source(&mut self, p: &Value) -> R {
        let metric = Metric::parse(pstr(p, "metric", 30)?).ok_or("Unknown metric.")?;
        let src = match p.opt_str("device")? {
            None => None,
            Some(d) => {
                let service = Service::parse(pstr(p, "service", 30)?).ok_or("Unknown service.")?;
                if !self.dm.devices.contains_key(d) {
                    return Err("Unknown device.".into());
                }
                Some(SourceKey::new(d, service))
            }
        };
        self.dm.telemetry.assign(metric, src);
        Ok(Value::Null)
    }

    pub(crate) fn request_control(&mut self, _p: &Value) -> R {
        let now = self.now();
        self.dm.controller.request_control(now)?;
        Ok(Value::Null)
    }

    pub(crate) fn sim_rider(&mut self, p: &Value) -> R {
        let effort = p.opt_f64("effort_w")?;
        let cadence = p.opt_f64("cadence_rpm")?;
        let sim = self.sim().ok_or("Demo mode is off.")?;
        if let Some(e) = effort {
            sim.rider.effort_w = rl_json::check_range("effort_w", e, 0.0, 1200.0)?;
        }
        if let Some(c) = cadence {
            sim.rider.cadence_rpm = rl_json::check_range("cadence_rpm", c, 0.0, 160.0)?;
        }
        Ok(Value::obj([("effort_w", sim.rider.effort_w.into()), ("cadence_rpm", sim.rider.cadence_rpm.into())]))
    }

    pub(crate) fn sim_fault(&mut self, p: &Value) -> R {
        let dev = pstr(p, "device", 60)?.to_string();
        let fault = pstr(p, "fault", 40)?.to_string();
        let on = p.req_bool("on")?;
        let sim = self.sim().ok_or("Demo mode is off.")?;
        if !sim.set_fault(&dev, &fault, on) {
            return Err("Unknown simulator device or fault.".into());
        }
        Ok(Value::Null)
    }

    pub(crate) fn sim_json(&mut self) -> Value {
        let Some(sim) = self.sim() else { return Value::Null };
        let faults = Value::Arr(
            sim.devices
                .iter()
                .map(|d| {
                    let f = &d.faults;
                    Value::obj([
                        ("device", d.id.0.clone().into()),
                        ("deny_control", f.deny_control.into()),
                        ("drop_acks", f.drop_acks.into()),
                        ("fail_writes", f.fail_writes.into()),
                        ("offline", f.offline.into()),
                        ("stale", f.stale.into()),
                        ("malformed", f.malformed.into()),
                        ("busy", f.busy.into()),
                        ("no_simulation", f.no_simulation.into()),
                        ("slow_acks", (f.ack_latency_ms > 1000).into()),
                        ("low_power_limit", (d.max_power_w < 2000).into()),
                        ("low_grade_limit", (d.max_grade_pct < 20.0).into()),
                        ("low_battery", (d.battery < 10).into()),
                    ])
                })
                .collect(),
        );
        Value::obj([("effort_w", sim.rider.effort_w.into()), ("cadence_rpm", sim.rider.cadence_rpm.into()), ("faults", faults)])
    }
}
