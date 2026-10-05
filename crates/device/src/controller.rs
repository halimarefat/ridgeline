//! FTMS trainer controller: serialized control-point command queue.
//!
//! Rules implemented here (spec §6):
//! * one command in flight; each response is matched to its request op code;
//! * transport write success is tracked separately from machine acceptance;
//! * bounded acknowledgment timeout (3 s); a timed-out command leaves the
//!   state *uncertain* and start/stop are never retried blindly;
//! * only the newest pending target is kept (obsolete targets are dropped) and
//!   targets are rate-limited (≤ 1 Hz by default);
//! * every command carries the session generation and is discarded if the
//!   generation changed before dispatch;
//! * control is requested only after the control point indication is
//!   subscribed, and a lost/reconnected trainer must be explicitly re-granted;
//! * stop/pause discards pending targets, sends a low-load target and then a
//!   stop/pause; success is reported only on acknowledgment.

use crate::adapter::{AdapterError, DeviceAdapter, DeviceId, RequestId};
use crate::ftms::{ControlCommand, ControlResponse, FtmsFeatures, MachineStatus, PowerRange, ResistanceRange, ResultCode, StopKind};
use crate::gatt::CHR_FTMS_CONTROL_POINT;
use rl_json::{json_enum, ToJson, Value};
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub struct FtmsCaps {
    pub features: FtmsFeatures,
    pub power_range: Option<PowerRange>,
    pub resistance_range: Option<ResistanceRange>,
}

