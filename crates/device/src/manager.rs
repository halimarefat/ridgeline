//! Device manager: scanning, per-device connection state machine
//! (disconnected → connecting → discovering → subscribing → ready, with
//! reconnecting and faulted paths), capability discovery, notification
//! parsing into normalized telemetry, and routing of control-point traffic
//! to the trainer controller.
//!
//! A device's connection state is separate from the session's control state:
//! a trainer that reconnects is `ready` but not `controlled` until the rider
//! explicitly resumes control.

use crate::adapter::*;
use crate::controller::{FtmsCaps, TrainerController};
use crate::ftms::{FtmsFeatures, IndoorBikeData, PowerRange, ResistanceRange};
use crate::gatt::*;
use crate::sensors::{parse_battery_level, parse_utf8, CscMeasurement, CyclingPowerMeasurement, HeartRateMeasurement, RevolutionRate};
use crate::telemetry::{Metric, Service, SourceKey, Telemetry};
use rl_json::{json_enum, ToJson, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

pub const SCAN_DURATION_MS: u64 = 15_000;
pub const CONNECT_TIMEOUT_MS: u64 = 20_000;
const WHEEL_CIRCUMFERENCE_M: f64 = 2.105;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Discovering,
    Subscribing,
    Ready,
    Reconnecting,
    Faulted,
}
json_enum!(ConnState {
    Disconnected = "disconnected",
    Connecting = "connecting",
    Discovering = "discovering",
    Subscribing = "subscribing",
    Ready = "ready",
    Reconnecting = "reconnecting",
    Faulted = "faulted",
});

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub key: String,
    pub adapter: usize,
    pub adapter_kind: &'static str,
    pub id: DeviceId,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub adv_services: Vec<u16>,
    pub state: ConnState,
    pub want_connected: bool,
    pub reconnect_attempt: u32,
    pub next_reconnect_ms: u64,
    pub connect_started_ms: u64,
    pub services: Vec<u16>,
    pub characteristics: Vec<u16>,
    pub info: DeviceInfo,
    pub battery: Option<u8>,
    pub ftms_features: Option<FtmsFeatures>,
    pub power_range: Option<PowerRange>,
    pub resistance_range: Option<ResistanceRange>,
    pub subscribed: BTreeSet<u16>,
    pub pending: BTreeSet<RequestId>,
    pub last_seen_ms: u64,
    pub last_data_ms: Option<u64>,
    pub error: Option<String>,
    pub error_kind: Option<AdapterErrorKind>,
    pub parse_errors: u64,
    pub notifications: u64,
    crank_cps: RevolutionRate,
    crank_csc: RevolutionRate,
    wheel_csc: RevolutionRate,
}

impl DeviceEntry {
    fn new(adapter: usize, kind: &'static str, id: DeviceId, now: u64) -> Self {
        DeviceEntry {
            key: format!("{kind}:{id}"),
            adapter,
            adapter_kind: kind,
            id,
            name: None,
            rssi: None,
            adv_services: vec![],
            state: ConnState::Disconnected,
            want_connected: false,
            reconnect_attempt: 0,
            next_reconnect_ms: 0,
            connect_started_ms: 0,
            services: vec![],
            characteristics: vec![],
            info: DeviceInfo::default(),
            battery: None,
            ftms_features: None,
            power_range: None,
            resistance_range: None,
            subscribed: BTreeSet::new(),
            pending: BTreeSet::new(),
            last_seen_ms: now,
            last_data_ms: None,
            error: None,
            error_kind: None,
            parse_errors: 0,
            notifications: 0,
            crank_cps: RevolutionRate::crank(),
            crank_csc: RevolutionRate::crank(),
            wheel_csc: RevolutionRate::csc_wheel(),
        }
    }

    pub fn all_services(&self) -> Vec<u16> {
        let mut s: Vec<u16> = self.services.iter().chain(self.adv_services.iter()).copied().collect();
        s.sort();
        s.dedup();
        s
    }

