//! Session coordinator: the sole authority over ride state, resistance
//! ownership, timing and recording.
//!
//! * Exactly one owner controls resistance at a time ([`ControlMode`]).
//!   Switching owner starts a new controller generation so nothing queued by
//!   the previous owner can take effect, and the switch is logged.
//! * Time advances only from the runtime's monotonic clock; a gap > 2 s
//!   (sleep/wake, frozen process) is an interruption, not ride time.
//! * Stop/pause enter the controller stopping path immediately. A stop is
//!   reported as confirmed only when the trainer acknowledged it.
//! * Losing trainer control while riding pauses the ride; resuming requires
//!   the rider to resume explicitly, which renegotiates control and ramps the
//!   load back in.

use crate::record::{flags, Lap, RecordSink, Sample, SessionEvent};
use crate::route_engine::{GradeLimits, RouteRun};
use crate::workout_engine::WorkoutRun;
use rl_device::controller::{describe, ControlEventKind};
use rl_device::ftms::{ControlCommand, SimParams, StopKind};
use rl_device::manager::DeviceManager;
use rl_device::telemetry::{Metric, Reading};
use rl_domain::physics::BikeModel;
use rl_domain::route::RouteProfile;
use rl_domain::workout::Workout;
use rl_json::{json_enum, ToJson, Value};
use std::collections::VecDeque;
use std::sync::Arc;

/// What the rider set out to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// Structured workout with ERG targets.
    Workout,
    /// Structured workout while the map advances (terrain does not control resistance).
    WorkoutMap,
    /// Map free ride with road-gradient simulation.
    FreeRide,
    /// Rider sets resistance level.
    Manual,
    /// Record sensors only; no trainer control.
    ReadOnly,
}
json_enum!(SessionKind { Workout = "workout", WorkoutMap = "workout_map", FreeRide = "free_ride", Manual = "manual", ReadOnly = "read_only" });