impl FtmsCaps {
    pub fn to_json(&self) -> Value {
        Value::obj([
            ("erg", self.features.supports_power_target().into()),
            ("simulation", self.features.supports_simulation().into()),
            ("resistance", self.features.supports_resistance_target().into()),
            ("spin_down", self.features.supports_spin_down().into()),
            ("reports_power", self.features.reports_power().into()),
            ("reports_cadence", self.features.reports_cadence().into()),
            ("power_min", self.power_range.map(|p| p.min_w).into()),
            ("power_max", self.power_range.map(|p| p.max_w).into()),
            ("power_inc", self.power_range.map(|p| p.inc_w).into()),
            ("resistance_min", self.resistance_range.map(|r| r.min).into()),
            ("resistance_max", self.resistance_range.map(|r| r.max).into()),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    NoTrainer,
    /// Trainer ready; control not requested (or must be re-requested).
    NotControlled,
    Requesting,
    Controlled,
    Denied,
    /// Trainer revoked control or disconnected while controlled.
    Lost,
    /// A control request timed out; the trainer's state is not known.
    Uncertain,
}
json_enum!(ControlState {
    NoTrainer = "no_trainer",
    NotControlled = "not_controlled",
    Requesting = "requesting",
    Controlled = "controlled",
    Denied = "denied",
    Lost = "lost",
    Uncertain = "uncertain",
});

#[derive(Debug, Clone, PartialEq)]
pub enum ControlEventKind {
    Granted,
    Denied,
    Lost,
    Acked,
    Rejected(ResultCode),
    TimedOut,
    WriteFailed,
    Dropped,
    Unsupported,
    StopConfirmed,
    StopUnconfirmed,
    UserStoppedOnTrainer,
    SafetyKey,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlEvent {
    pub kind: ControlEventKind,
    pub cmd: Option<ControlCommand>,
    pub generation: u64,
    pub at_ms: u64,
    pub detail: String,
}

#[derive(Debug, Clone)]
struct Queued {
    cmd: ControlCommand,
    generation: u64,
    enqueued_ms: u64,
    stopping: bool,
}

#[derive(Debug, Clone)]
struct InFlight {
    q: Queued,
    sent_ms: u64,
    write_req: RequestId,
    write_ok: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub at_ms: u64,
    pub generation: u64,
    pub command: String,
    pub outcome: String,
    pub latency_ms: Option<u64>,
}

pub fn describe(cmd: &ControlCommand) -> String {
    match cmd {
        ControlCommand::RequestControl => "request control".into(),
        ControlCommand::Reset => "reset".into(),
        ControlCommand::StartOrResume => "start/resume".into(),
        ControlCommand::StopOrPause(StopKind::Stop) => "stop".into(),
        ControlCommand::StopOrPause(StopKind::Pause) => "pause".into(),
        ControlCommand::SetTargetPower { watts } => format!("ERG {watts} W"),
        ControlCommand::SetTargetResistance { level } => format!("resistance {level:.1}"),
        ControlCommand::SetSimulation(p) => format!("simulation {:+.2}% (crr {:.4}, cw {:.2})", p.grade_percent, p.crr, p.cw_kg_per_m),
    }
}

pub struct TrainerController {
    pub adapter_index: Option<usize>,
    pub device: Option<DeviceId>,
    pub device_key: Option<String>,
    pub caps: Option<FtmsCaps>,
    pub state: ControlState,
    generation: u64,
    ops: VecDeque<Queued>,
    target: Option<Queued>,
    in_flight: Option<InFlight>,
    last_target_dispatch_ms: Option<u64>,
    pub min_target_interval_ms: u64,
    pub ack_timeout_ms: u64,
    /// Last target the machine acknowledged (commanded state).
    pub acked_target: Option<ControlCommand>,
    /// Last target written (may be unacknowledged).
    pub sent_target: Option<ControlCommand>,
    /// Last machine status notification (reported state).
    pub reported: Option<MachineStatus>,
    /// None until a stop path completes: Some(true) acknowledged, Some(false) not confirmed.
    pub stop_confirmed: Option<bool>,
    stopping: bool,
    log: VecDeque<LogEntry>,
    events: Vec<ControlEvent>,
    pub commands_sent: u64,
    /// Time between queuing and dispatch of the last command (ms).
    pub last_queue_delay_ms: Option<u64>,
}

impl Default for TrainerController {
    fn default() -> Self {
        Self::new()
    }
}

impl TrainerController {
    pub fn new() -> Self {
        TrainerController {
            adapter_index: None,
            device: None,
            device_key: None,
            caps: None,
            state: ControlState::NoTrainer,
            generation: 1,
            ops: VecDeque::new(),
            target: None,
            in_flight: None,
            last_target_dispatch_ms: None,
            min_target_interval_ms: 1000,
            ack_timeout_ms: 3000,
            acked_target: None,
            sent_target: None,
            reported: None,
            stop_confirmed: None,
            stopping: false,
            log: VecDeque::new(),
            events: Vec::new(),
            commands_sent: 0,
            last_queue_delay_ms: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Start a new control generation: everything queued under an older
    /// generation can no longer take effect.
    pub fn new_generation(&mut self) -> u64 {
        self.generation += 1;
        self.target = None;
        self.ops.clear();
        self.generation
    }

    fn emit(&mut self, kind: ControlEventKind, cmd: Option<ControlCommand>, now: u64, detail: impl Into<String>) {
        self.events.push(ControlEvent { kind, cmd, generation: self.generation, at_ms: now, detail: detail.into() });
    }

    fn log(&mut self, now: u64, gen: u64, cmd: &ControlCommand, outcome: &str, latency: Option<u64>) {
        self.log.push_back(LogEntry { at_ms: now, generation: gen, command: describe(cmd), outcome: outcome.into(), latency_ms: latency });
        while self.log.len() > 300 {
            self.log.pop_front();
        }
    }

    pub fn drain_events(&mut self) -> Vec<ControlEvent> {
        std::mem::take(&mut self.events)
    }

    /// Trainer is ready (connected, features read, control point subscribed).
    pub fn attach(&mut self, adapter_index: usize, id: DeviceId, key: String, caps: FtmsCaps) {
        let same = self.device_key.as_deref() == Some(key.as_str());
        self.adapter_index = Some(adapter_index);
        self.device = Some(id);
        self.device_key = Some(key);
        self.caps = Some(caps);
        if !same || matches!(self.state, ControlState::NoTrainer) {
            self.state = ControlState::NotControlled;
        } else if matches!(self.state, ControlState::Lost | ControlState::Controlled | ControlState::Requesting | ControlState::Uncertain) {
            // Reconnected: permission is never assumed to persist.
            self.state = ControlState::NotControlled;
        }
        self.in_flight = None;
        self.ops.clear();
        self.target = None;
    }

    /// Trainer disconnected (or was un-assigned).
    pub fn detach(&mut self, now: u64, reason: &str) {
        if let Some(f) = self.in_flight.take() {
            let gen = f.q.generation;
            self.log(now, gen, &f.q.cmd, "unknown (disconnected before acknowledgment)", None);
            if f.q.stopping {
                self.stop_confirmed = Some(false);
                self.emit(ControlEventKind::StopUnconfirmed, Some(f.q.cmd.clone()), now, "Trainer disconnected before confirming the stop. It may keep its last resistance; stop pedalling if needed.");
            }
        }
        let had = !matches!(self.state, ControlState::NoTrainer | ControlState::NotControlled | ControlState::Denied);
        self.ops.clear();
        self.target = None;
        if self.stopping && self.stop_confirmed.is_none() {
            self.stop_confirmed = Some(false);
        }
        if had {
            self.emit(ControlEventKind::Lost, None, now, format!("Trainer control lost: {reason}"));
        }
        self.state = if had { ControlState::Lost } else { ControlState::NoTrainer };
    }

    pub fn unassign(&mut self) {
        *self = TrainerController { log: std::mem::take(&mut self.log), generation: self.generation + 1, ..TrainerController::new() };
    }

    pub fn is_controlled(&self) -> bool {
        self.state == ControlState::Controlled
    }

    pub fn request_control(&mut self, now: u64) -> Result<(), String> {
        if self.device.is_none() {
            return Err("No trainer is connected.".into());
        }
        if self.state == ControlState::Requesting {
            return Ok(());
        }
        self.state = ControlState::Requesting;
        self.stopping = false;
        self.stop_confirmed = None;
        let gen = self.generation;
        self.ops.push_back(Queued { cmd: ControlCommand::RequestControl, generation: gen, enqueued_ms: now, stopping: false });
        Ok(())
    }

    pub fn start(&mut self, now: u64) {
        let gen = self.generation;
        self.stopping = false;
        self.stop_confirmed = None;
        self.ops.push_back(Queued { cmd: ControlCommand::StartOrResume, generation: gen, enqueued_ms: now, stopping: false });
    }

    fn supported(&self, cmd: &ControlCommand) -> bool {
        let Some(c) = &self.caps else { return false };
        match cmd {
            ControlCommand::SetTargetPower { .. } => c.features.supports_power_target(),
            ControlCommand::SetSimulation(_) => c.features.supports_simulation(),
            ControlCommand::SetTargetResistance { .. } => c.features.supports_resistance_target(),
            _ => true,
        }
    }

    /// Clamp a target to the advertised device range. Returns the command to
    /// send and whether it was saturated.
    pub fn clamp_target(&self, cmd: ControlCommand) -> (ControlCommand, bool) {
        match (&cmd, self.caps.as_ref()) {
            (ControlCommand::SetTargetPower { watts }, Some(FtmsCaps { power_range: Some(r), .. })) => {
                let lo = r.min_w.max(0);
                let w = (*watts).clamp(lo, r.max_w);
                (ControlCommand::SetTargetPower { watts: w }, w != *watts)
            }
            (ControlCommand::SetTargetPower { watts }, _) => {
                let w = (*watts).clamp(0, 2000);
                (ControlCommand::SetTargetPower { watts: w }, w != *watts)
            }
            (ControlCommand::SetTargetResistance { level }, Some(FtmsCaps { resistance_range: Some(r), .. })) => {
                let l = level.clamp(r.min, r.max);
                (ControlCommand::SetTargetResistance { level: l }, (l - level).abs() > 1e-9)
            }
            _ => (cmd, false),
        }
    }

    /// Queue a target (ERG watts, simulation parameters or resistance). Only
    /// the newest pending target is kept. Ignored unless control is held.
    pub fn set_target(&mut self, cmd: ControlCommand, now: u64) -> Result<bool, String> {
        if !cmd.is_target() {
            return Err("not a target command".into());
        }
        if self.state != ControlState::Controlled {
            return Err("Trainer control is not active.".into());
        }
        if self.stopping {
            return Err("Trainer is stopping; targets are discarded.".into());
        }
        if !self.supported(&cmd) {
            self.emit(ControlEventKind::Unsupported, Some(cmd.clone()), now, "This trainer does not advertise support for that control mode.");
            return Err("Mode not supported by this trainer.".into());
        }
        let (cmd, saturated) = self.clamp_target(cmd);
        if let Some(old) = self.target.take() {
            self.log(now, old.generation, &old.cmd, "dropped (superseded before dispatch)", None);
        }
        self.target = Some(Queued { cmd, generation: self.generation, enqueued_ms: now, stopping: false });
        Ok(saturated)
    }

    /// Enter the stopping path immediately: discard pending targets, then
    /// send a low-load target and stop/pause. `low_load` is mode-specific.
    pub fn stop_path(&mut self, kind: StopKind, low_load: Option<ControlCommand>, now: u64) {
        if let Some(t) = self.target.take() {
            self.log(now, t.generation, &t.cmd, "dropped (stop requested)", None);
        }
        self.ops.retain(|q| !q.cmd.is_target() && !matches!(q.cmd, ControlCommand::StartOrResume));
        self.stopping = true;
        self.stop_confirmed = None;
        if self.state != ControlState::Controlled {
            // Without control (or connection) nothing can be confirmed.
            self.stop_confirmed = Some(false);
            self.emit(ControlEventKind::StopUnconfirmed, None, now, "Trainer is not under control; the app cannot confirm the trainer's resistance. Stop pedalling if resistance remains.");
            return;
        }
        let gen = self.generation;
        if let Some(l) = low_load {
            if self.supported(&l) {
                let (l, _) = self.clamp_target(l);
                self.ops.push_front(Queued { cmd: ControlCommand::StopOrPause(kind), generation: gen, enqueued_ms: now, stopping: true });
                self.ops.push_front(Queued { cmd: l, generation: gen, enqueued_ms: now, stopping: true });
                return;
            }
        }
        self.ops.push_front(Queued { cmd: ControlCommand::StopOrPause(kind), generation: gen, enqueued_ms: now, stopping: true });
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }
    pub fn has_pending(&self) -> bool {
        self.in_flight.is_some() || !self.ops.is_empty() || self.target.is_some()
    }

    /// Dispatch the next command if nothing is in flight; handle timeouts.
    pub fn tick(&mut self, now: u64, adapter: &mut dyn DeviceAdapter) {
        if let Some(f) = &self.in_flight {
            if now.saturating_sub(f.sent_ms) > self.ack_timeout_ms {
                let f = self.in_flight.take().unwrap();
                let gen = f.q.generation;
                let outcome = if f.write_ok == Some(true) { "timed out waiting for acknowledgment" } else { "timed out (write not confirmed)" };
                self.log(now, gen, &f.q.cmd, outcome, None);
                self.emit(ControlEventKind::TimedOut, Some(f.q.cmd.clone()), now, format!("No acknowledgment for {} within {} s.", describe(&f.q.cmd), self.ack_timeout_ms / 1000));
                match f.q.cmd {
                    ControlCommand::RequestControl => self.state = ControlState::Uncertain,
                    _ if f.q.stopping => {
                        if matches!(f.q.cmd, ControlCommand::StopOrPause(_)) {
                            self.stop_confirmed = Some(false);
                            self.emit(ControlEventKind::StopUnconfirmed, Some(f.q.cmd.clone()), now, "The trainer did not confirm the stop. It may keep its last resistance; stop pedalling if needed.");
                        }
                    }
                    _ => {}
                }
            } else {
                return;
            }
        }
        let Some(id) = self.device.clone() else { return };
        // Next op (skipping stale generations), else the latest target.
        let next = loop {
            match self.ops.pop_front() {
                Some(q) if q.generation != self.generation => {
                    self.log(now, q.generation, &q.cmd, "dropped (previous session generation)", None);
                    continue;
                }
                Some(q) => break Some(q),
                None => break None,
            }
        };
        let next = match next {
            Some(q) => Some(q),
            None => {
                let ready = self.state == ControlState::Controlled
                    && self.last_target_dispatch_ms.map(|t| now.saturating_sub(t) >= self.min_target_interval_ms).unwrap_or(true);
                match self.target.take() {
                    Some(q) if q.generation != self.generation => {
                        self.log(now, q.generation, &q.cmd, "dropped (previous session generation)", None);
                        None
                    }
                    Some(q) if ready => {
                        self.last_target_dispatch_ms = Some(now);
                        Some(q)
                    }
                    other => {
                        self.target = other;
                        None
                    }
                }
            }
        };
        let Some(q) = next else { return };
        if q.cmd.is_target() && self.state != ControlState::Controlled {
            self.log(now, q.generation, &q.cmd, "dropped (control not held)", None);
            return;
        }
        let bytes = match q.cmd.encode() {
            Ok(b) => b,
            Err(e) => {
                self.log(now, q.generation, &q.cmd, &format!("rejected locally: {e}"), None);
                self.emit(ControlEventKind::Dropped, Some(q.cmd.clone()), now, e.to_string());
                return;
            }
        };
        match adapter.write(&id, CHR_FTMS_CONTROL_POINT, &bytes, true) {
            Ok(req) => {
                self.commands_sent += 1;
                self.last_queue_delay_ms = Some(now.saturating_sub(q.enqueued_ms));
                if q.cmd.is_target() {
                    self.sent_target = Some(q.cmd.clone());
                }
                self.in_flight = Some(InFlight { q, sent_ms: now, write_req: req, write_ok: None });
            }
            Err(e) => {
                self.log(now, q.generation, &q.cmd, &format!("write failed: {}", e.message), None);
                self.emit(ControlEventKind::WriteFailed, Some(q.cmd.clone()), now, e.message);
            }
        }
    }

    pub fn on_write_complete(&mut self, req: RequestId, result: &Result<(), AdapterError>, now: u64) {
        let Some(f) = self.in_flight.as_mut() else { return };
        if f.write_req != req {
            return;
        }
        match result {
            Ok(()) => f.write_ok = Some(true),
            Err(e) => {
                let f = self.in_flight.take().unwrap();
                let gen = f.q.generation;
                self.log(now, gen, &f.q.cmd, &format!("write failed: {}", e.message), None);
                self.emit(ControlEventKind::WriteFailed, Some(f.q.cmd.clone()), now, e.message.clone());
                if matches!(f.q.cmd, ControlCommand::RequestControl) {
                    self.state = ControlState::NotControlled;
                }
                if f.q.stopping && matches!(f.q.cmd, ControlCommand::StopOrPause(_)) {
                    self.stop_confirmed = Some(false);
                    self.emit(ControlEventKind::StopUnconfirmed, Some(f.q.cmd.clone()), now, "Stop command could not be written. Stop pedalling if resistance remains.");
                }
            }
        }
    }

    /// Control point indication.
    pub fn on_indication(&mut self, data: &[u8], now: u64) {
        let resp = match ControlResponse::parse(data) {
            Ok(r) => r,
            Err(e) => {
                self.log.push_back(LogEntry { at_ms: now, generation: self.generation, command: "indication".into(), outcome: format!("malformed: {e}"), latency_ms: None });
                return;
            }
        };
        let matches = self.in_flight.as_ref().map(|f| f.q.cmd.opcode() == resp.request_opcode).unwrap_or(false);
        if !matches {
            self.log.push_back(LogEntry {
                at_ms: now,
                generation: self.generation,
                command: format!("response to op {:#04x}", resp.request_opcode),
                outcome: format!("unmatched ({}), ignored", resp.result.label()),
                latency_ms: None,
            });
            return;
        }
        let f = self.in_flight.take().unwrap();
        let latency = now.saturating_sub(f.sent_ms);
        let gen = f.q.generation;
        self.log(now, gen, &f.q.cmd, resp.result.label(), Some(latency));
        let cmd = f.q.cmd.clone();
        match (&cmd, resp.result) {
            (ControlCommand::RequestControl, ResultCode::Success) => {
                self.state = ControlState::Controlled;
                self.emit(ControlEventKind::Granted, Some(cmd), now, "Trainer granted control.");
            }
            (ControlCommand::RequestControl, code) => {
                self.state = ControlState::Denied;
                self.emit(ControlEventKind::Denied, Some(cmd), now, format!("Trainer refused control ({}). Another app may be controlling it.", code.label()));
            }
            (c, ResultCode::Success) => {
                if c.is_target() {
                    self.acked_target = Some(c.clone());
                }
                if let ControlCommand::StopOrPause(_) = c {
                    if f.q.stopping {
                        self.stop_confirmed = Some(true);
                        self.emit(ControlEventKind::StopConfirmed, Some(c.clone()), now, "Trainer acknowledged the stop.");
                    }
                }
                self.emit(ControlEventKind::Acked, Some(cmd), now, format!("acknowledged in {latency} ms"));
            }
            (c, ResultCode::ControlNotPermitted) => {
                self.state = ControlState::Lost;
                self.target = None;
                if f.q.stopping {
                    self.stop_confirmed = Some(false);
                }
                self.emit(ControlEventKind::Lost, Some(c.clone()), now, "Trainer reports control is not permitted.");
            }
            (c, code) => {
                if f.q.stopping && matches!(c, ControlCommand::StopOrPause(_)) {
                    self.stop_confirmed = Some(false);
                    self.emit(ControlEventKind::StopUnconfirmed, Some(c.clone()), now, format!("Trainer rejected stop ({}).", code.label()));
                }
                let msg = format!("Trainer rejected {} ({}).", describe(c), code.label());
                self.emit(ControlEventKind::Rejected(code), Some(cmd.clone()), now, msg);
            }
        }
    }

    pub fn on_machine_status(&mut self, data: &[u8], now: u64) {
        let Ok(st) = MachineStatus::parse(data) else { return };
        match &st {
            MachineStatus::ControlPermissionLost => {
                if self.state == ControlState::Controlled || self.state == ControlState::Requesting {
                    self.state = ControlState::Lost;
                    self.target = None;
                    self.emit(ControlEventKind::Lost, None, now, "Trainer reported that control permission was lost.");
                }
            }
            MachineStatus::StoppedOrPausedByUser(_) => self.emit(ControlEventKind::UserStoppedOnTrainer, None, now, "Trainer reports it was stopped or paused on the device."),
            MachineStatus::StoppedBySafetyKey => self.emit(ControlEventKind::SafetyKey, None, now, "Trainer stopped by safety key."),
            _ => {}
        }
        self.reported = Some(st);
    }

    pub fn log_json(&self, max: usize) -> Value {
        Value::Arr(
            self.log
                .iter()
                .rev()
                .take(max)
                .map(|e| {
                    Value::obj([
                        ("at_ms", e.at_ms.into()),
                        ("generation", e.generation.into()),
                        ("command", e.command.clone().into()),
                        ("outcome", e.outcome.clone().into()),
                        ("latency_ms", e.latency_ms.into()),
                    ])
                })
                .collect(),
        )
    }

    pub fn to_json(&self) -> Value {
        Value::obj([
            ("state", self.state.to_json()),
            ("device", self.device_key.clone().into()),
            ("caps", self.caps.as_ref().map(|c| c.to_json()).unwrap_or(Value::Null)),
            ("generation", self.generation.into()),
            ("in_flight", self.in_flight.as_ref().map(|f| describe(&f.q.cmd)).into()),
            ("pending_target", self.target.as_ref().map(|q| describe(&q.cmd)).into()),
            ("sent_target", self.sent_target.as_ref().map(describe).into()),
            ("acked_target", self.acked_target.as_ref().map(describe).into()),
            ("stopping", self.stopping.into()),
            ("stop_confirmed", self.stop_confirmed.into()),
            ("commands_sent", self.commands_sent.into()),
            ("last_queue_delay_ms", self.last_queue_delay_ms.into()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::AdapterEvent;
    use crate::ftms::SimParams;
    use crate::simulator::{SimulatorAdapter, TrainerMode};

    struct Rig {
        sim: SimulatorAdapter,
        c: TrainerController,
        t: u64,
    }

    impl Rig {
        fn new() -> Rig {
            let mut sim = SimulatorAdapter::new();
            sim.poll(0);
            let id = DeviceId("sim-trainer".into());
            sim.connect(&id).unwrap();
            let mut r = Rig { sim, c: TrainerController::new(), t: 0 };
            r.run(500);
            r.sim.subscribe(&id, CHR_FTMS_CONTROL_POINT).unwrap();
            r.run(200);
            let caps = FtmsCaps { features: crate::ftms::FtmsFeatures { machine: 0, target: crate::ftms::target_feature::POWER | crate::ftms::target_feature::INDOOR_BIKE_SIMULATION }, power_range: Some(PowerRange { min_w: 0, max_w: 1500, inc_w: 1 }), resistance_range: None };
            r.c.attach(0, id, "sim:sim-trainer".into(), caps);
            r
        }
        fn run(&mut self, ms: u64) {
            let end = self.t + ms;
            while self.t < end {
                self.t += 50;
                self.c.tick(self.t, &mut self.sim);
                for e in self.sim.poll(self.t) {
                    match e {
                        AdapterEvent::Notification { characteristic: CHR_FTMS_CONTROL_POINT, data, .. } => self.c.on_indication(&data, self.t),
                        AdapterEvent::WriteComplete { req, result, .. } => self.c.on_write_complete(req, &result, self.t),
                        AdapterEvent::Disconnected { .. } => self.c.detach(self.t, "link lost"),
                        _ => {}
                    }
                }
            }
        }
        fn sim_mode(&self) -> TrainerMode {
            self.sim.trainer().mode.clone()
        }
    }

    #[test]
    fn request_control_then_erg_target() {
        let mut r = Rig::new();
        assert!(r.c.set_target(ControlCommand::SetTargetPower { watts: 200 }, r.t).is_err(), "no target before control");
        r.c.request_control(r.t).unwrap();
        r.run(500);
        assert_eq!(r.c.state, ControlState::Controlled);
        r.c.start(r.t);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 200 }, r.t).unwrap();
        r.run(1000);
        assert_eq!(r.sim_mode(), TrainerMode::Erg(200));
        assert_eq!(r.c.acked_target, Some(ControlCommand::SetTargetPower { watts: 200 }));
    }

    #[test]
    fn denied_control_is_reported() {
        let mut r = Rig::new();
        r.sim.set_fault("sim-trainer", "deny_control", true);
        r.c.request_control(r.t).unwrap();
        r.run(500);
        assert_eq!(r.c.state, ControlState::Denied);
        assert!(r.c.drain_events().iter().any(|e| e.kind == ControlEventKind::Denied));
    }

    #[test]
    fn newest_target_wins_and_rate_limited() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        let sent_before = r.c.commands_sent;
        for w in [150, 160, 170, 180, 190] {
            r.c.set_target(ControlCommand::SetTargetPower { watts: w }, r.t).unwrap();
            r.run(100);
        }
        r.run(1500);
        // 5 targets over 0.5 s at ≤1 Hz: at most 2 dispatched, and the last one wins.
        assert!(r.c.commands_sent - sent_before <= 2, "{}", r.c.commands_sent - sent_before);
        assert_eq!(r.sim_mode(), TrainerMode::Erg(190));
    }

    #[test]
    fn timeouts_make_state_uncertain_without_blind_retry() {
        let mut r = Rig::new();
        r.sim.set_fault("sim-trainer", "drop_acks", true);
        r.c.request_control(r.t).unwrap();
        r.run(4000);
        assert_eq!(r.c.state, ControlState::Uncertain);
        let n = r.c.commands_sent;
        r.run(5000);
        assert_eq!(r.c.commands_sent, n, "no automatic retry");
    }

    #[test]
    fn stop_path_discards_targets_and_confirms() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 300 }, r.t).unwrap();
        r.run(1200);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 400 }, r.t).unwrap(); // pending, must be discarded
        r.c.stop_path(StopKind::Stop, Some(ControlCommand::SetTargetPower { watts: 0 }), r.t);
        assert!(r.c.set_target(ControlCommand::SetTargetPower { watts: 500 }, r.t).is_err());
        r.run(1500);
        assert_eq!(r.c.stop_confirmed, Some(true));
        assert_eq!(r.sim_mode(), TrainerMode::Idle);
        let log = r.c.log_json(50).to_string_compact();
        assert!(log.contains("dropped (stop requested)"), "{log}");
        assert!(!log.contains("ERG 400 W\",\"outcome\":\"success"));
    }

    #[test]
    fn stop_without_ack_is_not_reported_as_success() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 250 }, r.t).unwrap();
        r.run(1200);
        r.sim.set_fault("sim-trainer", "drop_acks", true);
        r.c.stop_path(StopKind::Stop, None, r.t);
        r.run(4000);
        assert_eq!(r.c.stop_confirmed, Some(false));
        assert!(r.c.drain_events().iter().any(|e| e.kind == ControlEventKind::StopUnconfirmed));
    }

    #[test]
    fn disconnect_under_load_requires_new_grant_and_no_replay() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 280 }, r.t).unwrap();
        r.run(1200);
        r.c.set_target(ControlCommand::SetSimulation(SimParams { wind_speed_mps: 0.0, grade_percent: 8.0, crr: 0.004, cw_kg_per_m: 0.2 }), r.t).unwrap();
        r.sim.set_fault("sim-trainer", "offline", true);
        r.run(500);
        assert_eq!(r.c.state, ControlState::Lost);
        assert!(!r.c.has_pending(), "queued hill must not survive the disconnect");
        // Reconnect: attach again → NotControlled, nothing replayed.
        r.sim.set_fault("sim-trainer", "offline", false);
        let id = DeviceId("sim-trainer".into());
        r.sim.connect(&id).unwrap();
        r.run(500);
        r.sim.subscribe(&id, CHR_FTMS_CONTROL_POINT).unwrap();
        r.run(200);
        let caps = r.c.caps.clone().unwrap();
        r.c.attach(0, id, "sim:sim-trainer".into(), caps);
        let n = r.c.commands_sent;
        r.run(2000);
        assert_eq!(r.c.state, ControlState::NotControlled);
        assert_eq!(r.c.commands_sent, n);
    }

    #[test]
    fn stale_generation_commands_are_dropped() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        r.c.set_target(ControlCommand::SetTargetPower { watts: 222 }, r.t).unwrap();
        r.c.new_generation();
        r.run(1500);
        assert_ne!(r.sim_mode(), TrainerMode::Erg(222));
    }

    #[test]
    fn unsupported_mode_and_saturation() {
        let mut r = Rig::new();
        r.c.request_control(r.t).unwrap();
        r.run(500);
        assert!(r.c.set_target(ControlCommand::SetTargetResistance { level: 20.0 }, r.t).is_err());
        let sat = r.c.set_target(ControlCommand::SetTargetPower { watts: 1800 }, r.t).unwrap();
        assert!(sat);
        r.run(1200);
        assert_eq!(r.sim_mode(), TrainerMode::Erg(1500));
    }
}