    /// Roles this device can fill, from its (advertised or discovered) services.
    pub fn roles(&self) -> Vec<&'static str> {
        let s = self.all_services();
        let mut r = Vec::new();
        if s.contains(&SVC_FTMS) {
            r.push("trainer");
        }
        if s.contains(&SVC_FTMS) || s.contains(&SVC_CYCLING_POWER) {
            r.push("power");
        }
        if s.contains(&SVC_FTMS) || s.contains(&SVC_CSC) || s.contains(&SVC_CYCLING_POWER) {
            r.push("cadence");
        }
        if s.contains(&SVC_HEART_RATE) {
            r.push("heart_rate");
        }
        r
    }

    pub fn is_relevant(&self) -> bool {
        !self.roles().is_empty()
    }

    pub fn caps(&self) -> Option<FtmsCaps> {
        self.ftms_features.map(|f| FtmsCaps { features: f, power_range: self.power_range, resistance_range: self.resistance_range })
    }

    pub fn control_point_ready(&self) -> bool {
        self.subscribed.contains(&CHR_FTMS_CONTROL_POINT)
    }

    pub fn to_json(&self, now: u64, is_trainer: bool) -> Value {
        Value::obj([
            ("key", self.key.clone().into()),
            ("adapter", self.adapter_kind.into()),
            ("simulated", (self.adapter_kind == "simulator").into()),
            ("name", self.name.clone().unwrap_or_else(|| "Unnamed device".into()).into()),
            ("rssi", self.rssi.into()),
            ("state", self.state.to_json()),
            ("want_connected", self.want_connected.into()),
            ("roles", Value::Arr(self.roles().into_iter().map(Value::from).collect())),
            ("services", Value::Arr(self.all_services().iter().map(|s| Value::from(service_name(*s))).collect())),
            ("battery", self.battery.into()),
            ("manufacturer", self.info.manufacturer.clone().into()),
            ("model", self.info.model.clone().into()),
            ("firmware", self.info.firmware.clone().into()),
            ("caps", self.caps().map(|c| c.to_json()).unwrap_or(Value::Null)),
            ("control_point", self.control_point_ready().into()),
            ("is_trainer", is_trainer.into()),
            ("last_data_age_ms", self.last_data_ms.map(|t| now.saturating_sub(t)).into()),
            ("last_seen_age_ms", now.saturating_sub(self.last_seen_ms).into()),
            ("error", self.error.clone().into()),
            ("reconnect_attempt", self.reconnect_attempt.into()),
            ("parse_errors", self.parse_errors.into()),
        ])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterStatus {
    pub kind: &'static str,
    pub available: Option<bool>,
    pub message: Option<String>,
    pub scanning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceLog {
    pub at_ms: u64,
    pub device: String,
    pub level: &'static str,
    pub message: String,
}

pub struct DeviceManager {
    pub adapters: Vec<Box<dyn DeviceAdapter>>,
    pub adapter_status: Vec<AdapterStatus>,
    pub devices: BTreeMap<String, DeviceEntry>,
    pub telemetry: Telemetry,
    pub controller: TrainerController,
    pub trainer_key: Option<String>,
    scan_until: Vec<Option<u64>>,
    req_owner: HashMap<(usize, RequestId), String>,
    log: VecDeque<DeviceLog>,
    /// Keys connected-state transitions since last drain (for the session).
    transitions: Vec<(String, ConnState)>,
}

impl DeviceManager {
    pub fn new(adapters: Vec<Box<dyn DeviceAdapter>>) -> Self {
        let adapter_status = adapters.iter().map(|a| AdapterStatus { kind: a.kind(), available: None, message: None, scanning: false }).collect();
        let n = adapters.len();
        DeviceManager {
            adapters,
            adapter_status,
            devices: BTreeMap::new(),
            telemetry: Telemetry::new(),
            controller: TrainerController::new(),
            trainer_key: None,
            scan_until: vec![None; n],
            req_owner: HashMap::new(),
            log: VecDeque::new(),
            transitions: Vec::new(),
        }
    }

    pub fn add_adapter(&mut self, a: Box<dyn DeviceAdapter>) -> usize {
        self.adapter_status.push(AdapterStatus { kind: a.kind(), available: None, message: None, scanning: false });
        self.adapters.push(a);
        self.scan_until.push(None);
        self.adapters.len() - 1
    }

    pub fn adapter_index(&self, kind: &str) -> Option<usize> {
        self.adapters.iter().position(|a| a.kind() == kind)
    }

    fn note(&mut self, now: u64, device: &str, level: &'static str, msg: impl Into<String>) {
        self.log.push_back(DeviceLog { at_ms: now, device: device.to_string(), level, message: msg.into() });
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    pub fn drain_transitions(&mut self) -> Vec<(String, ConnState)> {
        std::mem::take(&mut self.transitions)
    }

    fn set_state(&mut self, key: &str, st: ConnState) {
        if let Some(d) = self.devices.get_mut(key) {
            if d.state != st {
                d.state = st;
                self.transitions.push((key.to_string(), st));
            }
        }
    }

    pub fn start_scan(&mut self, adapter: Option<usize>, now: u64) -> Result<(), String> {
        let idxs: Vec<usize> = match adapter {
            Some(i) if i < self.adapters.len() => vec![i],
            Some(_) => return Err("Unknown adapter.".into()),
            None => (0..self.adapters.len()).collect(),
        };
        let mut errors = Vec::new();
        for i in idxs {
            match self.adapters[i].start_scan() {
                Ok(()) => {
                    self.scan_until[i] = Some(now + SCAN_DURATION_MS);
                    self.adapter_status[i].scanning = true;
                }
                Err(e) => {
                    let msg = friendly_error(&e);
                    self.adapter_status[i].available = Some(!matches!(e.kind, AdapterErrorKind::AdapterUnavailable | AdapterErrorKind::PermissionDenied));
                    self.adapter_status[i].message = Some(msg.clone());
                    errors.push(msg);
                }
            }
        }
        if errors.is_empty() || self.adapter_status.iter().any(|s| s.scanning) {
            Ok(())
        } else {
            Err(errors.join(" "))
        }
    }

    pub fn stop_scan(&mut self) {
        for i in 0..self.adapters.len() {
            if self.scan_until[i].is_some() {
                self.adapters[i].stop_scan();
                self.scan_until[i] = None;
                self.adapter_status[i].scanning = false;
            }
        }
    }

    pub fn connect(&mut self, key: &str, now: u64) -> Result<(), String> {
        let d = self.devices.get_mut(key).ok_or("Unknown device. Scan again.")?;
        if matches!(d.state, ConnState::Ready | ConnState::Connecting | ConnState::Discovering | ConnState::Subscribing) {
            return Ok(());
        }
        d.want_connected = true;
        d.reconnect_attempt = 0;
        d.error = None;
        d.error_kind = None;
        d.connect_started_ms = now;
        let (ai, id) = (d.adapter, d.id.clone());
        self.set_state(key, ConnState::Connecting);
        self.note(now, key, "info", "Connecting…");
        if let Err(e) = self.adapters[ai].connect(&id) {
            let msg = friendly_error(&e);
            if let Some(d) = self.devices.get_mut(key) {
                d.error = Some(msg.clone());
                d.error_kind = Some(e.kind.clone());
            }
            self.set_state(key, ConnState::Faulted);
            return Err(msg);
        }
        Ok(())
    }

    pub fn disconnect(&mut self, key: &str, now: u64) {
        if let Some(d) = self.devices.get_mut(key) {
            d.want_connected = false;
            let (ai, id) = (d.adapter, d.id.clone());
            self.adapters[ai].disconnect(&id);
            self.set_state(key, ConnState::Disconnected);
            self.telemetry.set_connected(key, false);
            if self.trainer_key.as_deref() == Some(key) {
                self.controller.detach(now, "disconnected by rider");
            }
            self.note(now, key, "info", "Disconnected.");
        }
    }

    pub fn forget(&mut self, key: &str, now: u64) {
        self.disconnect(key, now);
        if self.trainer_key.as_deref() == Some(key) {
            self.trainer_key = None;
            self.telemetry.trainer = None;
            self.controller.unassign();
        }
        self.telemetry.forget_device(key);
        self.devices.remove(key);
    }

    /// Choose the controllable trainer (`None` = no trainer control).
    pub fn set_trainer(&mut self, key: Option<&str>, now: u64) -> Result<(), String> {
        match key {
            None => {
                self.trainer_key = None;
                self.telemetry.trainer = None;
                self.controller.unassign();
                Ok(())
            }
            Some(k) => {
                let d = self.devices.get(k).ok_or("Unknown device.")?;
                if !d.all_services().contains(&SVC_FTMS) {
                    return Err("This device does not offer the Fitness Machine (FTMS) service, so it cannot be controlled.".into());
                }
                if self.trainer_key.as_deref() != Some(k) {
                    self.controller.unassign();
                }
                self.trainer_key = Some(k.to_string());
                self.telemetry.trainer = Some(k.to_string());
                self.try_attach_trainer(now);
                Ok(())
            }
        }
    }

    fn try_attach_trainer(&mut self, _now: u64) {
        let Some(k) = self.trainer_key.clone() else { return };
        let Some(d) = self.devices.get(&k) else { return };
        if d.state == ConnState::Ready && d.control_point_ready() {
            if let Some(caps) = d.caps() {
                self.controller.attach(d.adapter, d.id.clone(), k, caps);
            }
        }
    }

    fn issue_reads(&mut self, key: &str) {
        let Some(d) = self.devices.get(key) else { return };
        let (ai, id) = (d.adapter, d.id.clone());
        let wanted = [
            CHR_MANUFACTURER_NAME,
            CHR_MODEL_NUMBER,
            CHR_FIRMWARE_REVISION,
            CHR_BATTERY_LEVEL,
            CHR_FTMS_FEATURE,
            CHR_SUPPORTED_POWER_RANGE,
            CHR_SUPPORTED_RESISTANCE_RANGE,
        ];
        let chars = d.characteristics.clone();
        let mut reqs = Vec::new();
        for c in wanted {
            if chars.contains(&c) {
                if let Ok(r) = self.adapters[ai].read(&id, c) {
                    reqs.push(r);
                }
            }
        }
        for r in &reqs {
            self.req_owner.insert((ai, *r), key.to_string());
        }
        let d = self.devices.get_mut(key).unwrap();
        d.pending.extend(reqs);
        if d.pending.is_empty() {
            self.begin_subscribe(key);
        }
    }

    fn begin_subscribe(&mut self, key: &str) {
        self.set_state(key, ConnState::Subscribing);
        let Some(d) = self.devices.get(key) else { return };
        let (ai, id) = (d.adapter, d.id.clone());
        let wanted = [
            CHR_INDOOR_BIKE_DATA,
            CHR_FTMS_STATUS,
            CHR_FTMS_CONTROL_POINT,
            CHR_CYCLING_POWER_MEASUREMENT,
            CHR_HEART_RATE_MEASUREMENT,
            CHR_CSC_MEASUREMENT,
            CHR_BATTERY_LEVEL,
        ];
        let chars = d.characteristics.clone();
        let mut reqs = Vec::new();
        for c in wanted {
            if chars.contains(&c) {
                if let Ok(r) = self.adapters[ai].subscribe(&id, c) {
                    reqs.push(r);
                }
            }
        }
        for r in &reqs {
            self.req_owner.insert((ai, *r), key.to_string());
        }
        let d = self.devices.get_mut(key).unwrap();
        d.pending.extend(reqs);
        if d.pending.is_empty() {
            self.become_ready(key, 0);
        }
    }

    fn become_ready(&mut self, key: &str, now: u64) {
        self.set_state(key, ConnState::Ready);
        self.telemetry.set_connected(key, true);
        let has_ftms;
        {
            let d = self.devices.get_mut(key).unwrap();
            d.reconnect_attempt = 0;
            d.error = None;
            has_ftms = d.ftms_features.is_some();
        }
        self.note(now, key, "info", "Ready.");
        if self.trainer_key.is_none() && has_ftms {
            self.trainer_key = Some(key.to_string());
            self.telemetry.trainer = Some(key.to_string());
        }
        if self.trainer_key.as_deref() == Some(key) {
            self.try_attach_trainer(now);
        }
    }

    fn complete_pending(&mut self, ai: usize, req: RequestId, now: u64) -> Option<String> {
        let key = self.req_owner.remove(&(ai, req))?;
        let d = self.devices.get_mut(&key)?;
        d.pending.remove(&req);
        if d.pending.is_empty() {
            match d.state {
                ConnState::Discovering => self.begin_subscribe(&key),
                ConnState::Subscribing => self.become_ready(&key, now),
                _ => {}
            }
        }
        Some(key)
    }

    fn key_for(&self, ai: usize, id: &DeviceId) -> String {
        format!("{}:{}", self.adapters[ai].kind(), id)
    }

    fn handle(&mut self, ai: usize, ev: AdapterEvent, now: u64) {
        match ev {
            AdapterEvent::AdapterState { available, message } => {
                self.adapter_status[ai].available = Some(available);
                self.adapter_status[ai].message = message;
            }
            AdapterEvent::ScanStarted => self.adapter_status[ai].scanning = true,
            AdapterEvent::ScanStopped => {
                self.adapter_status[ai].scanning = false;
                self.scan_until[ai] = None;
            }
            AdapterEvent::Discovered(ad) => {
                let key = self.key_for(ai, &ad.id);
                let kind = self.adapters[ai].kind();
                let d = self.devices.entry(key).or_insert_with(|| DeviceEntry::new(ai, kind, ad.id.clone(), now));
                if ad.name.is_some() {
                    d.name = ad.name.map(|n| n.chars().filter(|c| !c.is_control()).take(48).collect());
                }
                d.rssi = ad.rssi.or(d.rssi);
                for s in ad.services {
                    if !d.adv_services.contains(&s) {
                        d.adv_services.push(s);
                    }
                }
                d.last_seen_ms = now;
            }
            AdapterEvent::Connected { id, services, characteristics } => {
                let key = self.key_for(ai, &id);
                let kind = self.adapters[ai].kind();
                let d = self.devices.entry(key.clone()).or_insert_with(|| DeviceEntry::new(ai, kind, id.clone(), now));
                if !d.want_connected {
                    // Late connect after the rider cancelled.
                    self.adapters[ai].disconnect(&id);
                    return;
                }
                d.services = services;
                d.characteristics = characteristics;
                d.subscribed.clear();
                d.pending.clear();
                d.crank_cps.reset();
                d.crank_csc.reset();
                d.wheel_csc.reset();
                self.set_state(&key, ConnState::Discovering);
                self.issue_reads(&key);
            }
            AdapterEvent::ConnectFailed { id, error } => {
                let key = self.key_for(ai, &id);
                let msg = friendly_error(&error);
                let reconnecting = self.devices.get(&key).map(|d| d.want_connected && d.reconnect_attempt > 0).unwrap_or(false);
                if let Some(d) = self.devices.get_mut(&key) {
                    d.error = Some(msg.clone());
                    d.error_kind = Some(error.kind.clone());
                }
                if reconnecting {
                    self.schedule_reconnect(&key, now);
                } else {
                    self.set_state(&key, ConnState::Faulted);
                    if let Some(d) = self.devices.get_mut(&key) {
                        d.want_connected = false;
                    }
                }
                self.note(now, &key, "error", msg);
            }
            AdapterEvent::Disconnected { id, reason } => {
                let key = self.key_for(ai, &id);
                self.telemetry.set_connected(&key, false);
                if self.trainer_key.as_deref() == Some(key.as_str()) {
                    self.controller.detach(now, reason.as_deref().unwrap_or("disconnected"));
                }
                let want = self.devices.get(&key).map(|d| d.want_connected).unwrap_or(false);
                if let Some(d) = self.devices.get_mut(&key) {
                    d.subscribed.clear();
                    for r in std::mem::take(&mut d.pending) {
                        self.req_owner.remove(&(ai, r));
                    }
                }
                self.note(now, &key, "warn", format!("Disconnected{}", reason.map(|r| format!(": {r}")).unwrap_or_default()));
                if want {
                    if let Some(d) = self.devices.get_mut(&key) {
                        d.reconnect_attempt = 0;
                    }
                    self.schedule_reconnect(&key, now);
                } else {
                    self.set_state(&key, ConnState::Disconnected);
                }
            }
            AdapterEvent::ReadResult { id, req, characteristic, result } => {
                let key = self.key_for(ai, &id);
                if let (Ok(data), Some(d)) = (&result, self.devices.get_mut(&key)) {
                    match characteristic {
                        CHR_MANUFACTURER_NAME => d.info.manufacturer = Some(parse_utf8(data)),
                        CHR_MODEL_NUMBER => d.info.model = Some(parse_utf8(data)),
                        CHR_FIRMWARE_REVISION => d.info.firmware = Some(parse_utf8(data)),
                        CHR_BATTERY_LEVEL => d.battery = parse_battery_level(data).ok(),
                        CHR_FTMS_FEATURE => d.ftms_features = FtmsFeatures::parse(data).ok(),
                        CHR_SUPPORTED_POWER_RANGE => d.power_range = PowerRange::parse(data).ok(),
                        CHR_SUPPORTED_RESISTANCE_RANGE => d.resistance_range = ResistanceRange::parse(data).ok(),
                        _ => {}
                    }
                }
                self.complete_pending(ai, req, now);
            }
            AdapterEvent::SubscribeResult { id, req, characteristic, result } => {
                let key = self.key_for(ai, &id);
                match &result {
                    Ok(()) => {
                        if let Some(d) = self.devices.get_mut(&key) {
                            d.subscribed.insert(characteristic);
                        }
                    }
                    Err(e) => {
                        let m = format!("Could not subscribe to {characteristic:#06x}: {}", e.message);
                        self.note(now, &key, "warn", m);
                    }
                }
                self.complete_pending(ai, req, now);
            }
            AdapterEvent::WriteComplete { id, req, result, .. } => {
                let key = self.key_for(ai, &id);
                if self.trainer_key.as_deref() == Some(key.as_str()) {
                    self.controller.on_write_complete(req, &result, now);
                }
            }
            AdapterEvent::Notification { id, characteristic, data } => {
                let key = self.key_for(ai, &id);
                self.on_notification(&key, characteristic, &data, now);
            }
        }
    }

    fn schedule_reconnect(&mut self, key: &str, now: u64) {
        if let Some(d) = self.devices.get_mut(key) {
            d.reconnect_attempt += 1;
            let backoff = (1000u64 << d.reconnect_attempt.min(4)).min(15_000);
            d.next_reconnect_ms = now + backoff;
        }
        self.set_state(key, ConnState::Reconnecting);
    }

    fn on_notification(&mut self, key: &str, ch: u16, data: &[u8], now: u64) {
        let is_trainer = self.trainer_key.as_deref() == Some(key);
        if ch == CHR_FTMS_CONTROL_POINT {
            if is_trainer {
                self.controller.on_indication(data, now);
            }
            return;
        }
        if ch == CHR_FTMS_STATUS {
            if is_trainer {
                self.controller.on_machine_status(data, now);
            }
            return;
        }
        let Some(d) = self.devices.get_mut(key) else { return };
        d.notifications += 1;
        let mut obs: Vec<(Metric, Service, f64)> = Vec::new();
        let ok = match ch {
            CHR_INDOOR_BIKE_DATA => match IndoorBikeData::parse(data) {
                Ok(ibd) => {
                    if let Some(p) = ibd.power_w {
                        obs.push((Metric::Power, Service::Ftms, p as f64));
                    }
                    if let Some(c) = ibd.cadence_rpm {
                        obs.push((Metric::Cadence, Service::Ftms, c));
                    }
                    if let Some(h) = ibd.heart_rate_bpm {
                        if h > 0 {
                            obs.push((Metric::HeartRate, Service::Ftms, h as f64));
                        }
                    }
                    if let Some(s) = ibd.speed_kmh {
                        obs.push((Metric::TrainerSpeed, Service::Ftms, s));
                    }
                    true
                }
                Err(_) => false,
            },
            CHR_CYCLING_POWER_MEASUREMENT => match CyclingPowerMeasurement::parse(data) {
                Ok(m) => {
                    obs.push((Metric::Power, Service::CyclingPower, m.power_w as f64));
                    if let Some((revs, t)) = m.crank {
                        if let Some(rpm) = d.crank_cps.update(revs as u64, t, now) {
                            obs.push((Metric::Cadence, Service::CyclingPower, rpm));
                        }
                    }
                    true
                }
                Err(_) => false,
            },
            CHR_HEART_RATE_MEASUREMENT => match HeartRateMeasurement::parse(data) {
                Ok(m) => {
                    if m.bpm > 0 {
                        obs.push((Metric::HeartRate, Service::HeartRate, m.bpm as f64));
                    }
                    true
                }
                Err(_) => false,
            },
            CHR_CSC_MEASUREMENT => match CscMeasurement::parse(data) {
                Ok(m) => {
                    if let Some((revs, t)) = m.crank {
                        if let Some(rpm) = d.crank_csc.update(revs as u64, t, now) {
                            obs.push((Metric::Cadence, Service::Csc, rpm));
                        }
                    }
                    if let Some((revs, t)) = m.wheel {
                        if let Some(rpm) = d.wheel_csc.update(revs as u64, t, now) {
                            obs.push((Metric::TrainerSpeed, Service::Csc, rpm * WHEEL_CIRCUMFERENCE_M * 60.0 / 1000.0));
                        }
                    }
                    true
                }
                Err(_) => false,
            },
            CHR_BATTERY_LEVEL => {
                d.battery = parse_battery_level(data).ok();
                true
            }
            _ => true,
        };
        if ok {
            d.last_data_ms = Some(now);
        } else {
            d.parse_errors += 1;
        }
        for (m, s, v) in obs {
            self.telemetry.observe(m, SourceKey::new(key, s), v, now);
        }
    }

    pub fn tick(&mut self, now: u64) {
        // Scan timeouts.
        for i in 0..self.adapters.len() {
            if let Some(until) = self.scan_until[i] {
                if now >= until {
                    self.adapters[i].stop_scan();
                    self.scan_until[i] = None;
                    self.adapter_status[i].scanning = false;
                }
            }
        }
        // Adapter events.
        for i in 0..self.adapters.len() {
            let evs = self.adapters[i].poll(now);
            for e in evs {
                self.handle(i, e, now);
            }
        }
        // Reconnects and connect timeouts.
        let keys: Vec<String> = self.devices.keys().cloned().collect();
        for k in keys {
            let (state, want, next, started, ai, id) = {
                let d = &self.devices[&k];
                (d.state, d.want_connected, d.next_reconnect_ms, d.connect_started_ms, d.adapter, d.id.clone())
            };
            if state == ConnState::Reconnecting && want && now >= next {
                if let Some(d) = self.devices.get_mut(&k) {
                    d.connect_started_ms = now;
                }
                self.set_state(&k, ConnState::Connecting);
                if self.adapters[ai].connect(&id).is_err() {
                    self.schedule_reconnect(&k, now);
                }
            } else if matches!(state, ConnState::Connecting | ConnState::Discovering | ConnState::Subscribing) && now.saturating_sub(started) > CONNECT_TIMEOUT_MS {
                self.note(now, &k, "error", "Connection attempt timed out.");
                self.adapters[ai].disconnect(&id);
                let reconnecting = self.devices[&k].reconnect_attempt > 0;
                if reconnecting && want {
                    self.schedule_reconnect(&k, now);
                } else {
                    if let Some(d) = self.devices.get_mut(&k) {
                        d.error = Some("The device did not finish connecting. Wake it (pedal or wear the strap) and try again.".into());
                        d.want_connected = false;
                    }
                    self.set_state(&k, ConnState::Faulted);
                }
            }
        }
        // Trainer controller.
        if let Some(ai) = self.controller.adapter_index {
            if ai < self.adapters.len() {
                self.controller.tick(now, self.adapters[ai].as_mut());
            }
        }
    }

    pub fn trainer(&self) -> Option<&DeviceEntry> {
        self.trainer_key.as_ref().and_then(|k| self.devices.get(k))
    }

    pub fn to_json(&self, now: u64, show_all: bool) -> Value {
        let tk = self.trainer_key.as_deref();
        Value::obj([
            (
                "adapters",
                Value::Arr(
                    self.adapter_status
                        .iter()
                        .map(|a| Value::obj([("kind", a.kind.into()), ("available", a.available.into()), ("message", a.message.clone().into()), ("scanning", a.scanning.into())]))
                        .collect(),
                ),
            ),
            (
                "devices",
                Value::Arr(
                    self.devices
                        .values()
                        .filter(|d| show_all || d.is_relevant() || d.want_connected)
                        .map(|d| d.to_json(now, tk == Some(d.key.as_str())))
                        .collect(),
                ),
            ),
            ("trainer", tk.into()),
            ("control", self.controller.to_json()),
            (
                "log",
                Value::Arr(
                    self.log
                        .iter()
                        .rev()
                        .take(30)
                        .map(|l| Value::obj([("at_ms", l.at_ms.into()), ("device", l.device.clone().into()), ("level", l.level.into()), ("message", l.message.clone().into())]))
                        .collect(),
                ),
            ),
        ])
    }
}

pub fn friendly_error(e: &AdapterError) -> String {
    match e.kind {
        AdapterErrorKind::AdapterUnavailable => "Bluetooth is off or no Bluetooth adapter was found. Turn Bluetooth on and try again.".into(),
        AdapterErrorKind::PermissionDenied => "Bluetooth permission was denied. Allow Ridgeline to use Bluetooth in your system privacy settings.".into(),
        AdapterErrorKind::Busy => "The device seems to be connected to another app (for example another training app or a phone). Close that app or disconnect it, then try again.".into(),
        AdapterErrorKind::Timeout => "The device did not respond. It may be asleep: pedal the trainer or wear the heart-rate strap, then try again.".into(),
        AdapterErrorKind::NotFound => "The device was not found. Scan again.".into(),
        AdapterErrorKind::Disconnected => "The device disconnected.".into(),
        AdapterErrorKind::Unsupported => format!("Not supported by this device: {}", e.message),
        AdapterErrorKind::Other => format!("Bluetooth error: {}", e.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::ControlState;
    use crate::ftms::ControlCommand;
    use crate::simulator::SimulatorAdapter;
    use crate::telemetry::Freshness;

    fn run(m: &mut DeviceManager, t: &mut u64, ms: u64) {
        let end = *t + ms;
        while *t < end {
            *t += 50;
            m.tick(*t);
        }
    }

    fn connected_rig() -> (DeviceManager, u64) {
        let mut m = DeviceManager::new(vec![Box::new(SimulatorAdapter::new())]);
        let mut t = 0;
        m.tick(t);
        m.start_scan(None, t).unwrap();
        run(&mut m, &mut t, 1000);
        assert_eq!(m.devices.len(), 4);
        for k in ["simulator:sim-trainer", "simulator:sim-hrm", "simulator:sim-cadence"] {
            m.connect(k, t).unwrap();
        }
        run(&mut m, &mut t, 2000);
        (m, t)
    }

    #[test]
    fn full_connection_flow_and_simultaneous_sensors() {
        let (mut m, mut t) = connected_rig();
        for k in ["simulator:sim-trainer", "simulator:sim-hrm", "simulator:sim-cadence"] {
            assert_eq!(m.devices[k].state, ConnState::Ready, "{k}");
        }
        let tr = &m.devices["simulator:sim-trainer"];
        assert!(tr.caps().unwrap().features.supports_simulation());
        assert_eq!(tr.info.manufacturer.as_deref(), Some("Ridgeline Simulator"));
        assert_eq!(m.trainer_key.as_deref(), Some("simulator:sim-trainer"), "FTMS device auto-assigned");
        assert_eq!(m.controller.state, ControlState::NotControlled);
        assert_eq!(m.devices["simulator:sim-hrm"].battery, Some(78));
        run(&mut m, &mut t, 3000);
        assert!(m.telemetry.value(Metric::Power, t).is_some());
        assert!(m.telemetry.value(Metric::HeartRate, t).is_some());
        let cad = m.telemetry.reading(Metric::Cadence, t);
        assert!(cad.value.unwrap() > 70.0);
        // Rider chooses the separate cadence sensor.
        m.telemetry.assign(Metric::Cadence, Some(SourceKey::new("simulator:sim-cadence", Service::Csc)));
        run(&mut m, &mut t, 3000);
        let c = m.telemetry.value(Metric::Cadence, t).unwrap();
        assert!((c - 88.0).abs() < 8.0, "{c}");
    }

    #[test]
    fn busy_device_reports_other_app() {
        let mut sim = SimulatorAdapter::new();
        sim.set_fault("sim-trainer", "busy", true);
        let mut m = DeviceManager::new(vec![Box::new(sim)]);
        let mut t = 0;
        m.start_scan(None, t).unwrap();
        run(&mut m, &mut t, 1000);
        m.connect("simulator:sim-trainer", t).unwrap();
        run(&mut m, &mut t, 1000);
        let d = &m.devices["simulator:sim-trainer"];
        assert_eq!(d.state, ConnState::Faulted);
        assert!(d.error.as_ref().unwrap().contains("another app"));
    }

    #[test]
    fn stale_sensor_and_reconnect() {
        let (mut m, mut t) = connected_rig();
        run(&mut m, &mut t, 2000);
        // HR strap stops sending: value becomes stale (not a plausible number).
        {
            let sim = m.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap();
            sim.set_fault("sim-hrm", "stale", true);
        }
        run(&mut m, &mut t, 4000);
        let r = m.telemetry.reading(Metric::HeartRate, t);
        assert_eq!(r.value, None);
        assert_eq!(r.freshness, Freshness::Stale);
        // Trainer drops and comes back: state machine reconnects; control not restored.
        m.controller.request_control(t).unwrap();
        run(&mut m, &mut t, 1000);
        assert_eq!(m.controller.state, ControlState::Controlled);
        {
            let sim = m.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap();
            sim.set_fault("sim-trainer", "offline", true);
        }
        run(&mut m, &mut t, 500);
        assert_eq!(m.devices["simulator:sim-trainer"].state, ConnState::Reconnecting);
        assert_eq!(m.controller.state, ControlState::Lost);
        assert_eq!(m.telemetry.reading(Metric::Power, t).freshness, Freshness::Disconnected);
        {
            let sim = m.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap();
            sim.set_fault("sim-trainer", "offline", false);
        }
        run(&mut m, &mut t, 20_000);
        assert_eq!(m.devices["simulator:sim-trainer"].state, ConnState::Ready);
        assert_eq!(m.controller.state, ControlState::NotControlled, "control must be re-requested explicitly");
        assert!(m.controller.set_target(ControlCommand::SetTargetPower { watts: 200 }, t).is_err());
    }

    #[test]
    fn malformed_packets_are_counted_not_used() {
        let (mut m, mut t) = connected_rig();
        {
            let sim = m.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap();
            sim.set_fault("sim-hrm", "malformed", true);
        }
        run(&mut m, &mut t, 5000);
        assert!(m.devices["simulator:sim-hrm"].parse_errors > 0);
        assert_eq!(m.telemetry.value(Metric::HeartRate, t), None);
    }
}