/// Who owns trainer resistance right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMode {
    /// Workout engine sends ERG power targets.
    Erg,
    /// Route engine sends road simulation parameters.
    Sim,
    /// Rider-selected resistance level.
    Manual,
    /// No trainer control.
    None,
}
json_enum!(ControlMode { Erg = "erg", Sim = "sim", Manual = "manual", None = "none" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Ready,
    RequestingControl,
    Running,
    Paused,
    Stopping,
    Finished,
}
json_enum!(SessionState { Ready = "ready", RequestingControl = "requesting_control", Running = "running", Paused = "paused", Stopping = "stopping", Finished = "finished" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Rider,
    ControlLost,
    TrainerStopped,
    Interrupted,
}
json_enum!(PauseReason { Rider = "rider", ControlLost = "control_lost", TrainerStopped = "trainer_stopped", Interrupted = "interrupted" });

#[derive(Debug, Clone)]
pub struct RouteSpec {
    pub id: String,
    pub name: String,
    pub profile: Arc<RouteProfile>,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub id: String,
    pub kind: SessionKind,
    pub workout: Option<Workout>,
    /// FTP snapshot used to derive watts for this session.
    pub ftp: Option<f64>,
    /// Ride the workout by perceived effort (no ERG targets).
    pub rpe_mode: bool,
    pub route: Option<RouteSpec>,
    pub system_mass_kg: f64,
    pub difficulty_pct: f64,
    pub lookahead_m: f64,
    pub grade_limits: GradeLimits,
    pub demo: bool,
    pub planned_session_id: Option<String>,
    pub tz_offset_min: i32,
    pub manual_level: f64,
    pub low_cadence_protection: bool,
    /// FTMS rolling resistance and wind coefficients for simulation mode.
    pub crr: f64,
    pub cw_kg_per_m: f64,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.kind {
            SessionKind::Workout | SessionKind::WorkoutMap => {
                let w = self.workout.as_ref().ok_or("Choose a workout.")?;
                let issues = w.validate();
                if !issues.is_empty() {
                    return Err(format!("Workout is not valid: {}", issues[0].message));
                }
                if !self.rpe_mode && self.ftp.is_none() && w.uses_basis(rl_domain::workout::Basis::FtpPct) {
                    return Err("This workout uses % of FTP targets and no FTP is set. Enter an FTP or ride it by perceived effort.".into());
                }
            }
            _ => {}
        }
        if matches!(self.kind, SessionKind::FreeRide | SessionKind::WorkoutMap) {
            let r = self.route.as_ref().ok_or("Choose a route.")?;
            if self.kind == SessionKind::FreeRide && !r.profile.simulation_ready {
                return Err(r.profile.blocked_reason.clone().unwrap_or_else(|| "Route elevation is incomplete.".into()));
            }
        }
        if !(30.0..=300.0).contains(&self.system_mass_kg) {
            return Err("Rider + bike mass must be 30–300 kg.".into());
        }
        if !(0.0..=100.0).contains(&self.difficulty_pct) {
            return Err("Trainer difficulty must be 0–100%.".into());
        }
        if !(0.0..=0.0255).contains(&self.crr) || !(0.0..=2.55).contains(&self.cw_kg_per_m) {
            return Err("Simulation coefficients are out of range.".into());
        }
        Ok(())
    }

    pub fn initial_control_mode(&self) -> ControlMode {
        match self.kind {
            SessionKind::Workout | SessionKind::WorkoutMap => ControlMode::Erg,
            SessionKind::FreeRide => ControlMode::Sim,
            SessionKind::Manual => ControlMode::Manual,
            SessionKind::ReadOnly => ControlMode::None,
        }
    }
}

/// Seconds below the threshold before low-cadence recovery starts.
pub const LOW_CADENCE_RPM: f64 = 45.0;
pub const LOW_CADENCE_HOLD_MS: u64 = 4000;
pub const RECOVERED_CADENCE_RPM: f64 = 70.0;
pub const RECOVERED_HOLD_MS: u64 = 3000;
pub const LOW_CADENCE_FACTOR: f64 = 0.5;
pub const RAMP_MS: u64 = 15_000;
/// A tick gap longer than this is an interruption (sleep/wake or a stall).
pub const INTERRUPTION_GAP_MS: u64 = 2000;
const STOP_WAIT_MS: u64 = 8000;
const SAMPLE_BUFFER_MAX: usize = 7200;

#[derive(Debug, Clone, Copy, PartialEq)]
enum LowCad {
    Normal { below_since: Option<u64> },
    Recovery { since: u64, above_since: Option<u64> },
}

pub struct Session {
    pub cfg: SessionConfig,
    pub state: SessionState,
    pub control_mode: ControlMode,
    pub pause_reason: Option<PauseReason>,
    pub workout: Option<WorkoutRun>,
    pub route: Option<RouteRun>,
    pub start_ms: Option<u64>,
    pub start_utc_ms: i64,
    pub end_utc_ms: Option<i64>,
    last_tick_ms: Option<u64>,
    pub active_ms: u64,
    next_sample_ms: u64,
    pub samples_written: u32,
    pub laps: Vec<Lap>,
    lap_open: Option<Lap>,
    last_step_index: Option<usize>,
    last_erg_w: Option<i16>,
    last_free_effort_sent: bool,
    manual_level: f64,
    manual_dirty: bool,
    ramp_from_ms: Option<u64>,
    low_cad: LowCad,
    pub messages: VecDeque<(u64, String, String)>,
    pending: VecDeque<Pending>,
    pub record_error: Option<String>,
    stop_started_ms: Option<u64>,
    pub stop_confirmed: Option<bool>,
    pub adjustments: Vec<(u32, i32)>,
    resume_requires_control: bool,
    virt_v: f64,
    virt_dist: f64,
    pub finished_reason: Option<String>,
    pub saturated: bool,
}

enum Pending {
    Sample(Sample),
    Event(SessionEvent),
    Lap(Lap),
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

impl Session {
    pub fn new(cfg: SessionConfig) -> Result<Session, String> {
        cfg.validate()?;
        let workout = cfg.workout.clone().map(|w| WorkoutRun::new(w, cfg.ftp, cfg.rpe_mode));
        let route = cfg.route.as_ref().map(|r| {
            RouteRun::new(
                r.id.clone(),
                r.name.clone(),
                r.profile.clone(),
                BikeModel::new(cfg.system_mass_kg),
                cfg.difficulty_pct,
                cfg.lookahead_m,
                cfg.grade_limits.clone(),
                cfg.kind == SessionKind::FreeRide,
            )
        });
        let control_mode = cfg.initial_control_mode();
        let manual_level = cfg.manual_level;
        Ok(Session {
            cfg,
            state: SessionState::Ready,
            control_mode,
            pause_reason: None,
            workout,
            route,
            start_ms: None,
            start_utc_ms: 0,
            end_utc_ms: None,
            last_tick_ms: None,
            active_ms: 0,
            next_sample_ms: 1000,
            samples_written: 0,
            laps: Vec::new(),
            lap_open: None,
            last_step_index: None,
            last_erg_w: None,
            last_free_effort_sent: false,
            manual_level,
            manual_dirty: true,
            ramp_from_ms: None,
            low_cad: LowCad::Normal { below_since: None },
            messages: VecDeque::new(),
            pending: VecDeque::new(),
            record_error: None,
            stop_started_ms: None,
            stop_confirmed: None,
            adjustments: Vec::new(),
            resume_requires_control: false,
            virt_v: 0.0,
            virt_dist: 0.0,
            finished_reason: None,
            saturated: false,
        })
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.state, SessionState::Finished)
    }

    fn t(&self, now: u64) -> u64 {
        self.start_ms.map(|s| now.saturating_sub(s)).unwrap_or(0)
    }

    fn msg(&mut self, now: u64, level: &str, text: impl Into<String>) {
        self.messages.push_back((now, level.to_string(), text.into()));
        while self.messages.len() > 20 {
            self.messages.pop_front();
        }
    }

    fn event(&mut self, now: u64, utc: i64, kind: &str, detail: impl Into<String>) {
        let e = SessionEvent { t_ms: self.t(now), utc_ms: utc, kind: kind.into(), detail: detail.into() };
        self.pending.push_back(Pending::Event(e));
    }

    fn needs_control(&self) -> bool {
        self.control_mode != ControlMode::None
    }

    fn low_load(&self, dm: &DeviceManager) -> Option<ControlCommand> {
        match self.control_mode {
            ControlMode::Erg => Some(ControlCommand::SetTargetPower { watts: 0 }),
            ControlMode::Sim => Some(self.sim_cmd(0.0)),
            ControlMode::Manual => {
                let min = dm.controller.caps.as_ref().and_then(|c| c.resistance_range).map(|r| r.min).unwrap_or(0.0);
                Some(ControlCommand::SetTargetResistance { level: min })
            }
            ControlMode::None => None,
        }
    }

    fn sim_cmd(&self, grade: f64) -> ControlCommand {
        ControlCommand::SetSimulation(SimParams { wind_speed_mps: 0.0, grade_percent: grade, crr: self.cfg.crr, cw_kg_per_m: self.cfg.cw_kg_per_m })
    }

    // ------------------------------------------------------------ commands

    /// Start the ride. In controlled modes this requests trainer control
    /// first; riding begins when the trainer grants it.
    pub fn start(&mut self, dm: &mut DeviceManager, now: u64, utc: i64) -> Result<(), String> {
        if self.state != SessionState::Ready {
            return Err("The session has already started.".into());
        }
        if self.needs_control() {
            if dm.controller.device.is_none() {
                return Err("No controllable trainer is connected. Connect a trainer, or choose a read-only ride.".into());
            }
            let caps = dm.controller.caps.clone();
            let ok = match (self.control_mode, caps.as_ref()) {
                (ControlMode::Erg, Some(c)) => c.features.supports_power_target(),
                (ControlMode::Sim, Some(c)) => c.features.supports_simulation(),
                (ControlMode::Manual, Some(c)) => c.features.supports_resistance_target(),
                _ => false,
            };
            if !ok {
                return Err(format!(
                    "The trainer does not advertise support for {} mode. Ridgeline will not silently substitute another mode; choose a different ride type or a read-only ride.",
                    match self.control_mode {
                        ControlMode::Erg => "ERG (target power)",
                        ControlMode::Sim => "road simulation",
                        _ => "resistance level",
                    }
                ));
            }
        }
        self.start_ms = Some(now);
        self.start_utc_ms = utc;
        self.last_tick_ms = Some(now);
        self.event(now, utc, "session_start", format!("{} / control {}", self.cfg.kind.as_str(), self.control_mode.as_str()));
        if self.cfg.demo {
            self.event(now, utc, "demo", "Simulated devices: this is not a real ride.");
        }
        if self.needs_control() {
            dm.controller.new_generation();
            if dm.controller.is_controlled() {
                self.begin_running(dm, now, utc, false);
            } else {
                dm.controller.request_control(now)?;
                self.state = SessionState::RequestingControl;
                self.event(now, utc, "control_requested", "");
            }
        } else {
            self.begin_running(dm, now, utc, false);
        }
        Ok(())
    }

    fn begin_running(&mut self, dm: &mut DeviceManager, now: u64, utc: i64, resumed: bool) {
        if self.needs_control() {
            dm.controller.clear_stopping();
            dm.controller.start(now);
            self.ramp_from_ms = Some(now);
            self.last_erg_w = None;
            self.last_free_effort_sent = false;
            self.manual_dirty = true;
            if let Some(r) = self.route.as_mut() {
                r.commander.reset(0.0);
            }
        }
        self.state = SessionState::Running;
        self.pause_reason = None;
        self.resume_requires_control = false;
        if self.lap_open.is_none() {
            self.open_lap(now, utc, "Start", "step");
        }
        self.event(now, utc, if resumed { "resume" } else { "timer_start" }, "");
    }

    pub fn pause(&mut self, dm: &mut DeviceManager, now: u64, utc: i64, reason: PauseReason) {
        if !matches!(self.state, SessionState::Running | SessionState::RequestingControl) {
            return;
        }
        self.state = SessionState::Paused;
        self.pause_reason = Some(reason);
        if self.needs_control() && dm.controller.is_controlled() {
            let ll = self.low_load(dm);
            dm.controller.stop_path(StopKind::Pause, ll, now);
        }
        if reason != PauseReason::Rider {
            self.resume_requires_control = self.needs_control();
        }
        self.event(now, utc, "pause", reason.as_str());
    }

    pub fn resume(&mut self, dm: &mut DeviceManager, now: u64, utc: i64) -> Result<(), String> {
        if self.state != SessionState::Paused {
            return Err("The ride is not paused.".into());
        }
        if self.route.as_ref().map(|r| r.finished).unwrap_or(false) && self.cfg.kind == SessionKind::FreeRide {
            return Err("You reached the end of the route. Finish the ride or start a new lap.".into());
        }
        self.last_tick_ms = Some(now);
        if !self.needs_control() {
            self.begin_running(dm, now, utc, true);
            return Ok(());
        }
        if dm.controller.device.is_none() {
            return Err("The trainer is not connected. Wait for it to reconnect, or switch this ride to read-only.".into());
        }
        if dm.controller.is_controlled() && !self.resume_requires_control {
            self.begin_running(dm, now, utc, true);
        } else {
            // Fresh control negotiation; nothing from before can replay.
            dm.controller.new_generation();
            dm.controller.request_control(now)?;
            self.state = SessionState::RequestingControl;
            self.event(now, utc, "control_requested", "resume");
        }
        Ok(())
    }

    /// Stop the ride: enter the stopping path immediately.
    pub fn stop(&mut self, dm: &mut DeviceManager, now: u64, utc: i64, reason: &str) {
        if matches!(self.state, SessionState::Stopping | SessionState::Finished) {
            return;
        }
        if self.state == SessionState::Ready {
            self.state = SessionState::Finished;
            self.finished_reason = Some("cancelled".into());
            return;
        }
        self.finished_reason = Some(reason.into());
        self.state = SessionState::Stopping;
        self.stop_started_ms = Some(now);
        if self.needs_control() {
            let ll = self.low_load(dm);
            dm.controller.stop_path(StopKind::Stop, ll, now);
        } else {
            self.stop_confirmed = None;
        }
        self.event(now, utc, "stop_requested", reason);
    }

    pub fn skip_step(&mut self, now: u64, utc: i64) -> Result<(), String> {
        if !matches!(self.state, SessionState::Running | SessionState::Paused) {
            return Err("The ride is not running.".into());
        }
        let w = self.workout.as_mut().ok_or("No workout in this ride.")?;
        let i = w.skip().ok_or("Nothing to skip.")?;
        self.event(now, utc, "skip", format!("step {i}"));
        Ok(())
    }

    pub fn adjust_intensity(&mut self, delta: i32, now: u64, utc: i64) -> Result<i32, String> {
        if delta.abs() > 10 {
            return Err("Adjust in small steps (±10% at most).".into());
        }
        let w = self.workout.as_mut().ok_or("No workout in this ride.")?;
        let v = w.adjust(delta);
        let at = (self.active_ms / 1000) as u32;
        self.adjustments.push((at, v));
        self.event(now, utc, "intensity_adjust", format!("{v:+}%"));
        Ok(v)
    }

    pub fn set_manual_level(&mut self, level: f64) -> Result<(), String> {
        if self.control_mode != ControlMode::Manual {
            return Err("Resistance level is only used in manual mode.".into());
        }
        if !level.is_finite() || !(0.0..=100.0).contains(&level) {
            return Err("Resistance level must be 0–100.".into());
        }
        self.manual_level = level;
        self.manual_dirty = true;
        Ok(())
    }

    /// Explicit resistance-owner change (spec A08). Starts a new controller
    /// generation so commands queued by the previous owner are discarded.
    pub fn switch_control(&mut self, dm: &mut DeviceManager, mode: ControlMode, now: u64, utc: i64) -> Result<(), String> {
        if mode == self.control_mode {
            return Ok(());
        }
        match mode {
            ControlMode::Erg if self.workout.is_none() => return Err("ERG needs a workout.".into()),
            ControlMode::Sim if self.route.is_none() => return Err("Road simulation needs a route.".into()),
            ControlMode::Sim if !self.route.as_ref().unwrap().profile.simulation_ready => return Err("Route elevation is incomplete.".into()),
            _ => {}
        }
        if let Some(c) = dm.controller.caps.as_ref() {
            let ok = match mode {
                ControlMode::Erg => c.features.supports_power_target(),
                ControlMode::Sim => c.features.supports_simulation(),
                ControlMode::Manual => c.features.supports_resistance_target(),
                ControlMode::None => true,
            };
            if !ok {
                return Err("The trainer does not support that mode.".into());
            }
        } else if mode != ControlMode::None {
            return Err("No controllable trainer is connected.".into());
        }
        let old = self.control_mode;
        dm.controller.new_generation();
        if mode == ControlMode::None && dm.controller.is_controlled() {
            // Hand back to the rider at low load.
            let ll = self.low_load(dm);
            dm.controller.stop_path(StopKind::Stop, ll, now);
        }
        self.control_mode = mode;
        if let Some(r) = self.route.as_mut() {
            r.controls_resistance = mode == ControlMode::Sim;
            r.commander.reset(0.0);
        }
        self.last_erg_w = None;
        self.last_free_effort_sent = false;
        self.manual_dirty = true;
        self.ramp_from_ms = Some(now);
        if mode != ControlMode::None && self.state == SessionState::Running && !dm.controller.is_controlled() {
            dm.controller.request_control(now)?;
            self.state = SessionState::RequestingControl;
        }
        self.event(now, utc, "mode_change", format!("{} -> {}", old.as_str(), mode.as_str()));
        self.msg(now, "info", format!("Resistance control switched from {} to {}.", old.as_str(), mode.as_str()));
        Ok(())
    }

    pub fn new_route_lap(&mut self, now: u64, utc: i64) -> Result<(), String> {
        let r = self.route.as_mut().ok_or("No route in this ride.")?;
        if !r.finished {
            return Err("You have not reached the end of the route yet.".into());
        }
        r.new_lap();
        let lap = r.lap;
        self.close_lap(now, utc);
        self.open_lap(now, utc, &format!("Lap {lap}"), "route_lap");
        self.event(now, utc, "route_lap", format!("lap {lap}"));
        if self.state == SessionState::Paused {
            self.resume_requires_control = false;
        }
        Ok(())
    }

    pub fn manual_lap(&mut self, now: u64, utc: i64) {
        if self.state == SessionState::Running {
            let n = self.laps.len() + 2;
            self.close_lap(now, utc);
            self.open_lap(now, utc, &format!("Lap {n}"), "manual");
        }
    }

    pub fn resume_target_now(&mut self, now: u64, utc: i64) {
        if let LowCad::Recovery { .. } = self.low_cad {
            self.low_cad = LowCad::Normal { below_since: None };
            self.ramp_from_ms = Some(now);
            self.event(now, utc, "low_cadence_recovery_end", "rider");
        }
    }

    // ------------------------------------------------------------ laps

    fn open_lap(&mut self, now: u64, utc: i64, label: &str, trigger: &str) {
        let step = self.workout.as_ref().and_then(|w| w.current_index()).map(|i| i as u32);
        let target = self.workout.as_ref().and_then(|w| w.step_state()).and_then(|s| s.target_w).map(|w| w.round());
        self.lap_open = Some(Lap {
            index: self.laps.len() as u32,
            start_t_ms: self.t(now),
            end_t_ms: self.t(now),
            start_active_s: (self.active_ms / 1000) as u32,
            end_active_s: (self.active_ms / 1000) as u32,
            start_utc_ms: utc,
            end_utc_ms: utc,
            label: label.into(),
            trigger: trigger.into(),
            step,
            target_w: target,
        });
    }

    fn close_lap(&mut self, now: u64, utc: i64) {
        if let Some(mut l) = self.lap_open.take() {
            l.end_t_ms = self.t(now);
            l.end_active_s = (self.active_ms / 1000) as u32;
            l.end_utc_ms = utc;
            if l.end_active_s > l.start_active_s {
                self.laps.push(l.clone());
                self.pending.push_back(Pending::Lap(l));
            }
        }
    }

    // ------------------------------------------------------------ tick

    pub fn tick(&mut self, dm: &mut DeviceManager, sink: &mut dyn RecordSink, now: u64, utc: i64) {
        // Controller events first: they drive control state transitions.
        for ev in dm.controller.drain_events() {
            match ev.kind {
                ControlEventKind::Granted => {
                    self.event(now, utc, "control_granted", "");
                    if self.state == SessionState::RequestingControl {
                        let resumed = self.active_ms > 0;
                        self.begin_running(dm, now, utc, resumed);
                    }
                }
                ControlEventKind::Denied => {
                    self.event(now, utc, "control_denied", ev.detail.clone());
                    self.msg(now, "error", ev.detail.clone());
                    if self.state == SessionState::RequestingControl {
                        self.state = if self.active_ms == 0 { SessionState::Ready } else { SessionState::Paused };
                        if self.state == SessionState::Paused {
                            self.pause_reason = Some(PauseReason::ControlLost);
                            self.resume_requires_control = true;
                        }
                    }
                }
                ControlEventKind::Lost => {
                    self.event(now, utc, "control_lost", ev.detail.clone());
                    if self.state == SessionState::Running && self.needs_control() {
                        self.msg(now, "error", "Trainer control was lost. The ride is paused. The trainer may keep its last resistance; stop pedalling if it feels wrong. Press Resume when the trainer is back.");
                        self.pause(dm, now, utc, PauseReason::ControlLost);
                    }
                }
                ControlEventKind::TimedOut => {
                    self.event(now, utc, "control_timeout", ev.detail.clone());
                    if self.state == SessionState::RequestingControl {
                        self.msg(now, "error", "The trainer did not answer the control request. Its state is unknown. Check that no other app is connected, then press Resume to try again.");
                        self.state = if self.active_ms == 0 { SessionState::Ready } else { SessionState::Paused };
                        self.resume_requires_control = true;
                    }
                }
                ControlEventKind::StopConfirmed => {
                    self.event(now, utc, "trainer_stop_confirmed", "");
                }
                ControlEventKind::StopUnconfirmed => {
                    self.event(now, utc, "trainer_stop_unconfirmed", ev.detail.clone());
                    self.msg(now, "warn", ev.detail.clone());
                }
                ControlEventKind::UserStoppedOnTrainer | ControlEventKind::SafetyKey => {
                    self.event(now, utc, "trainer_stopped", ev.detail.clone());
                    if self.state == SessionState::Running && self.needs_control() {
                        self.msg(now, "warn", "The trainer reports it was stopped on the device. The ride is paused.");
                        self.pause(dm, now, utc, PauseReason::TrainerStopped);
                    }
                }
                ControlEventKind::Rejected(_) | ControlEventKind::WriteFailed | ControlEventKind::Unsupported => {
                    self.event(now, utc, "control_error", ev.detail.clone());
                    self.msg(now, "warn", ev.detail.clone());
                }
                ControlEventKind::Acked | ControlEventKind::Dropped => {}
            }
        }
        for (key, st) in dm.drain_transitions() {
            if self.state != SessionState::Ready && self.state != SessionState::Finished {
                self.event(now, utc, "device", format!("{key}: {}", st.as_str()));
            }
        }

        // Timing.
        let prev = self.last_tick_ms.unwrap_or(now);
        self.last_tick_ms = Some(now);
        let gap = now.saturating_sub(prev);
        if self.state == SessionState::Running && gap > INTERRUPTION_GAP_MS {
            self.event(now, utc, "interrupted", format!("{gap} ms without updates (sleep or stall)"));
            self.msg(now, "warn", "The ride was interrupted (the computer may have slept). It is paused; press Resume to renegotiate trainer control.");
            self.pause(dm, now, utc, PauseReason::Interrupted);
            self.flush_pending(sink);
            return;
        }

        match self.state {
            SessionState::Running => self.tick_running(dm, now, utc, gap),
            SessionState::Stopping => {
                let done = !self.needs_control() || dm.controller.stop_confirmed.is_some() || !dm.controller.has_pending() || now.saturating_sub(self.stop_started_ms.unwrap_or(now)) > STOP_WAIT_MS;
                if done {
                    self.stop_confirmed = if self.needs_control() { Some(dm.controller.stop_confirmed.unwrap_or(false)) } else { None };
                    self.close_lap(now, utc);
                    self.end_utc_ms = Some(utc);
                    self.state = SessionState::Finished;
                    self.event(now, utc, "session_end", match self.stop_confirmed {
                        Some(true) => "trainer acknowledged stop",
                        Some(false) => "trainer stop NOT confirmed",
                        None => "no trainer control",
                    });
                }
            }
            _ => {}
        }
        self.flush_pending(sink);
    }

    fn tick_running(&mut self, dm: &mut DeviceManager, now: u64, utc: i64, gap: u64) {
        let dt_s = gap as f64 / 1000.0;
        self.active_ms += gap;
        let power_r = dm.telemetry.reading(Metric::Power, now);
        let cad_r = dm.telemetry.reading(Metric::Cadence, now);
        let hr_r = dm.telemetry.reading(Metric::HeartRate, now);
        let tspeed = dm.telemetry.value(Metric::TrainerSpeed, now);

        // Workout progression.
        if let Some(w) = self.workout.as_mut() {
            w.advance(dt_s);
        }
        let step = self.workout.as_ref().and_then(|w| w.step_state());
        let step_idx = step.as_ref().map(|s| s.index);
        if step_idx != self.last_step_index {
            if self.last_step_index.is_some() {
                self.close_lap(now, utc);
                let label = step.as_ref().map(|s| s.label.clone()).unwrap_or_default();
                self.open_lap(now, utc, &label, "step");
                self.event(now, utc, "step", label);
            } else if let Some(l) = self.lap_open.as_mut() {
                l.label = step.as_ref().map(|s| s.label.clone()).unwrap_or_else(|| l.label.clone());
                l.step = step_idx.map(|i| i as u32);
                l.target_w = step.as_ref().and_then(|s| s.target_w).map(|w| w.round());
            }
            self.last_step_index = step_idx;
            self.last_free_effort_sent = false;
        }

        // Route / virtual motion.
        let mut grade_cmd = None;
        if let Some(r) = self.route.as_mut() {
            let was_finished = r.finished;
            grade_cmd = r.step(dt_s, power_r.value, tspeed, now);
            let just_finished = (r.finished && !was_finished).then_some(r.lap);
            if let Some(lap) = just_finished {
                self.event(now, utc, "route_finished", format!("lap {lap}"));
                self.msg(now, "info", "You reached the end of the route. The trainer is easing to flat. Finish the ride or start a new lap.");
            }
        } else {
            // Virtual speed on a flat road, for distance in non-map rides.
            let m = BikeModel::new(self.cfg.system_mass_kg);
            self.virt_v = match power_r.value {
                Some(p) => m.step(self.virt_v, p, 0.0, dt_s),
                None => m.step(self.virt_v, 0.0, 0.0, dt_s),
            };
            self.virt_dist += self.virt_v * dt_s;
        }

        // Low-cadence protection (ERG only, real cadence only).
        let target_w_base = step.as_ref().and_then(|s| if s.free_effort { None } else { s.target_w });
        if self.control_mode == ControlMode::Erg && self.cfg.low_cadence_protection {
            match (cad_r.value, self.low_cad) {
                (Some(c), LowCad::Normal { below_since }) => {
                    if c < LOW_CADENCE_RPM && target_w_base.unwrap_or(0.0) > 0.0 {
                        let since = below_since.unwrap_or(now);
                        if now - since >= LOW_CADENCE_HOLD_MS {
                            self.low_cad = LowCad::Recovery { since: now, above_since: None };
                            self.event(now, utc, "low_cadence_recovery_start", format!("cadence {c:.0} rpm"));
                            self.msg(now, "warn", "Low cadence detected: target eased to help you get going. Spin up above 70 rpm and the target ramps back.");
                        } else {
                            self.low_cad = LowCad::Normal { below_since: Some(since) };
                        }
                    } else {
                        self.low_cad = LowCad::Normal { below_since: None };
                    }
                }
                (Some(c), LowCad::Recovery { since, above_since }) => {
                    if c >= RECOVERED_CADENCE_RPM {
                        let a = above_since.unwrap_or(now);
                        if now - a >= RECOVERED_HOLD_MS {
                            self.low_cad = LowCad::Normal { below_since: None };
                            self.ramp_from_ms = Some(now);
                            self.event(now, utc, "low_cadence_recovery_end", "cadence recovered");
                        } else {
                            self.low_cad = LowCad::Recovery { since, above_since: Some(a) };
                        }
                    } else {
                        self.low_cad = LowCad::Recovery { since, above_since: None };
                    }
                }
                // No reliable cadence: never invent one; keep the current state.
                (None, _) => {}
            }
        }
        let in_recovery = matches!(self.low_cad, LowCad::Recovery { .. });

        // Control.
        let ramp = match self.ramp_from_ms {
            Some(t0) if now.saturating_sub(t0) < RAMP_MS => 0.5 + 0.5 * (now - t0) as f64 / RAMP_MS as f64,
            Some(_) => {
                self.ramp_from_ms = None;
                1.0
            }
            None => 1.0,
        };
        let mut commanded_w: Option<f64> = None;
        if dm.controller.is_controlled() && !dm.controller.is_stopping() {
            match self.control_mode {
                ControlMode::Erg => {
                    if let Some(s) = step.as_ref() {
                        if s.free_effort {
                            if !self.last_free_effort_sent {
                                // Perceived-effort step: hand effort to the rider with a flat road feel.
                                let cmd = if dm.controller.supported(&self.sim_cmd(0.0)) { self.sim_cmd(0.0) } else { ControlCommand::SetTargetPower { watts: 0 } };
                                let _ = dm.controller.set_target(cmd.clone(), now);
                                self.event(now, utc, "free_effort_step", describe(&cmd));
                                self.last_free_effort_sent = true;
                                self.last_erg_w = None;
                            }
                        } else if let Some(tw) = s.target_w {
                            let mut w = tw * ramp;
                            if in_recovery {
                                w = (tw * LOW_CADENCE_FACTOR).min(w);
                            }
                            let wi = w.round().clamp(0.0, 2000.0) as i16;
                            commanded_w = Some(wi as f64);
                            if self.last_erg_w.map(|l| (l - wi).abs() >= 1).unwrap_or(true) {
                                if let Ok(sat) = dm.controller.set_target(ControlCommand::SetTargetPower { watts: wi }, now) {
                                    self.saturated = sat;
                                }
                                self.last_erg_w = Some(wi);
                            }
                        }
                    }
                }
                ControlMode::Sim => {
                    if let Some(g) = grade_cmd {
                        let _ = dm.controller.set_target(self.sim_cmd(g), now);
                    }
                }
                ControlMode::Manual => {
                    if self.manual_dirty {
                        let _ = dm.controller.set_target(ControlCommand::SetTargetResistance { level: self.manual_level }, now);
                        self.manual_dirty = false;
                    }
                }
                ControlMode::None => {}
            }
        } else if self.needs_control() && !dm.controller.is_controlled() && self.state == SessionState::Running {
            // Control vanished without an event (e.g. trainer unassigned).
            self.msg(now, "error", "Trainer control is not active. The ride is paused.");
            self.pause(dm, now, utc, PauseReason::ControlLost);
        }
        if self.control_mode == ControlMode::Erg && commanded_w.is_none() {
            commanded_w = self.last_erg_w.map(|w| w as f64).filter(|_| !self.last_free_effort_sent);
        }

        // Workout finished → stop.
        if self.workout.as_ref().map(|w| w.complete).unwrap_or(false) && self.cfg.kind != SessionKind::FreeRide {
            self.event(now, utc, "workout_complete", "");
            self.msg(now, "info", "Workout complete. Nice work!");
            self.record_samples(dm, now, utc, &power_r, &cad_r, &hr_r, tspeed, commanded_w, in_recovery);
            self.stop(dm, now, utc, "workout_complete");
            return;
        }
        self.record_samples(dm, now, utc, &power_r, &cad_r, &hr_r, tspeed, commanded_w, in_recovery);
    }

    #[allow(clippy::too_many_arguments)]
    fn record_samples(&mut self, dm: &DeviceManager, now: u64, utc: i64, p: &Reading, c: &Reading, h: &Reading, tspeed: Option<f64>, commanded_w: Option<f64>, in_recovery: bool) {
        let mut first = true;
        while self.active_ms >= self.next_sample_ms {
            let a = (self.next_sample_ms / 1000) as u32;
            self.next_sample_ms += 1000;
            let mut f = 0u32;
            if p.value.is_none() && p.source.is_some() {
                f |= flags::POWER_STALE;
            }
            if c.value.is_none() && c.source.is_some() {
                f |= flags::CADENCE_STALE;
            }
            if h.value.is_none() && h.source.is_some() {
                f |= flags::HR_STALE;
            }
            if self.needs_control() && !dm.controller.is_controlled() {
                f |= flags::CONTROL_LOST;
            }
            if in_recovery {
                f |= flags::LOW_CADENCE_RECOVERY;
            }
            if self.cfg.demo {
                f |= flags::DEMO;
            }
            if self.saturated || self.route.as_ref().map(|r| r.commander.saturated).unwrap_or(false) {
                f |= flags::TRAINER_SATURATED;
            }
            if !first {
                // More than one sample due in one tick: a scheduling gap. The
                // repeated measurement is not reused; values are missing.
                f |= flags::DATA_GAP;
            }
            let (speed, dist, ele, lat, lon, grade, cg) = match self.route.as_ref() {
                Some(r) => {
                    if r.progression != crate::route_engine::Progression::Power {
                        f |= flags::APPROX_PROGRESSION;
                    }
                    if r.flat_fallback_active {
                        f |= flags::FLAT_FALLBACK;
                    }
                    let pt = r.profile.at(r.s);
                    (Some(r.v), Some(r.total_distance_m), pt.ele, Some(pt.lat), Some(pt.lon), r.road_grade, if self.control_mode == ControlMode::Sim { r.commanded_grade() } else { None })
                }
                None => (Some(self.virt_v), Some(self.virt_dist), None, None, None, None, None),
            };
            let s = Sample {
                t_ms: self.t(now),
                utc_ms: utc,
                active_s: a,
                power: if first { p.value.map(round1) } else { None },
                cadence: if first { c.value.map(round1) } else { None },
                hr: if first { h.value.map(|x| x.round()) } else { None },
                speed: speed.map(|v| (v * 1000.0).round() / 1000.0),
                distance: dist.map(|d| (d * 100.0).round() / 100.0),
                ele: ele.map(|e| (e * 10.0).round() / 10.0),
                lat,
                lon,
                grade: grade.map(|g| (g * 100.0).round() / 100.0),
                cmd_grade: cg,
                target_w: commanded_w,
                trainer_speed: if first { tspeed.map(round1) } else { None },
                step: self.last_step_index.map(|i| i as u32),
                power_src: p.source.as_ref().map(|s| s.label()),
                cadence_src: c.source.as_ref().map(|s| s.label()),
                hr_src: h.source.as_ref().map(|s| s.label()),
                flags: f,
            };
            self.samples_written += 1;
            self.pending.push_back(Pending::Sample(s));
            first = false;
        }
    }

    fn flush_pending(&mut self, sink: &mut dyn RecordSink) {
        while let Some(p) = self.pending.front() {
            let r = match p {
                Pending::Sample(s) => sink.sample(s),
                Pending::Event(e) => sink.event(e),
                Pending::Lap(l) => sink.lap(l),
            };
            match r {
                Ok(()) => {
                    self.pending.pop_front();
                    if self.record_error.take().is_some() {
                        self.messages.push_back((0, "info".into(), "Recording resumed; buffered data was saved.".into()));
                    }
                }
                Err(e) => {
                    if self.record_error.is_none() {
                        self.messages.push_back((0, "error".into(), format!("Recording problem: {e}. Data is kept in memory and retried; free some disk space.")));
                    }
                    self.record_error = Some(e);
                    // Bound memory: drop oldest samples (events and laps are kept).
                    let n_samples = self.pending.iter().filter(|p| matches!(p, Pending::Sample(_))).count();
                    if n_samples > SAMPLE_BUFFER_MAX {
                        if let Some(pos) = self.pending.iter().position(|p| matches!(p, Pending::Sample(_))) {
                            self.pending.remove(pos);
                        }
                    }
                    break;
                }
            }
        }
        let _ = sink.flush(self.state == SessionState::Finished);
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    // ------------------------------------------------------------ snapshot

    pub fn snapshot_json(&self, dm: &DeviceManager, now: u64) -> Value {
        let mut v = Value::obj([
            ("id", self.cfg.id.clone().into()),
            ("kind", self.cfg.kind.to_json()),
            ("state", self.state.to_json()),
            ("control_mode", self.control_mode.to_json()),
            ("pause_reason", self.pause_reason.map(|p| p.to_json()).unwrap_or(Value::Null)),
            ("demo", self.cfg.demo.into()),
            ("elapsed_ms", self.start_ms.map(|s| now.saturating_sub(s)).into()),
            ("active_ms", self.active_ms.into()),
            ("ftp", self.cfg.ftp.into()),
            ("rpe_mode", self.cfg.rpe_mode.into()),
            ("power", dm.telemetry.reading(Metric::Power, now).to_json()),
            ("cadence", dm.telemetry.reading(Metric::Cadence, now).to_json()),
            ("heart_rate", dm.telemetry.reading(Metric::HeartRate, now).to_json()),
            ("power_alternatives", dm.telemetry.alternatives_json(Metric::Power, now)),
            ("control", dm.controller.to_json()),
            ("control_state", dm.controller.state.to_json()),
            ("stop_confirmed", self.stop_confirmed.into()),
            ("low_cadence_recovery", matches!(self.low_cad, LowCad::Recovery { .. }).into()),
            ("low_cadence_protection", (self.cfg.low_cadence_protection && self.control_mode == ControlMode::Erg).into()),
            ("cadence_available", dm.telemetry.reading(Metric::Cadence, now).source.is_some().into()),
            ("record_error", self.record_error.clone().into()),
            ("pending_records", self.pending.len().into()),
            ("samples", self.samples_written.into()),
            ("laps", self.laps.len().into()),
            ("saturated", self.saturated.into()),
            ("finished_reason", self.finished_reason.clone().into()),
            ("resume_requires_control", self.resume_requires_control.into()),
            ("messages", Value::Arr(self.messages.iter().rev().take(5).map(|(t, l, m)| Value::obj([("t", (*t).into()), ("level", l.clone().into()), ("text", m.clone().into())])).collect())),
            ("virtual_speed", (if self.route.is_some() { self.route.as_ref().unwrap().v } else { self.virt_v }).into()),
            ("virtual_distance", (if self.route.is_some() { self.route.as_ref().unwrap().total_distance_m } else { self.virt_dist }).into()),
        ]);
        if let Some(w) = &self.workout {
            let st = w.step_state();
            let next = w.next_step();
            v.set(
                "workout",
                Value::obj([
                    ("id", w.workout.id.clone().into()),
                    ("name", w.workout.name.clone().into()),
                    ("pos_s", w.pos_s.into()),
                    ("total_s", w.total_s.into()),
                    ("adjust_pct", w.adjust_pct.into()),
                    ("complete", w.complete.into()),
                    (
                        "step",
                        match st {
                            Some(s) => Value::obj([
                                ("index", s.index.into()),
                                ("label", s.label.into()),
                                ("elapsed_s", s.elapsed_s.into()),
                                ("remaining_s", s.remaining_s.into()),
                                ("target_w", s.target_w.map(|w| w.round()).into()),
                                ("target_pct", s.target_pct.map(|p| p.round()).into()),
                                ("rpe", s.rpe.into()),
                                ("rpe_words", s.rpe.map(rl_domain::workout::rpe_words).into()),
                                ("cadence", s.cadence.map(|(a, b)| Value::Arr(vec![a.into(), b.into()])).unwrap_or(Value::Null)),
                                ("text", s.text.into()),
                                ("free_effort", s.free_effort.into()),
                            ]),
                            None => Value::Null,
                        },
                    ),
                    (
                        "next",
                        match next {
                            Some(n) => Value::obj([
                                ("label", n.label.clone().into()),
                                ("dur_s", n.dur_s.into()),
                                ("target_w", n.target.watts_at(0.0, self.cfg.ftp).filter(|_| !self.cfg.rpe_mode).map(|x| (x * (1.0 + w.adjust_pct as f64 / 100.0)).round()).into()),
                                ("target_pct", n.target.pct_at(0.0, self.cfg.ftp).map(|p| p.round()).into()),
                            ]),
                            None => Value::Null,
                        },
                    ),
                    (
                        "profile",
                        Value::Arr(
                            w.timeline
                                .iter()
                                .map(|s| Value::Arr(vec![s.start_s.into(), s.dur_s.into(), s.target.pct_at(0.0, self.cfg.ftp).unwrap_or(0.0).round().into(), s.target.pct_at(1.0, self.cfg.ftp).unwrap_or(0.0).round().into()]))
                                .collect(),
                        ),
                    ),
                ]),
            );
        }
        if let Some(r) = &self.route {
            let pt = r.profile.at(r.s);
            v.set(
                "route",
                Value::obj([
                    ("id", r.route_id.clone().into()),
                    ("name", r.route_name.clone().into()),
                    ("s", r.s.into()),
                    ("total_m", r.profile.total_m.into()),
                    ("lat", pt.lat.into()),
                    ("lon", pt.lon.into()),
                    ("ele", pt.ele.into()),
                    ("road_grade", r.road_grade.into()),
                    ("requested_grade", r.requested_grade.into()),
                    ("commanded_grade", (if self.control_mode == ControlMode::Sim { r.commanded_grade() } else { None }).into()),
                    ("difficulty_pct", (r.difficulty * 100.0).into()),
                    ("saturated", r.commander.saturated.into()),
                    ("progression", r.progression.as_str().into()),
                    ("finished", r.finished.into()),
                    ("lap", r.lap.into()),
                    ("ascent_m", r.ascent_m.into()),
                    ("flat_fallback_active", r.flat_fallback_active.into()),
                    ("controls_resistance", r.controls_resistance.into()),
                    ("segment", r.segment.into()),
                    ("synthetic", (r.profile.source.kind == "synthetic").into()),
                ]),
            );
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::MemorySink;
    use rl_device::simulator::{SimulatorAdapter, TrainerMode};
    use rl_domain::geo::LatLon;
    use rl_domain::library::{builtin_workouts, test_fixtures};
    use rl_domain::route::{process, synthetic, ElevationSource, ProfileConfig, RouteInput};

    struct Rig {
        dm: DeviceManager,
        sink: MemorySink,
        t: u64,
        utc: i64,
    }

    impl Rig {
        fn new() -> Rig {
            let mut dm = DeviceManager::new(vec![Box::new(SimulatorAdapter::new())]);
            let mut r = Rig { dm: DeviceManager::new(vec![]), sink: MemorySink::default(), t: 0, utc: 1_790_000_000_000 };
            dm.tick(0);
            dm.start_scan(None, 0).unwrap();
            r.dm = dm;
            r.run_dm(1000);
            for k in ["simulator:sim-trainer", "simulator:sim-hrm", "simulator:sim-cadence"] {
                r.dm.connect(k, r.t).unwrap();
            }
            r.run_dm(2500);
            r
        }
        fn run_dm(&mut self, ms: u64) {
            let end = self.t + ms;
            while self.t < end {
                self.t += 100;
                self.utc += 100;
                self.dm.tick(self.t);
            }
        }
        fn run(&mut self, s: &mut Session, ms: u64) {
            let end = self.t + ms;
            while self.t < end {
                self.t += 100;
                self.utc += 100;
                self.dm.tick(self.t);
                s.tick(&mut self.dm, &mut self.sink, self.t, self.utc);
            }
        }
        fn sim(&mut self) -> &mut SimulatorAdapter {
            self.dm.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap()
        }
        fn mode(&mut self) -> TrainerMode {
            self.sim().trainer().mode.clone()
        }
    }

    fn cfg(kind: SessionKind, workout: Option<Workout>, route: Option<RouteSpec>) -> SessionConfig {
        SessionConfig {
            id: "test".into(),
            kind,
            workout,
            ftp: Some(250.0),
            rpe_mode: false,
            route,
            system_mass_kg: 84.0,
            difficulty_pct: 100.0,
            lookahead_m: 0.0,
            grade_limits: GradeLimits::default(),
            demo: true,
            planned_session_id: None,
            tz_offset_min: 0,
            manual_level: 20.0,
            low_cadence_protection: true,
            crr: 0.004,
            cw_kg_per_m: 0.51,
        }
    }

    fn a05_route() -> RouteSpec {
        let pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(1000.0, 0.0), (1000.0, 5.0), (500.0, 0.0), (1000.0, -3.0), (500.0, 0.0)], false);
        let segs = vec![pts];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("synthetic"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        RouteSpec { id: "a05".into(), name: "A05".into(), profile: Arc::new(p) }
    }

    #[test]
    fn erg_workout_end_to_end_with_pause_resume() {
        let mut r = Rig::new();
        let w = test_fixtures().into_iter().find(|w| w.id == "test-erg-short").unwrap();
        let mut s = Session::new(cfg(SessionKind::Workout, Some(w), None)).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        assert_eq!(s.state, SessionState::RequestingControl);
        r.run(&mut s, 1000);
        assert_eq!(s.state, SessionState::Running);
        r.run(&mut s, 20_000);
        assert_eq!(r.mode(), TrainerMode::Erg(100), "warm-up target reached after ramp");
        r.run(&mut s, 45_000); // into the 150 W step (ramp is reset only on resume)
        assert_eq!(r.mode(), TrainerMode::Erg(150));
        // Pause: stopping path with low load, then pause acknowledged.
        let (t, u) = (r.t, r.utc);
        s.pause(&mut r.dm, t, u, PauseReason::Rider);
        r.run(&mut s, 1500);
        assert_eq!(r.mode(), TrainerMode::Erg(0));
        let active_before = s.active_ms;
        r.run(&mut s, 5000);
        assert_eq!(s.active_ms, active_before, "timer frozen while paused");
        let (t, u) = (r.t, r.utc);
        s.resume(&mut r.dm, t, u).unwrap();
        r.run(&mut s, 2500);
        match r.mode() {
            TrainerMode::Erg(w) => assert!(w < 150 && w >= 75, "controlled ramp after resume, got {w}"),
            m => panic!("{m:?}"),
        }
        r.run(&mut s, 120_000);
        assert_eq!(s.state, SessionState::Finished, "workout completes and stops");
        assert_eq!(s.stop_confirmed, Some(true));
        assert_eq!(r.mode(), TrainerMode::Idle);
        let n = r.sink.samples.len();
        assert!((179..=181).contains(&n), "one sample per active second: {n}");
        assert!(r.sink.samples.windows(2).all(|w| w[1].active_s == w[0].active_s + 1), "no duplicated/missing seconds");
        assert!(r.sink.laps.len() >= 3);
        assert!(r.sink.events.iter().any(|e| e.kind == "trainer_stop_confirmed"));
        assert!(r.sink.samples.iter().all(|s| s.flags & flags::DEMO != 0), "demo data is labelled");
    }

    #[test]
    fn disconnect_under_load_pauses_and_requires_controlled_resume() {
        let mut r = Rig::new();
        let w = builtin_workouts().into_iter().find(|w| w.id == "threshold-4x5").unwrap();
        let mut s = Session::new(cfg(SessionKind::Workout, Some(w), None)).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        r.run(&mut s, 30_000);
        assert_eq!(s.state, SessionState::Running);
        r.sim().set_fault("sim-trainer", "offline", true);
        r.run(&mut s, 3000);
        assert_eq!(s.state, SessionState::Paused);
        assert_eq!(s.pause_reason, Some(PauseReason::ControlLost));
        assert!(s.stop_confirmed.is_none(), "no false stop acknowledgment");
        // Trainer returns; nothing is replayed until the rider resumes.
        r.sim().set_fault("sim-trainer", "offline", false);
        let cmds_before = r.sim().trainer().command_log.len();
        r.run(&mut s, 20_000);
        assert_eq!(r.dm.devices["simulator:sim-trainer"].state, rl_device::manager::ConnState::Ready);
        assert!(!r.dm.controller.is_controlled());
        // The simulated trainer, like many real ones, may still hold its old
        // resistance; what matters is that the app sent nothing new.
        assert_eq!(r.sim().trainer().command_log.len(), cmds_before, "no commands replayed after reconnect");
        let (t, u) = (r.t, r.utc);
        s.resume(&mut r.dm, t, u).unwrap();
        assert_eq!(s.state, SessionState::RequestingControl);
        r.run(&mut s, 3000);
        assert_eq!(s.state, SessionState::Running);
        match r.mode() {
            TrainerMode::Erg(w) => assert!(w < 200, "ramped, not full target at once: {w}"),
            m => panic!("{m:?}"),
        }
        assert!(r.sink.events.iter().any(|e| e.kind == "control_lost"));
    }

    #[test]
    fn free_ride_sends_signed_grades() {
        let mut r = Rig::new();
        r.sim().rider.effort_w = 250.0;
        r.sim().rider.responds_to_grade = false;
        let mut s = Session::new(cfg(SessionKind::FreeRide, None, Some(a05_route()))).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        let mut seen_climb = false;
        let mut seen_desc = false;
        for _ in 0..2000 {
            r.run(&mut s, 500);
            let pos = s.route.as_ref().unwrap().s;
            if let TrainerMode::Simulation(p) = r.mode() {
                if (1400.0..1900.0).contains(&pos) && (p.grade_percent - 5.0).abs() < 0.3 {
                    seen_climb = true;
                }
                if (2900.0..3400.0).contains(&pos) && (p.grade_percent + 3.0).abs() < 0.3 {
                    seen_desc = true;
                }
            }
            if s.route.as_ref().unwrap().finished {
                break;
            }
        }
        assert!(seen_climb && seen_desc, "climb {seen_climb} desc {seen_desc}");
        r.run(&mut s, 20_000);
        match r.mode() {
            TrainerMode::Simulation(p) => assert!(p.grade_percent.abs() < 0.3, "eased to flat at finish"),
            m => panic!("{m:?}"),
        }
        assert_eq!(s.state, SessionState::Running, "rider chooses finish or new lap");
        let (t, u) = (r.t, r.utc);
        s.new_route_lap(t, u).unwrap();
        assert_eq!(s.route.as_ref().unwrap().lap, 2);
    }

    #[test]
    fn mode_switch_drops_stale_commands() {
        let mut r = Rig::new();
        let w = builtin_workouts().into_iter().find(|w| w.id == "endurance-45").unwrap();
        let mut s = Session::new(cfg(SessionKind::WorkoutMap, Some(w), Some(a05_route()))).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        r.run(&mut s, 20_000);
        assert!(matches!(r.mode(), TrainerMode::Erg(_)));
        let (t, u) = (r.t, r.utc);
        s.switch_control(&mut r.dm, ControlMode::Sim, t, u).unwrap();
        r.run(&mut s, 5000);
        assert!(matches!(r.mode(), TrainerMode::Simulation(_)), "{:?}", r.mode());
        // Only one owner: no ERG commands after the switch.
        let switch_t = t;
        let erg_after = r.sim().trainer().command_log.iter().filter(|(ct, c)| *ct > switch_t + 200 && matches!(c, ControlCommand::SetTargetPower { .. })).count();
        assert_eq!(erg_after, 0);
        assert!(r.sink.events.iter().any(|e| e.kind == "mode_change"));
    }

    #[test]
    fn low_cadence_recovery_eases_and_ramps_back() {
        let mut r = Rig::new();
        let w = builtin_workouts().into_iter().find(|w| w.id == "threshold-4x5").unwrap();
        let mut c = cfg(SessionKind::Workout, Some(w), None);
        c.ftp = Some(200.0);
        let mut s = Session::new(c).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        r.run(&mut s, 800_000); // into the first threshold interval (starts at 720 s)
        assert_eq!(r.mode(), TrainerMode::Erg(200));
        r.sim().rider.cadence_rpm = 30.0;
        r.run(&mut s, 9000);
        match r.mode() {
            TrainerMode::Erg(w) => assert!(w <= 100, "eased: {w}"),
            m => panic!("{m:?}"),
        }
        r.sim().rider.cadence_rpm = 90.0;
        r.run(&mut s, 25_000);
        assert_eq!(r.mode(), TrainerMode::Erg(200), "ramped back after recovery");
        assert!(r.sink.events.iter().any(|e| e.kind == "low_cadence_recovery_start"));
        assert!(r.sink.events.iter().any(|e| e.kind == "low_cadence_recovery_end"));
    }

    #[test]
    fn interruption_and_unsupported_mode() {
        let mut r = Rig::new();
        let mut s = Session::new(cfg(SessionKind::ReadOnly, None, None)).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        r.run(&mut s, 5000);
        assert_eq!(s.state, SessionState::Running);
        // Simulated sleep: 60 s without ticks.
        r.t += 60_000;
        r.utc += 60_000;
        r.dm.tick(r.t);
        s.tick(&mut r.dm, &mut r.sink, r.t, r.utc);
        assert_eq!(s.state, SessionState::Paused);
        assert_eq!(s.pause_reason, Some(PauseReason::Interrupted));
        assert!(s.active_ms < 6000, "sleep time is not ride time");
        // Trainer without simulation support: free ride refused explicitly.
        let mut r2 = Rig::new();
        r2.sim().set_fault("sim-trainer", "no_simulation", true);
        r2.dm.disconnect("simulator:sim-trainer", r2.t);
        r2.run_dm(500);
        r2.dm.connect("simulator:sim-trainer", r2.t).unwrap();
        r2.run_dm(2000);
        let mut s2 = Session::new(cfg(SessionKind::FreeRide, None, Some(a05_route()))).unwrap();
        let err = s2.start(&mut r2.dm, r2.t, r2.utc).unwrap_err();
        assert!(err.contains("not silently substitute"), "{err}");
    }

    #[test]
    fn recording_failure_buffers_and_recovers() {
        let mut r = Rig::new();
        let mut s = Session::new(cfg(SessionKind::ReadOnly, None, None)).unwrap();
        s.start(&mut r.dm, r.t, r.utc).unwrap();
        r.run(&mut s, 3000);
        r.sink.fail = true;
        r.run(&mut s, 5000);
        assert!(s.record_error.is_some());
        assert!(s.pending_count() >= 4);
        r.sink.fail = false;
        r.run(&mut s, 2000);
        assert!(s.record_error.is_none());
        assert_eq!(s.pending_count(), 0);
        assert!(r.sink.samples.windows(2).all(|w| w[1].active_s == w[0].active_s + 1));
    }
}
