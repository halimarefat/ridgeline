//! Telemetry normalization: one active source per metric, explicit
//! staleness, provenance, and no invented values.
//!
//! Every observation is kept per source (device + service). Only the selected
//! source feeds the metric used for display, control and recording; other
//! sources stay visible as alternatives so discrepancies (e.g. trainer vs.
//! power meter) are not hidden. Missing is `None`, never zero; a measured
//! zero (e.g. coasting cadence) is `Some(0.0)`.

use rl_json::{json_enum, ToJson, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Metric {
    Power,
    Cadence,
    HeartRate,
    /// Trainer-reported speed. Informational only: virtual road speed comes
    /// from the physics model, not from the flywheel.
    TrainerSpeed,
}
json_enum!(Metric { Power = "power", Cadence = "cadence", HeartRate = "heart_rate", TrainerSpeed = "trainer_speed" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Service {
    Ftms,
    CyclingPower,
    HeartRate,
    Csc,
}
json_enum!(Service { Ftms = "ftms", CyclingPower = "cycling_power", HeartRate = "heart_rate", Csc = "csc" });

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceKey {
    pub device: String,
    pub service: Service,
}

impl SourceKey {
    pub fn new(device: &str, service: Service) -> SourceKey {
        SourceKey { device: device.to_string(), service }
    }
    pub fn to_json(&self) -> Value {
        Value::obj([("device", self.device.clone().into()), ("service", self.service.to_json())])
    }
    /// Compact provenance label stored with samples: "<device>|<service>".
    pub fn label(&self) -> String {
        format!("{}|{}", self.device, self.service.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obs {
    pub value: f64,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    /// Source device is disconnected.
    Disconnected,
    NoSource,
}
json_enum!(Freshness { Fresh = "fresh", Stale = "stale", Disconnected = "disconnected", NoSource = "no_source" });

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Only set when fresh. Stale values are never presented as current.
    pub value: Option<f64>,
    pub last_value: Option<f64>,
    pub age_ms: Option<u64>,
    pub freshness: Freshness,
    pub source: Option<SourceKey>,
    pub auto_selected: bool,
}

impl Reading {
    pub fn to_json(&self) -> Value {
        Value::obj([
            ("value", self.value.into()),
            ("last_value", self.last_value.into()),
            ("age_ms", self.age_ms.into()),
            ("freshness", self.freshness.to_json()),
            ("source", self.source.as_ref().map(|s| s.to_json()).unwrap_or(Value::Null)),
            ("auto", self.auto_selected.into()),
        ])
    }
}

pub const DEFAULT_STALE_MS: u64 = 3000;

#[derive(Debug, Default)]
pub struct Telemetry {
    latest: BTreeMap<(Metric, SourceKey), Obs>,
    assigned: BTreeMap<Metric, SourceKey>,
    /// Devices currently connected (others are reported as disconnected).
    connected: BTreeMap<String, bool>,
    /// Per-device stale threshold overrides.
    stale_ms: BTreeMap<String, u64>,
    /// The device chosen as controllable trainer (preferred for power/cadence).
    pub trainer: Option<String>,
}

fn priority(metric: Metric, src: &SourceKey, trainer: Option<&str>) -> i32 {
    let is_trainer = trainer == Some(src.device.as_str());
    match (metric, src.service) {
        (Metric::Power, Service::Ftms) if is_trainer => 100,
        (Metric::Power, Service::CyclingPower) if is_trainer => 90,
        (Metric::Power, Service::CyclingPower) => 50,
        (Metric::Power, Service::Ftms) => 40,
        (Metric::Cadence, Service::Ftms) if is_trainer => 100,
        (Metric::Cadence, Service::Csc) => 80,
        (Metric::Cadence, Service::CyclingPower) => 70,
        (Metric::Cadence, Service::Ftms) => 40,
        (Metric::HeartRate, Service::HeartRate) => 100,
        (Metric::HeartRate, Service::Ftms) => 30,
        (Metric::TrainerSpeed, Service::Ftms) if is_trainer => 100,
        (Metric::TrainerSpeed, _) => 10,
        _ => 0,
    }
}

impl Telemetry {
    pub fn new() -> Self {
        Telemetry::default()
    }

    pub fn observe(&mut self, metric: Metric, src: SourceKey, value: f64, at_ms: u64) {
        if !value.is_finite() {
            return;
        }
        let ok = match metric {
            Metric::Power => (-100.0..=4000.0).contains(&value),
            Metric::Cadence => (0.0..=250.0).contains(&value),
            Metric::HeartRate => (25.0..=250.0).contains(&value),
            Metric::TrainerSpeed => (0.0..=150.0).contains(&value),
        };
        if ok {
            self.latest.insert((metric, src), Obs { value, at_ms });
        }
    }

    pub fn set_connected(&mut self, device: &str, connected: bool) {
        self.connected.insert(device.to_string(), connected);
    }

    pub fn set_stale_ms(&mut self, device: &str, ms: u64) {
        self.stale_ms.insert(device.to_string(), ms.clamp(500, 60_000));
    }

    /// Rider's explicit source choice. `None` returns to automatic selection.
    pub fn assign(&mut self, metric: Metric, src: Option<SourceKey>) {
        match src {
            Some(s) => {
                self.assigned.insert(metric, s);
            }
            None => {
                self.assigned.remove(&metric);
            }
        }
    }

    pub fn assignment(&self, metric: Metric) -> Option<&SourceKey> {
        self.assigned.get(&metric)
    }

    pub fn forget_device(&mut self, device: &str) {
        self.latest.retain(|(_, s), _| s.device != device);
        self.assigned.retain(|_, s| s.device != device);
        self.connected.remove(device);
    }

    fn stale_after(&self, device: &str) -> u64 {
        self.stale_ms.get(device).copied().unwrap_or(DEFAULT_STALE_MS)
    }

    /// Candidate sources for a metric, best first.
    pub fn candidates(&self, metric: Metric) -> Vec<SourceKey> {
        let mut v: Vec<SourceKey> = self.latest.keys().filter(|(m, _)| *m == metric).map(|(_, s)| s.clone()).collect();
        v.sort_by_key(|s| -priority(metric, s, self.trainer.as_deref()));
        v.dedup();
        v
    }

    fn eval(&self, metric: Metric, src: &SourceKey, now_ms: u64) -> (Freshness, Option<Obs>) {
        let obs = self.latest.get(&(metric, src.clone())).copied();
        if !self.connected.get(&src.device).copied().unwrap_or(false) {
            return (Freshness::Disconnected, obs);
        }
        match obs {
            None => (Freshness::Stale, None),
            Some(o) if now_ms.saturating_sub(o.at_ms) > self.stale_after(&src.device) => (Freshness::Stale, Some(o)),
            Some(o) => (Freshness::Fresh, Some(o)),
        }
    }

    pub fn reading(&self, metric: Metric, now_ms: u64) -> Reading {
        let (src, auto) = match self.assigned.get(&metric) {
            Some(s) => (Some(s.clone()), false),
            None => {
                // Automatic: best fresh candidate, else best candidate at all.
                let c = self.candidates(metric);
                let fresh = c.iter().find(|s| self.eval(metric, s, now_ms).0 == Freshness::Fresh).cloned();
                (fresh.or_else(|| c.first().cloned()), true)
            }
        };
        let Some(src) = src else {
            return Reading { value: None, last_value: None, age_ms: None, freshness: Freshness::NoSource, source: None, auto_selected: auto };
        };
        let (fr, obs) = self.eval(metric, &src, now_ms);
        Reading {
            value: if fr == Freshness::Fresh { obs.map(|o| o.value) } else { None },
            last_value: obs.map(|o| o.value),
            age_ms: obs.map(|o| now_ms.saturating_sub(o.at_ms)),
            freshness: fr,
            source: Some(src),
            auto_selected: auto,
        }
    }

    /// Fresh value or `None`.
    pub fn value(&self, metric: Metric, now_ms: u64) -> Option<f64> {
        self.reading(metric, now_ms).value
    }

    /// All sources and their current values, for discrepancy display.
    pub fn alternatives_json(&self, metric: Metric, now_ms: u64) -> Value {
        Value::Arr(
            self.candidates(metric)
                .iter()
                .map(|s| {
                    let (fr, o) = self.eval(metric, s, now_ms);
                    Value::obj([("source", s.to_json()), ("value", if fr == Freshness::Fresh { o.map(|o| o.value).into() } else { Value::Null }), ("freshness", fr.to_json())])
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_staleness_and_provenance() {
        let mut t = Telemetry::new();
        t.trainer = Some("ble:trainer".into());
        t.set_connected("ble:trainer", true);
        t.set_connected("ble:pm", true);
        t.observe(Metric::Power, SourceKey::new("ble:pm", Service::CyclingPower), 190.0, 1000);
        t.observe(Metric::Power, SourceKey::new("ble:trainer", Service::Ftms), 200.0, 1000);
        t.observe(Metric::Power, SourceKey::new("ble:trainer", Service::CyclingPower), 201.0, 1000);
        // Automatic: trainer FTMS wins; duplicates from the trainer's CPS are not double counted.
        let r = t.reading(Metric::Power, 1500);
        assert_eq!(r.value, Some(200.0));
        assert_eq!(r.source.unwrap().service, Service::Ftms);
        // Rider chooses external power meter.
        t.assign(Metric::Power, Some(SourceKey::new("ble:pm", Service::CyclingPower)));
        assert_eq!(t.value(Metric::Power, 1500), Some(190.0));
        // Stale after 3 s: no current value, last value kept for context.
        let r = t.reading(Metric::Power, 4500);
        assert_eq!(r.value, None);
        assert_eq!(r.freshness, Freshness::Stale);
        assert_eq!(r.last_value, Some(190.0));
        // Disconnected source is reported as such.
        t.set_connected("ble:pm", false);
        assert_eq!(t.reading(Metric::Power, 1500).freshness, Freshness::Disconnected);
        // No HR source at all.
        assert_eq!(t.reading(Metric::HeartRate, 1500).freshness, Freshness::NoSource);
        // Measured zero cadence is a value, not missing.
        t.observe(Metric::Cadence, SourceKey::new("ble:trainer", Service::Ftms), 0.0, 2000);
        assert_eq!(t.value(Metric::Cadence, 2100), Some(0.0));
        // Implausible values are rejected.
        t.observe(Metric::HeartRate, SourceKey::new("ble:trainer", Service::Ftms), 0.0, 2000);
        assert_eq!(t.reading(Metric::HeartRate, 2100).freshness, Freshness::NoSource);
    }

    #[test]
    fn auto_selection_falls_back_to_fresh_candidate() {
        let mut t = Telemetry::new();
        t.trainer = Some("t".into());
        t.set_connected("t", true);
        t.set_connected("c", true);
        t.observe(Metric::Cadence, SourceKey::new("t", Service::Ftms), 90.0, 0);
        t.observe(Metric::Cadence, SourceKey::new("c", Service::Csc), 88.0, 5000);
        let r = t.reading(Metric::Cadence, 5500);
        assert_eq!(r.value, Some(88.0), "trainer cadence is stale, CSC is fresh");
    }
}
