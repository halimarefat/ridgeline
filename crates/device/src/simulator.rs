//! Deterministic trainer/sensor simulator implementing the same
//! [`DeviceAdapter`] contract as real BLE.
//!
//! Simulated devices speak real GATT byte formats: the trainer decodes FTMS
//! control point writes and answers with FTMS indications; sensors emit
//! encoded HR / CSC / Cycling Power notifications. This exercises the real
//! parsers and controller, but it is *not* hardware evidence.
//!
//! Faults can be injected per device: control denied, dropped
//! acknowledgments, write failures, disconnection, stale telemetry,
//! malformed packets, busy (held by another app), unsupported simulation
//! mode, latency and power/grade saturation.

use crate::adapter::*;
use crate::bytes::Writer;
use crate::ftms::{self, ControlCommand, ControlResponse, FtmsFeatures, IndoorBikeData, PowerRange, ResultCode, SimParams, StopKind};
use crate::gatt::*;
use crate::sensors::{CscMeasurement, CyclingPowerMeasurement, HeartRateMeasurement};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimKind {
    Trainer,
    HeartRate,
    Cadence,
    PowerMeter,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimFaults {
    pub deny_control: bool,
    pub drop_acks: bool,
    pub fail_writes: bool,
    /// Device unreachable: drops the link and refuses connections.
    pub offline: bool,
    /// Connected but stops sending notifications.
    pub stale: bool,
    /// Next notifications are truncated by one byte.
    pub malformed: bool,
    /// Another app holds the device; connect fails with Busy.
    pub busy: bool,
    /// Trainer advertises no simulation-mode support.
    pub no_simulation: bool,
    pub ack_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrainerMode {
    Idle,
    Erg(i16),
    Simulation(SimParams),
    Resistance(f64),
}

#[derive(Debug, Clone)]
pub struct SimDevice {
    pub id: DeviceId,
    pub name: String,
    pub kind: SimKind,
    pub connected: bool,
    pub subscribed: BTreeSet<u16>,
    pub battery: u8,
    pub faults: SimFaults,
    next_notify_ms: u64,
    crank_revs: f64,
    crank_event_ticks: u16,
    last_whole_rev: u64,
    // Trainer-only state
    pub control_granted: bool,
    pub started: bool,
    pub mode: TrainerMode,
    pub max_power_w: i16,
    pub max_grade_pct: f64,
    pub applied_power_w: f64,
    /// Every decoded control command, for tests and diagnostics.
    pub command_log: Vec<(u64, ControlCommand)>,
}

impl SimDevice {
    fn new(id: &str, name: &str, kind: SimKind) -> Self {
        SimDevice {
            id: DeviceId(id.into()),
            name: name.into(),
            kind,
            connected: false,
            subscribed: BTreeSet::new(),
            battery: match kind {
                SimKind::Trainer => 100,
                SimKind::HeartRate => 78,
                SimKind::Cadence => 64,
                SimKind::PowerMeter => 91,
            },
            faults: SimFaults { ack_latency_ms: 120, ..Default::default() },
            next_notify_ms: 0,
            crank_revs: 0.0,
            crank_event_ticks: 0,
            last_whole_rev: 0,
            control_granted: false,
            started: false,
            mode: TrainerMode::Idle,
            max_power_w: 2000,
            max_grade_pct: 20.0,
            applied_power_w: 0.0,
            command_log: Vec::new(),
        }
    }

    fn services(&self) -> Vec<u16> {
        match self.kind {
            SimKind::Trainer => vec![SVC_FTMS, SVC_CYCLING_POWER, SVC_DEVICE_INFO],
            SimKind::HeartRate => vec![SVC_HEART_RATE, SVC_BATTERY, SVC_DEVICE_INFO],
            SimKind::Cadence => vec![SVC_CSC, SVC_BATTERY, SVC_DEVICE_INFO],
            SimKind::PowerMeter => vec![SVC_CYCLING_POWER, SVC_BATTERY, SVC_DEVICE_INFO],
        }
    }

    fn characteristics(&self) -> Vec<u16> {
        let mut c = vec![CHR_MANUFACTURER_NAME, CHR_MODEL_NUMBER, CHR_FIRMWARE_REVISION];
        match self.kind {
            SimKind::Trainer => c.extend([
                CHR_FTMS_FEATURE,
                CHR_INDOOR_BIKE_DATA,
                CHR_SUPPORTED_POWER_RANGE,
                CHR_SUPPORTED_RESISTANCE_RANGE,
                CHR_FTMS_CONTROL_POINT,
                CHR_FTMS_STATUS,
                CHR_CYCLING_POWER_MEASUREMENT,
            ]),
            SimKind::HeartRate => c.extend([CHR_HEART_RATE_MEASUREMENT, CHR_BATTERY_LEVEL]),
            SimKind::Cadence => c.extend([CHR_CSC_MEASUREMENT, CHR_BATTERY_LEVEL]),
            SimKind::PowerMeter => c.extend([CHR_CYCLING_POWER_MEASUREMENT, CHR_BATTERY_LEVEL]),
        }
        c
    }

    fn features(&self) -> FtmsFeatures {
        use ftms::{machine_feature as m, target_feature as t};
        let mut target = t::POWER | t::RESISTANCE | t::INDOOR_BIKE_SIMULATION;
        if self.faults.no_simulation {
            target &= !t::INDOOR_BIKE_SIMULATION;
        }
        FtmsFeatures { machine: m::CADENCE | m::POWER_MEASUREMENT | m::RESISTANCE_LEVEL, target }
    }
}

/// Simulated rider inputs (demo controls).
#[derive(Debug, Clone)]
pub struct SimRider {
    /// Power the rider chooses to produce when the trainer is not in ERG.
    pub effort_w: f64,
    pub cadence_rpm: f64,
    /// When true the rider pushes harder uphill (makes demo rides lively).
    pub responds_to_grade: bool,
    pub hr_bpm: f64,
}

pub struct SimulatorAdapter {
    pub devices: Vec<SimDevice>,
    pub rider: SimRider,
    queue: Vec<(u64, u64, AdapterEvent)>, // (due, seq, event)
    seq: u64,
    next_req: RequestId,
    now_ms: u64,
    last_step_ms: Option<u64>,
    scanning: bool,
    rng: u64,
}

impl Default for SimulatorAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatorAdapter {
    pub fn new() -> Self {
        SimulatorAdapter {
            devices: vec![
                SimDevice::new("sim-trainer", "Ridgeline Sim Trainer", SimKind::Trainer),
                SimDevice::new("sim-hrm", "Sim HR Strap", SimKind::HeartRate),
                SimDevice::new("sim-cadence", "Sim Cadence Sensor", SimKind::Cadence),
                SimDevice::new("sim-power", "Sim Power Pedals", SimKind::PowerMeter),
            ],
            rider: SimRider { effort_w: 180.0, cadence_rpm: 88.0, responds_to_grade: true, hr_bpm: 62.0 },
            queue: Vec::new(),
            seq: 0,
            next_req: 1,
            now_ms: 0,
            last_step_ms: None,
            scanning: false,
            rng: 0x2545F4914F6CDD1D,
        }
    }

    pub fn device_mut(&mut self, id: &str) -> Option<&mut SimDevice> {
        self.devices.iter_mut().find(|d| d.id.0 == id)
    }
    pub fn device(&self, id: &str) -> Option<&SimDevice> {
        self.devices.iter().find(|d| d.id.0 == id)
    }
    pub fn trainer(&self) -> &SimDevice {
        self.devices.iter().find(|d| d.kind == SimKind::Trainer).expect("trainer")
    }

    /// Apply a fault toggle by name. Returns false for unknown fault names.
    pub fn set_fault(&mut self, id: &str, fault: &str, on: bool) -> bool {
        let now = self.now_ms;
        let Some(d) = self.device_mut(id) else { return false };
        match fault {
            "deny_control" => d.faults.deny_control = on,
            "drop_acks" => d.faults.drop_acks = on,
            "fail_writes" => d.faults.fail_writes = on,
            "stale" => d.faults.stale = on,
            "malformed" => d.faults.malformed = on,
            "busy" => d.faults.busy = on,
            "no_simulation" => d.faults.no_simulation = on,
            "slow_acks" => d.faults.ack_latency_ms = if on { 4500 } else { 120 },
            "offline" => {
                d.faults.offline = on;
                if on && d.connected {
                    d.connected = false;
                    d.control_granted = false;
                    d.subscribed.clear();
                    let id = d.id.clone();
                    self.push(now + 10, AdapterEvent::Disconnected { id, reason: Some("link lost (simulated)".into()) });
                }
            }
            "low_power_limit" => d.max_power_w = if on { 300 } else { 2000 },
            "low_grade_limit" => d.max_grade_pct = if on { 6.0 } else { 20.0 },
            "low_battery" => d.battery = if on { 7 } else { 78 },
            _ => return false,
        }
        true
    }

    fn push(&mut self, due: u64, ev: AdapterEvent) {
        self.seq += 1;
        self.queue.push((due, self.seq, ev));
    }

    fn noise(&mut self, amp: f64) -> f64 {
        // xorshift64*: deterministic noise.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545F4914F6CDD1D);
        ((v >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * amp
    }

    fn req(&mut self) -> RequestId {
        self.next_req += 1;
        self.next_req
    }

    fn idx(&self, id: &DeviceId) -> Result<usize, AdapterError> {
        self.devices
            .iter()
            .position(|d| &d.id == id)
            .ok_or_else(|| AdapterError::new(AdapterErrorKind::NotFound, format!("unknown device {id}")))
    }

    fn read_value(d: &SimDevice, ch: u16) -> Option<Vec<u8>> {
        Some(match ch {
            CHR_FTMS_FEATURE if d.kind == SimKind::Trainer => d.features().encode(),
            CHR_SUPPORTED_POWER_RANGE if d.kind == SimKind::Trainer => {
                PowerRange { min_w: 0, max_w: d.max_power_w, inc_w: 1 }.encode()
            }
            CHR_SUPPORTED_RESISTANCE_RANGE if d.kind == SimKind::Trainer => Writer::new().i16(0).i16(1000).u16(10).done(),
            CHR_BATTERY_LEVEL if d.kind != SimKind::Trainer => vec![d.battery],
            CHR_MANUFACTURER_NAME => b"Ridgeline Simulator".to_vec(),
            CHR_MODEL_NUMBER => d.name.as_bytes().to_vec(),
            CHR_FIRMWARE_REVISION => b"sim-1.0".to_vec(),
            _ => return None,
        })
    }

    fn handle_control_write(&mut self, i: usize, data: &[u8]) {
        let now = self.now_ms;
        let latency = self.devices[i].faults.ack_latency_ms;
        let drop = self.devices[i].faults.drop_acks;
        let dev_id = self.devices[i].id.clone();
        let (opcode, result) = match ControlCommand::decode(data) {
            Err(_) => (data.first().copied().unwrap_or(0), ResultCode::OpCodeNotSupported),
            Ok(cmd) => {
                let d = &mut self.devices[i];
                d.command_log.push((now, cmd.clone()));
                let op = cmd.opcode();
                let res = match cmd {
                    ControlCommand::RequestControl => {
                        if d.faults.deny_control {
                            ResultCode::ControlNotPermitted
                        } else {
                            d.control_granted = true;
                            ResultCode::Success
                        }
                    }
                    _ if !d.control_granted => ResultCode::ControlNotPermitted,
                    ControlCommand::Reset => {
                        d.mode = TrainerMode::Idle;
                        d.started = false;
                        ResultCode::Success
                    }
                    ControlCommand::StartOrResume => {
                        d.started = true;
                        ResultCode::Success
                    }
                    ControlCommand::StopOrPause(k) => {
                        d.started = false;
                        if k == StopKind::Stop {
                            d.mode = TrainerMode::Idle;
                        }
                        ResultCode::Success
                    }
                    ControlCommand::SetTargetPower { watts } => {
                        if watts < 0 {
                            ResultCode::InvalidParameter
                        } else {
                            // Saturate at the device limit, as many trainers do.
                            d.mode = TrainerMode::Erg(watts.min(d.max_power_w));
                            ResultCode::Success
                        }
                    }
                    ControlCommand::SetTargetResistance { level } => {
                        if !(0.0..=100.0).contains(&level) {
                            ResultCode::InvalidParameter
                        } else {
                            d.mode = TrainerMode::Resistance(level);
                            ResultCode::Success
                        }
                    }
                    ControlCommand::SetSimulation(mut p) => {
                        if d.faults.no_simulation {
                            ResultCode::OpCodeNotSupported
                        } else {
                            p.grade_percent = p.grade_percent.clamp(-d.max_grade_pct, d.max_grade_pct);
                            d.mode = TrainerMode::Simulation(p);
                            ResultCode::Success
                        }
                    }
                };
                (op, res)
            }
        };
        if !drop {
            let ev = AdapterEvent::Notification {
                id: dev_id,
                characteristic: CHR_FTMS_CONTROL_POINT,
                data: ControlResponse { request_opcode: opcode, result }.encode(),
            };
            self.push(now + latency, ev);
        }
    }

    fn step_physics(&mut self, dt_s: f64) {
        if dt_s <= 0.0 {
            return;
        }
        let cad = (self.rider.cadence_rpm).max(0.0);
        let pedaling = cad >= 5.0;
        let effort = self.rider.effort_w.max(0.0);
        let responds = self.rider.responds_to_grade;
        let mut trainer_power = 0.0;
        for d in self.devices.iter_mut() {
            if d.kind != SimKind::Trainer {
                continue;
            }
            let target = if !pedaling {
                0.0
            } else {
                match (&d.mode, d.control_granted) {
                    (TrainerMode::Erg(w), true) => *w as f64,
                    (TrainerMode::Simulation(p), true) => {
                        let extra = if responds { 14.0 * p.grade_percent } else { 0.0 };
                        (effort + extra).clamp(0.0, d.max_power_w as f64)
                    }
                    (TrainerMode::Resistance(l), true) => (l * cad * 0.04).min(d.max_power_w as f64),
                    _ => effort,
                }
            };
            let k = 1.0 - (-dt_s / 1.2).exp();
            d.applied_power_w += (target - d.applied_power_w) * k;
            trainer_power = d.applied_power_w;
        }
        let hr_target = 55.0 + 0.42 * trainer_power + 0.15 * cad;
        let k = 1.0 - (-dt_s / 25.0).exp();
        self.rider.hr_bpm += (hr_target - self.rider.hr_bpm) * k;
        let now_ticks = ((self.now_ms as f64 / 1000.0) * 1024.0) as u64;
        for d in self.devices.iter_mut() {
            d.crank_revs += cad / 60.0 * dt_s;
            let whole = d.crank_revs.floor() as u64;
            if whole != d.last_whole_rev {
                d.last_whole_rev = whole;
                d.crank_event_ticks = (now_ticks % 65536) as u16;
            }
        }
    }

    fn emit_notifications(&mut self) {
        let now = self.now_ms;
        let n_cad = self.noise(1.2);
        let n_pow = self.noise(4.0);
        let n_hr = self.noise(0.8);
        let cad = (self.rider.cadence_rpm + if self.rider.cadence_rpm > 5.0 { n_cad } else { 0.0 }).max(0.0);
        let hr = self.rider.hr_bpm + n_hr;
        let mut out = Vec::new();
        for d in self.devices.iter_mut() {
            if !d.connected || d.faults.stale || d.faults.offline || now < d.next_notify_ms {
                continue;
            }
            d.next_notify_ms = now + 1000;
            let p = if d.kind == SimKind::Trainer {
                (d.applied_power_w + if d.applied_power_w > 1.0 { n_pow } else { 0.0 }).max(0.0)
            } else {
                0.0
            };
            let crank = ((d.last_whole_rev % 65536) as u16, d.crank_event_ticks);
            let mut packets: Vec<(u16, Vec<u8>)> = Vec::new();
            match d.kind {
                SimKind::Trainer => {
                    let speed = if p > 1.0 { 3.6 * (p / 0.2).cbrt() } else { 0.0 };
                    packets.push((
                        CHR_INDOOR_BIKE_DATA,
                        IndoorBikeData::encode(Some(speed), Some(cad), Some(p.round() as i16), None),
                    ));
                    packets.push((CHR_CYCLING_POWER_MEASUREMENT, CyclingPowerMeasurement::encode(p.round() as i16, Some(crank))));
                }
                SimKind::HeartRate => {
                    packets.push((CHR_HEART_RATE_MEASUREMENT, HeartRateMeasurement::encode_simple(hr.round().clamp(30.0, 230.0) as u8)))
                }
                SimKind::Cadence => packets.push((CHR_CSC_MEASUREMENT, CscMeasurement::encode_crank(crank.0, crank.1))),
                SimKind::PowerMeter => {
                    out.push((d.id.clone(), 0u16, Vec::new())); // placeholder for power meter, filled below
                }
            }
            for (ch, mut data) in packets {
                if !d.subscribed.contains(&ch) {
                    continue;
                }
                if d.faults.malformed && !data.is_empty() {
                    data.pop();
                }
                out.push((d.id.clone(), ch, data));
            }
        }
        // Power meter reads ~3 % lower than the trainer, to make source
        // discrepancies visible in demo mode.
        let trainer_p = self.trainer().applied_power_w;
        let pm_noise = self.noise(3.0);
        let mut final_out = Vec::new();
        for (id, ch, data) in out {
            if ch == 0 {
                let d = self.devices.iter().find(|d| d.id == id).unwrap();
                if !d.subscribed.contains(&CHR_CYCLING_POWER_MEASUREMENT) {
                    continue;
                }
                let p = if trainer_p > 1.0 { (trainer_p * 0.97 + pm_noise).max(0.0) } else { 0.0 };
                let mut data = CyclingPowerMeasurement::encode(p.round() as i16, Some(((d.last_whole_rev % 65536) as u16, d.crank_event_ticks)));
                if d.faults.malformed {
                    data.pop();
                }
                final_out.push((id, CHR_CYCLING_POWER_MEASUREMENT, data));
            } else {
                final_out.push((id, ch, data));
            }
        }
        for (id, ch, data) in final_out {
            self.push(now, AdapterEvent::Notification { id, characteristic: ch, data });
        }
    }
}

impl DeviceAdapter for SimulatorAdapter {
    fn kind(&self) -> &'static str {
        "simulator"
    }

    fn start_scan(&mut self) -> Result<(), AdapterError> {
        self.scanning = true;
        let now = self.now_ms;
        self.push(now, AdapterEvent::ScanStarted);
        let ads: Vec<_> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.faults.offline)
            .map(|(i, d)| {
                (
                    now + 150 + 100 * i as u64,
                    Advertisement { id: d.id.clone(), name: Some(d.name.clone()), rssi: Some(-50 - 4 * i as i16), services: d.services() },
                )
            })
            .collect();
        for (due, ad) in ads {
            self.push(due, AdapterEvent::Discovered(ad));
        }
        Ok(())
    }

    fn stop_scan(&mut self) {
        if self.scanning {
            self.scanning = false;
            let now = self.now_ms;
            self.push(now, AdapterEvent::ScanStopped);
        }
    }

    fn connect(&mut self, id: &DeviceId) -> Result<(), AdapterError> {
        let i = self.idx(id)?;
        let now = self.now_ms;
        let d = &self.devices[i];
        let ev = if d.faults.busy {
            (now + 600, AdapterEvent::ConnectFailed {
                id: id.clone(),
                error: AdapterError::new(AdapterErrorKind::Busy, "device is connected to another app (simulated)"),
            })
        } else if d.faults.offline {
            (now + 2000, AdapterEvent::ConnectFailed { id: id.clone(), error: AdapterError::new(AdapterErrorKind::Timeout, "device did not respond (asleep or out of range)") })
        } else {
            (now + 400, AdapterEvent::Connected { id: id.clone(), services: d.services(), characteristics: d.characteristics() })
        };
        if matches!(ev.1, AdapterEvent::Connected { .. }) {
            let d = &mut self.devices[i];
            d.connected = true;
            d.subscribed.clear();
            d.control_granted = false;
            d.next_notify_ms = now + 600;
        }
        self.push(ev.0, ev.1);
        Ok(())
    }

    fn disconnect(&mut self, id: &DeviceId) {
        if let Ok(i) = self.idx(id) {
            let now = self.now_ms;
            let d = &mut self.devices[i];
            if d.connected {
                d.connected = false;
                d.control_granted = false;
                d.started = false;
                d.subscribed.clear();
                self.push(now + 50, AdapterEvent::Disconnected { id: id.clone(), reason: Some("disconnected by app".into()) });
            }
        }
    }

    fn read(&mut self, id: &DeviceId, characteristic: u16) -> Result<RequestId, AdapterError> {
        let i = self.idx(id)?;
        let req = self.req();
        let now = self.now_ms;
        let d = &self.devices[i];
        let result = if !d.connected {
            Err(AdapterError::new(AdapterErrorKind::Disconnected, "not connected"))
        } else {
            Self::read_value(d, characteristic).ok_or_else(|| AdapterError::new(AdapterErrorKind::Unsupported, "characteristic not readable"))
        };
        self.push(now + 60, AdapterEvent::ReadResult { id: id.clone(), req, characteristic, result });
        Ok(req)
    }

    fn subscribe(&mut self, id: &DeviceId, characteristic: u16) -> Result<RequestId, AdapterError> {
        let i = self.idx(id)?;
        let req = self.req();
        let now = self.now_ms;
        let result = if !self.devices[i].connected {
            Err(AdapterError::new(AdapterErrorKind::Disconnected, "not connected"))
        } else if !self.devices[i].characteristics().contains(&characteristic) {
            Err(AdapterError::new(AdapterErrorKind::Unsupported, "characteristic not present"))
        } else {
            self.devices[i].subscribed.insert(characteristic);
            Ok(())
        };
        self.push(now + 40, AdapterEvent::SubscribeResult { id: id.clone(), req, characteristic, result });
        Ok(req)
    }

    fn write(&mut self, id: &DeviceId, characteristic: u16, data: &[u8], _with_response: bool) -> Result<RequestId, AdapterError> {
        let i = self.idx(id)?;
        let req = self.req();
        let now = self.now_ms;
        let d = &self.devices[i];
        if !d.connected {
            self.push(now + 20, AdapterEvent::WriteComplete { id: id.clone(), req, characteristic, result: Err(AdapterError::new(AdapterErrorKind::Disconnected, "not connected")) });
            return Ok(req);
        }
        if d.faults.fail_writes {
            self.push(now + 30, AdapterEvent::WriteComplete { id: id.clone(), req, characteristic, result: Err(AdapterError::new(AdapterErrorKind::Other, "GATT write failed (simulated)")) });
            return Ok(req);
        }
        if characteristic == CHR_FTMS_CONTROL_POINT && d.kind == SimKind::Trainer {
            if !d.subscribed.contains(&CHR_FTMS_CONTROL_POINT) {
                // Real devices reject control point writes without indications enabled.
                self.push(now + 30, AdapterEvent::WriteComplete { id: id.clone(), req, characteristic, result: Err(AdapterError::new(AdapterErrorKind::Other, "CCCD improperly configured")) });
                return Ok(req);
            }
            self.push(now + 30, AdapterEvent::WriteComplete { id: id.clone(), req, characteristic, result: Ok(()) });
            self.handle_control_write(i, data);
        } else {
            self.push(now + 30, AdapterEvent::WriteComplete { id: id.clone(), req, characteristic, result: Err(AdapterError::new(AdapterErrorKind::Unsupported, "not writable")) });
        }
        Ok(req)
    }

    fn poll(&mut self, now_ms: u64) -> Vec<AdapterEvent> {
        if let Some(last) = self.last_step_ms {
            // Step physics in bounded increments for stability after long gaps.
            let mut t = last;
            while t < now_ms {
                let step = (now_ms - t).min(250);
                t += step;
                self.now_ms = t;
                self.step_physics(step as f64 / 1000.0);
            }
        }
        self.last_step_ms = Some(now_ms);
        self.now_ms = now_ms;
        self.emit_notifications();
        self.queue.sort_by_key(|(due, seq, _)| (*due, *seq));
        let split = self.queue.iter().position(|(due, _, _)| *due > now_ms).unwrap_or(self.queue.len());
        self.queue.drain(..split).map(|(_, _, e)| e).collect()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sim: &mut SimulatorAdapter, from: u64, to: u64) -> Vec<AdapterEvent> {
        let mut all = Vec::new();
        let mut t = from;
        while t <= to {
            all.extend(sim.poll(t));
            t += 50;
        }
        all
    }

    #[test]
    fn scan_connect_subscribe_and_notify() {
        let mut sim = SimulatorAdapter::new();
        sim.poll(0);
        sim.start_scan().unwrap();
        let ev = run(&mut sim, 0, 1000);
        assert_eq!(ev.iter().filter(|e| matches!(e, AdapterEvent::Discovered(_))).count(), 4);
        let id = DeviceId("sim-hrm".into());
        sim.connect(&id).unwrap();
        let ev = run(&mut sim, 1000, 1500);
        assert!(ev.iter().any(|e| matches!(e, AdapterEvent::Connected { .. })));
        sim.subscribe(&id, CHR_HEART_RATE_MEASUREMENT).unwrap();
        let ev = run(&mut sim, 1500, 4000);
        let n = ev.iter().filter(|e| matches!(e, AdapterEvent::Notification { characteristic: CHR_HEART_RATE_MEASUREMENT, .. })).count();
        assert!((2..=3).contains(&n), "{n}");
    }

    #[test]
    fn control_requires_permission_and_honours_denial() {
        let mut sim = SimulatorAdapter::new();
        sim.poll(0);
        let id = DeviceId("sim-trainer".into());
        sim.connect(&id).unwrap();
        run(&mut sim, 0, 500);
        sim.subscribe(&id, CHR_FTMS_CONTROL_POINT).unwrap();
        run(&mut sim, 500, 600);
        sim.write(&id, CHR_FTMS_CONTROL_POINT, &ControlCommand::SetTargetPower { watts: 200 }.encode().unwrap(), true).unwrap();
        let ev = run(&mut sim, 600, 1000);
        let resp = ev.iter().find_map(|e| match e {
            AdapterEvent::Notification { characteristic: CHR_FTMS_CONTROL_POINT, data, .. } => Some(ControlResponse::parse(data).unwrap()),
            _ => None,
        });
        assert_eq!(resp.unwrap().result, ResultCode::ControlNotPermitted);
        sim.set_fault("sim-trainer", "deny_control", true);
        sim.write(&id, CHR_FTMS_CONTROL_POINT, &[0x00], true).unwrap();
        let ev = run(&mut sim, 1000, 1400);
        let resp = ev.iter().find_map(|e| match e {
            AdapterEvent::Notification { characteristic: CHR_FTMS_CONTROL_POINT, data, .. } => Some(ControlResponse::parse(data).unwrap()),
            _ => None,
        });
        assert_eq!(resp.unwrap().result, ResultCode::ControlNotPermitted);
        assert!(!sim.trainer().control_granted);
    }

    #[test]
    fn grade_saturates_at_device_limit() {
        let mut sim = SimulatorAdapter::new();
        sim.poll(0);
        let id = DeviceId("sim-trainer".into());
        sim.connect(&id).unwrap();
        run(&mut sim, 0, 500);
        sim.subscribe(&id, CHR_FTMS_CONTROL_POINT).unwrap();
        run(&mut sim, 500, 600);
        sim.set_fault("sim-trainer", "low_grade_limit", true);
        sim.write(&id, CHR_FTMS_CONTROL_POINT, &[0x00], true).unwrap();
        let p = SimParams { wind_speed_mps: 0.0, grade_percent: 12.0, crr: 0.004, cw_kg_per_m: 0.51 };
        sim.write(&id, CHR_FTMS_CONTROL_POINT, &ControlCommand::SetSimulation(p).encode().unwrap(), true).unwrap();
        run(&mut sim, 600, 1000);
        match &sim.trainer().mode {
            TrainerMode::Simulation(p) => assert!((p.grade_percent - 6.0).abs() < 1e-9),
            m => panic!("{m:?}"),
        }
    }
}
