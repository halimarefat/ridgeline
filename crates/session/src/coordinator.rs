//! Session coordinator: the single authority over ride state, resistance
//! ownership, timing and recording.
//!
//! * Exactly one controller owns resistance (workout, route, rider or none).
//!   Mode transitions are explicit, logged and start a new control
//!   generation so stale commands cannot take effect.
//! * Time comes from the runtime's monotonic clock. Active (timer) time stops
//!   while paused; route progression freezes on pause but coasting continues
//!   while riding.
//! * Pause/stop enter the controller's stopping path immediately. A stop is
//!   reported as confirmed only on trainer acknowledgment.
//! * Losing the trainer never silently resumes a hard target: the rider must
//!   explicitly resume control, after which targets ramp in.

use crate::record::{flags, summarize, Lap, RecordSink, Sample, SessionEvent, Summary};
use crate::ride_coach::{CueInput, RideCoach};
use crate::route_engine::{GradeLimits, Progression, RouteRun};
use crate::workout_engine::WorkoutRun;
use rl_device::controller::{ControlEventKind, ControlState};
use rl_device::ftms::{ControlCommand, SimParams, StopKind};
use rl_device::manager::DeviceManager;
use rl_device::telemetry::{Freshness, Metric, Reading};
use rl_domain::physics::BikeModel;
use rl_domain::route::RouteProfile;
use rl_domain::workout::Workout;
use rl_json::{json_enum, ToJson, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RideMode {
    /// Workout engine sends ERG targets (optionally with a map background).
    Erg,
    /// Route engine sends road-simulation parameters.
    FreeRide,
    /// Rider sets a resistance level.
    Manual,
    /// No trainer control; record and display only.
    ReadOnly,
}
json_enum!(RideMode { Erg = "erg", FreeRide = "free_ride", Manual = "manual", ReadOnly = "read_only" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Workout,
    Route,
    Rider,
    None,
}
json_enum!(Owner { Workout = "workout", Route = "route", Rider = "rider", None = "none" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Prepared,
    Starting,
    Running,
    Paused,
    Stopping,
    Finished,
    Discarded,
}
json_enum!(SessionState {
    Prepared = "prepared",
    Starting = "starting",
    Running = "running",
    Paused = "paused",
    Stopping = "stopping",
    Finished = "finished",
    Discarded = "discarded",
});

pub const LOW_CADENCE_RPM: f64 = 40.0;
pub const LOW_CADENCE_HOLD_MS: u64 = 5000;
pub const RECOVER_CADENCE_RPM: f64 = 60.0;
pub const RECOVER_HOLD_MS: u64 = 3000;
pub const RAMP_IN_MS: u64 = 10_000;
pub const START_TIMEOUT_MS: u64 = 8000;
pub const STOP_WAIT_MS: u64 = 4500;

#[derive(Clone)]
pub struct PrepareSpec {
    pub mode: RideMode,
    pub workout: Option<Workout>,
    pub route: Option<(String, String, Arc<RouteProfile>)>,
    pub ftp_w: Option<f64>,
    pub rpe_mode: bool,
    pub difficulty_pct: f64,
    pub lookahead_m: f64,
    pub system_mass_kg: f64,
    pub grade_limits: GradeLimits,
    pub manual_level: f64,
    pub demo: bool,
    pub plan_session_id: Option<String>,
    pub tz_name: String,
    pub tz_offset_min: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreflightItem {
    pub key: &'static str,
    /// "ok", "warn" or "block"
    pub status: &'static str,
    pub message: String,
}

impl PreflightItem {
    fn new(key: &'static str, status: &'static str, message: impl Into<String>) -> Self {
        PreflightItem { key, status, message: message.into() }
    }
    pub fn to_json(&self) -> Value {
        Value::obj([("key", self.key.into()), ("status", self.status.into()), ("message", self.message.clone().into())])
    }
}

pub fn preflight(spec: &PrepareSpec, dm: &DeviceManager, now: u64) -> Vec<PreflightItem> {
    let mut v = Vec::new();
    let needs_control = spec.mode != RideMode::ReadOnly;
    let trainer = dm.trainer();
    if needs_control {
        match trainer {
            None => v.push(PreflightItem::new("trainer", "block", "No controllable (FTMS) trainer is connected. Connect one on the Devices screen, or choose a read-only ride.")),
            Some(t) if t.state != rl_device::manager::ConnState::Ready => {
                v.push(PreflightItem::new("trainer", "block", format!("Trainer '{}' is not ready ({}).", t.name.clone().unwrap_or_default(), t.state.as_str())))
            }
            Some(t) => {
                let caps = t.caps();
                let ok = match (spec.mode, &caps) {
                    (_, None) => false,
                    (RideMode::Erg, Some(c)) => c.features.supports_power_target(),
                    (RideMode::FreeRide, Some(c)) => c.features.supports_simulation(),
                    (RideMode::Manual, Some(c)) => c.features.supports_resistance_target(),
                    (RideMode::ReadOnly, _) => true,
                };
                if !ok {
                    v.push(PreflightItem::new("trainer", "block", format!("This trainer does not advertise support for {} mode.", spec.mode.as_str().replace('_', " "))));
                } else if !t.control_point_ready() {
                    v.push(PreflightItem::new("trainer", "block", "The trainer's control point is not available (subscription failed)."));
                } else {
                    let ctl = match dm.controller.state {
                        ControlState::Controlled => "control already granted",
                        _ => "control will be requested when you start",
                    };
                    v.push(PreflightItem::new("trainer", "ok", format!("{} ready; {ctl}.", t.name.clone().unwrap_or_else(|| "Trainer".into()))));
                }
            }
        }
    } else {
        v.push(PreflightItem::new("trainer", "ok", "Read-only ride: Ridgeline will not change trainer resistance."));
    }
    let power = dm.telemetry.reading(Metric::Power, now);
    let cad = dm.telemetry.reading(Metric::Cadence, now);
    let hr = dm.telemetry.reading(Metric::HeartRate, now);
    match power.freshness {
        Freshness::Fresh => v.push(PreflightItem::new("power", "ok", format!("Power: {:.0} W live.", power.value.unwrap_or(0.0)))),
        _ if spec.mode == RideMode::FreeRide => v.push(PreflightItem::new(
            "power",
            "warn",
            "No live power yet. Pedal to wake the trainer. Without power, route progress uses trainer speed (approximate) or coasting.",
        )),
        _ => v.push(PreflightItem::new("power", "warn", "No live power reading yet. Pedal to wake the trainer.")),
    }
    if spec.mode == RideMode::Erg {
        if cad.freshness == Freshness::Fresh {
            v.push(PreflightItem::new("cadence", "ok", "Cadence available: low-cadence protection is active."));
        } else {
            v.push(PreflightItem::new("cadence", "warn", "No cadence source: low-cadence protection is unavailable. Ease off manually if you struggle."));
        }
    }
    if hr.freshness != Freshness::Fresh {
        v.push(PreflightItem::new("heart_rate", "warn", "No live heart rate (optional)."));
    }
    if let Some(w) = &spec.workout {
        let issues = w.validate();
        if !issues.is_empty() {
            v.push(PreflightItem::new("workout", "block", format!("Workout is invalid: {}", issues[0].message)));
        } else if spec.mode == RideMode::Erg && !spec.rpe_mode && spec.ftp_w.is_none() && w.uses_basis(rl_domain::workout::Basis::FtpPct) {
            v.push(PreflightItem::new("workout", "block", "This workout uses % of FTP but no FTP is set. Enter an FTP, or ride it by perceived effort (no ERG)."));
        } else {
            v.push(PreflightItem::new("workout", "ok", format!("{} ({} min).", w.name, w.total_s() / 60)));
        }
    } else if spec.mode == RideMode::Erg {
        v.push(PreflightItem::new("workout", "block", "ERG mode needs a workout."));
    }
    if let Some((_, name, p)) = &spec.route {
        if spec.mode == RideMode::FreeRide && !p.simulation_ready {
            v.push(PreflightItem::new("route", "block", p.blocked_reason.clone().unwrap_or_else(|| "Route elevation is not ready.".into())));
        } else {
            let mut msg = format!("{}: {:.1} km, {:.0} m climbing.", name, p.total_m / 1000.0, p.ascent_m);
            if p.flat_fallback {
                msg.push_str(" Flat fallback in use for sections without elevation.");
            }
            if p.source.kind == "dem" {
                msg.push_str(" Elevation estimated from a terrain model.");
            }
            v.push(PreflightItem::new("route", if p.flat_fallback { "warn" } else { "ok" }, msg));
            v.push(PreflightItem::new("map", "ok", "Route control uses the downloaded profile; the background map is optional."));
        }
    } else if spec.mode == RideMode::FreeRide {
        v.push(PreflightItem::new("route", "block", "Choose a route for a free ride."));
    }
    if spec.demo {
        v.push(PreflightItem::new("demo", "warn", "Demo mode: simulated devices. This ride will be labelled as a demo, not a real ride."));
    }
    v
}

#[derive(Debug, Clone, Default)]
struct LowCadence {
    below_since: Option<u64>,
    above_since: Option<u64>,
    active: bool,
    rider_ack: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub level: &'static str,
    pub text: String,
    pub at_ms: u64,
}

pub struct Session {
    pub id: String,
    pub spec_mode: RideMode,
    pub mode: RideMode,
    pub owner: Owner,
    pub state: SessionState,
    pub demo: bool,
    pub ftp_w: Option<f64>,
    pub plan_session_id: Option<String>,
    pub tz_name: String,
    pub tz_offset_min: i32,
    pub workout: Option<WorkoutRun>,
    pub route: Option<RouteRun>,
    pub manual_level: f64,
    pub created_utc: i64,
    pub start_utc: Option<i64>,
    start_mono: Option<u64>,
    starting_since: u64,
    last_tick: Option<u64>,
    pub active_ms: u64,
    pub samples: Vec<Sample>,
    pub laps: Vec<Lap>,
    pub events: Vec<SessionEvent>,
    lap_open: Option<Lap>,
    last_step: Option<usize>,
    last_erg_w: Option<i16>,
    last_free_effort: bool,
    last_manual_sent: Option<f64>,
    ramp: Option<(u64, f64)>,
    low_cad: LowCadence,
    pub needs_resume_control: bool,
    pub control_interrupted: bool,
    stopping_since: Option<u64>,
    pub stop_confirmed: Option<bool>,
    sink: Option<Box<dyn RecordSink>>,
    pub recording_error: Option<String>,
    pub notices: Vec<Notice>,
    notice_seq: u64,
    flat_model: BikeModel,
    flat_v: f64,
    flat_dist: f64,
    dist_offset: f64,
    pub summary: Option<Summary>,
    last_freshness: [Freshness; 3],
    discard_after_stop: bool,
    /// In-ride coach feed (rule cues and coach replies). Text only: it never
    /// sends trainer commands.
    pub coach: RideCoach,
    /// Rule cues on/off (rider setting).
    pub coach_cues: bool,
    pub coach_imperial: bool,
}

fn sim_params(grade: f64, model: &BikeModel) -> ControlCommand {
    ControlCommand::SetSimulation(SimParams { wind_speed_mps: 0.0, grade_percent: grade, crr: model.crr, cw_kg_per_m: (model.cw_kg_per_m() * 100.0).round() / 100.0 })
}

impl Session {
    pub fn new(id: String, spec: PrepareSpec, created_utc: i64, sink: Option<Box<dyn RecordSink>>) -> Session {
        let model = BikeModel::new(spec.system_mass_kg);
        let workout = spec.workout.clone().map(|w| WorkoutRun::new(w, spec.ftp_w, spec.rpe_mode));
        let route = spec.route.clone().map(|(id, name, p)| {
            RouteRun::new(id, name, p, model.clone(), spec.difficulty_pct, spec.lookahead_m, spec.grade_limits.clone(), spec.mode == RideMode::FreeRide)
        });
        let owner = match spec.mode {
            RideMode::Erg => Owner::Workout,
            RideMode::FreeRide => Owner::Route,
            RideMode::Manual => Owner::Rider,
            RideMode::ReadOnly => Owner::None,
        };
        Session {
            id,
            spec_mode: spec.mode,
            mode: spec.mode,
            owner,
            state: SessionState::Prepared,
            demo: spec.demo,
            ftp_w: spec.ftp_w,
            plan_session_id: spec.plan_session_id,
            tz_name: spec.tz_name,
            tz_offset_min: spec.tz_offset_min,
            workout,
            route,
            manual_level: spec.manual_level,
            created_utc,
            start_utc: None,
            start_mono: None,
            starting_since: 0,
            last_tick: None,
            active_ms: 0,
            samples: Vec::new(),
            laps: Vec::new(),
            events: Vec::new(),
            lap_open: None,
            last_step: None,
            last_erg_w: None,
            last_free_effort: false,
            last_manual_sent: None,
            ramp: None,
            low_cad: LowCadence::default(),
            needs_resume_control: false,
            control_interrupted: false,
            stopping_since: None,
            stop_confirmed: None,
            sink,
            recording_error: None,
            notices: Vec::new(),
            notice_seq: 0,
            flat_model: model,
            flat_v: 0.0,
            flat_dist: 0.0,
            dist_offset: 0.0,
            summary: None,
            last_freshness: [Freshness::NoSource; 3],
            discard_after_stop: false,
            coach: RideCoach::default(),
            coach_cues: true,
            coach_imperial: false,
        }
    }

    /// Add a coach message to the ride feed and record it as a ride event.
    pub fn coach_say(&mut self, now: u64, utc: i64, from: &'static str, kind: &str, text: &str, action: Option<crate::ride_coach::CueAction>, speak: bool) -> u64 {
        let at = self.active_ms as f64 / 1000.0;
        let id = self.coach.push(at, from, kind, text, action, speak).id;
        self.event(now, utc, "coach", format!("{from}/{kind}: {text}"));
        id
    }

    fn t_rel(&self, now: u64) -> u64 {
        self.start_mono.map(|s| now.saturating_sub(s)).unwrap_or(0)
    }

    fn event(&mut self, now: u64, utc: i64, kind: &str, detail: impl Into<String>) {
        let e = SessionEvent { t_ms: self.t_rel(now), utc_ms: utc, kind: kind.into(), detail: detail.into() };
        if let Some(s) = self.sink.as_mut() {
            if let Err(err) = s.event(&e) {
                self.recording_error = Some(err);
            }
        }
        self.events.push(e);
    }

    pub fn notify(&mut self, now: u64, level: &'static str, text: impl Into<String>) {
        self.notice_seq += 1;
        self.notices.push(Notice { id: self.notice_seq, level, text: text.into(), at_ms: now });
        if self.notices.len() > 20 {
            self.notices.remove(0);
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, SessionState::Starting | SessionState::Running | SessionState::Paused | SessionState::Stopping)
    }

    fn low_load(&self, dm: &DeviceManager) -> Option<ControlCommand> {
        let caps = dm.controller.caps.as_ref()?;
        match self.owner {
            Owner::Workout if caps.features.supports_power_target() => {
                let min = caps.power_range.map(|r| r.min_w.max(0)).unwrap_or(0);
                Some(ControlCommand::SetTargetPower { watts: min })
            }
            Owner::Route if caps.features.supports_simulation() => {
                let m = self.route.as_ref().map(|r| r.model.clone()).unwrap_or_else(|| self.flat_model.clone());
                Some(sim_params(0.0, &m))
            }
            Owner::Rider if caps.features.supports_resistance_target() => {
                Some(ControlCommand::SetTargetResistance { level: caps.resistance_range.map(|r| r.min).unwrap_or(0.0) })
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------ lifecycle

    pub fn start(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) -> Result<(), String> {
        if self.state != SessionState::Prepared {
            return Err("Session already started.".into());
        }
        if self.owner == Owner::None {
            self.begin_running(now, utc, dm);
            return Ok(());
        }
        if dm.controller.device.is_none() {
            return Err("No trainer is connected.".into());
        }
        dm.controller.new_generation();
        if dm.controller.state == ControlState::Controlled {
            self.begin_running(now, utc, dm);
        } else {
            dm.controller.request_control(now)?;
            self.state = SessionState::Starting;
            self.starting_since = now;
        }
        Ok(())
    }

    fn begin_running(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) {
        self.state = SessionState::Running;
        self.start_mono = Some(now);
        self.start_utc = Some(utc);
        self.last_tick = Some(now);
        if self.owner != Owner::None {
            dm.controller.start(now);
            self.ramp = Some((now, 0.0));
            if let Some(r) = self.route.as_mut() {
                r.commander.reset(0.0);
            }
        }
        self.event(now, utc, "start", format!("mode={} owner={}{}", self.mode.as_str(), self.owner.as_str(), if self.demo { " demo" } else { "" }));
        self.open_lap(now, utc, "Ride", "start", None, None);
    }

    pub fn pause(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) {
        if self.state != SessionState::Running {
            return;
        }
        self.state = SessionState::Paused;
        if self.owner != Owner::None {
            let ll = self.low_load(dm);
            dm.controller.stop_path(StopKind::Pause, ll, now);
        }
        self.event(now, utc, "pause", "");
    }

    pub fn resume(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) {
        if self.state != SessionState::Paused {
            return;
        }
        self.state = SessionState::Running;
        self.last_tick = Some(now);
        if self.owner != Owner::None {
            if dm.controller.state == ControlState::Controlled {
                dm.controller.start(now);
                self.ramp = Some((now, self.last_erg_w.map(|w| w as f64 * 0.5).unwrap_or(0.0)));
                self.last_erg_w = None;
                self.last_manual_sent = None;
                if let Some(r) = self.route.as_mut() {
                    r.commander.reset(0.0);
                }
            } else {
                self.needs_resume_control = true;
            }
        }
        self.event(now, utc, "resume", "");
    }

    /// Stop the ride. `save=false` discards it.
    pub fn stop(&mut self, now: u64, utc: i64, dm: &mut DeviceManager, save: bool) {
        match self.state {
            SessionState::Prepared => {
                self.state = SessionState::Discarded;
                return;
            }
            SessionState::Starting => {
                dm.controller.new_generation();
                self.state = SessionState::Discarded;
                return;
            }
            SessionState::Running | SessionState::Paused => {}
            _ => return,
        }
        self.discard_after_stop = !save;
        self.close_lap(now, utc, "session_end");
        self.event(now, utc, "stop", if save { "save" } else { "discard" });
        if self.owner != Owner::None && dm.controller.device.is_some() {
            let ll = self.low_load(dm);
            dm.controller.stop_path(StopKind::Stop, ll, now);
            self.state = SessionState::Stopping;
            self.stopping_since = Some(now);
        } else {
            self.stop_confirmed = if self.owner == Owner::None { None } else { Some(false) };
            self.finish(now, utc, dm);
        }
    }

    fn finish(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) {
        dm.controller.new_generation();
        if let Some(s) = self.sink.as_mut() {
            if let Err(e) = s.flush(true) {
                self.recording_error = Some(e);
            }
        }
        let elapsed = self.t_rel(now) as f64 / 1000.0;
        self.summary = Some(summarize(&self.samples, &self.laps, elapsed, self.ftp_w));
        self.state = if self.discard_after_stop { SessionState::Discarded } else { SessionState::Finished };
        let _ = utc;
    }

    pub fn skip(&mut self, now: u64, utc: i64) {
        if let Some(w) = self.workout.as_mut() {
            if let Some(i) = w.skip() {
                self.event(now, utc, "skip", format!("step {i}"));
            }
        }
    }

    pub fn adjust_intensity(&mut self, delta: i32, now: u64, utc: i64) -> Option<i32> {
        let v = self.workout.as_mut()?.adjust(delta);
        self.event(now, utc, "intensity", format!("{v:+}%"));
        Some(v)
    }

    pub fn set_manual_level(&mut self, level: f64, now: u64, utc: i64) {
        if level.is_finite() {
            self.manual_level = level.clamp(0.0, 100.0);
            self.event(now, utc, "manual_level", format!("{:.1}", self.manual_level));
        }
    }

    pub fn manual_lap(&mut self, now: u64, utc: i64) {
        if self.state == SessionState::Running {
            self.close_lap(now, utc, "manual");
            let n = self.laps.len() + 1;
            self.open_lap(now, utc, &format!("Lap {n}"), "manual", None, None);
        }
    }

    /// Rider confirms the low-cadence recovery prompt.
    pub fn acknowledge_low_cadence(&mut self, now: u64, utc: i64) {
        if self.low_cad.active {
            self.low_cad.rider_ack = true;
            self.event(now, utc, "low_cadence_ack", "");
        }
    }

    /// After a trainer disconnect/permission loss: explicitly request control again.
    pub fn resume_control(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) -> Result<(), String> {
        if self.owner == Owner::None {
            return Err("This ride is read-only.".into());
        }
        dm.controller.new_generation();
        dm.controller.request_control(now)?;
        self.event(now, utc, "resume_control_requested", "");
        Ok(())
    }

    /// Explicit mode transition. Logged; starts a new control generation.
    pub fn switch_mode(&mut self, to: RideMode, now: u64, utc: i64, dm: &mut DeviceManager) -> Result<(), String> {
        if !matches!(self.state, SessionState::Running | SessionState::Paused) {
            return Err("Mode can only change during a ride.".into());
        }
        if to == self.mode {
            return Ok(());
        }
        let new_owner = match to {
            RideMode::ReadOnly => Owner::None,
            RideMode::Erg if self.workout.as_ref().map(|w| !w.complete).unwrap_or(false) => Owner::Workout,
            RideMode::Erg => return Err("ERG needs an unfinished workout in this ride.".into()),
            RideMode::FreeRide if self.route.is_some() => Owner::Route,
            RideMode::FreeRide => return Err("Free ride needs a route in this ride.".into()),
            RideMode::Manual => Owner::Rider,
        };
        if let Some(caps) = &dm.controller.caps {
            let ok = match to {
                RideMode::Erg => caps.features.supports_power_target(),
                RideMode::FreeRide => caps.features.supports_simulation(),
                RideMode::Manual => caps.features.supports_resistance_target(),
                RideMode::ReadOnly => true,
            };
            if !ok {
                return Err("The trainer does not support that mode.".into());
            }
        } else if to != RideMode::ReadOnly {
            return Err("No trainer connected.".into());
        }
        // Release the previous owner: drop everything queued, ease load.
        let ll = self.low_load(dm);
        dm.controller.new_generation();
        if self.owner != Owner::None && new_owner == Owner::None {
            dm.controller.stop_path(StopKind::Pause, ll, now);
        }
        let from = self.mode;
        self.mode = to;
        self.owner = new_owner;
        self.last_erg_w = None;
        self.last_manual_sent = None;
        self.low_cad = LowCadence::default();
        if let Some(r) = self.route.as_mut() {
            r.controls_resistance = new_owner == Owner::Route;
            r.commander.reset(0.0);
        }
        if new_owner != Owner::None {
            if dm.controller.state == ControlState::Controlled {
                dm.controller.start(now);
                self.ramp = Some((now, 0.0));
            } else {
                self.needs_resume_control = true;
            }
        }
        self.event(now, utc, "mode_change", format!("{} -> {}", from.as_str(), to.as_str()));
        self.notify(now, "info", format!("Mode changed to {}.", to.as_str().replace('_', " ")));
        Ok(())
    }

    pub fn new_route_lap(&mut self, now: u64, utc: i64) {
        if let Some(r) = self.route.as_mut() {
            if r.finished {
                self.dist_offset += r.profile.total_m;
                r.new_lap();
                let lap = r.lap;
                self.close_lap(now, utc, "route_lap");
                self.open_lap(now, utc, &format!("Route lap {lap}"), "route_lap", None, None);
                self.event(now, utc, "new_route_lap", format!("{lap}"));
            }
        }
    }

    // ------------------------------------------------------------ laps

    fn open_lap(&mut self, now: u64, utc: i64, label: &str, trigger: &str, step: Option<u32>, target: Option<f64>) {
        let active_s = (self.active_ms / 1000) as u32;
        self.lap_open = Some(Lap {
            index: self.laps.len() as u32,
            start_t_ms: self.t_rel(now),
            end_t_ms: self.t_rel(now),
            start_active_s: active_s,
            end_active_s: active_s,
            start_utc_ms: utc,
            label: label.into(),
            trigger: trigger.into(),
            step,
            target_w: target,
        });
    }

    fn close_lap(&mut self, now: u64, _utc: i64, trigger: &str) {
        if let Some(mut l) = self.lap_open.take() {
            l.end_t_ms = self.t_rel(now);
            l.end_active_s = (self.active_ms / 1000) as u32;
            if l.end_active_s <= l.start_active_s && !self.laps.is_empty() {
                return; // empty lap
            }
            if l.trigger == "start" {
                l.trigger = trigger.into();
            }
            if let Some(s) = self.sink.as_mut() {
                if let Err(e) = s.lap(&l) {
                    self.recording_error = Some(e);
                }
            }
            self.laps.push(l);
        }
    }

    // ------------------------------------------------------------ tick

    pub fn tick(&mut self, now: u64, utc: i64, dm: &mut DeviceManager) {
        let dt_ms = self.last_tick.map(|t| now.saturating_sub(t)).unwrap_or(0).min(2000);
        self.last_tick = Some(now);
        for ev in dm.controller.drain_events() {
            self.on_control_event(ev.kind, &ev.detail, now, utc);
        }
        match self.state {
            SessionState::Starting => {
                match dm.controller.state {
                    ControlState::Controlled => self.begin_running(now, utc, dm),
                    ControlState::Denied | ControlState::Uncertain | ControlState::Lost | ControlState::NoTrainer => {
                        self.state = SessionState::Prepared;
                        self.notify(now, "error", "Could not get control of the trainer. Close other training apps, then try again — or start a read-only ride.");
                    }
                    _ if now.saturating_sub(self.starting_since) > START_TIMEOUT_MS => {
                        self.state = SessionState::Prepared;
                        dm.controller.new_generation();
                        self.notify(now, "error", "The trainer did not answer the control request.");
                    }
                    _ => {}
                }
            }
            SessionState::Running => self.run_tick(now, utc, dt_ms, dm),
            SessionState::Stopping => {
                if let Some(c) = dm.controller.stop_confirmed {
                    self.stop_confirmed = Some(c);
                    self.finish(now, utc, dm);
                } else if now.saturating_sub(self.stopping_since.unwrap_or(now)) > STOP_WAIT_MS {
                    self.stop_confirmed = Some(false);
                    self.notify(now, "warn", "The trainer did not confirm the stop. Stop pedalling if resistance remains.");
                    self.finish(now, utc, dm);
                }
            }
            _ => {}
        }
        if let Some(s) = self.sink.as_mut() {
            match s.flush(false) {
                Ok(()) => {}
                Err(e) => self.recording_error = Some(e),
            }
        }
    }

    fn on_control_event(&mut self, kind: ControlEventKind, detail: &str, now: u64, utc: i64) {
        match kind {
            ControlEventKind::Granted => {
                self.event(now, utc, "control_granted", "");
                if self.needs_resume_control && matches!(self.state, SessionState::Running | SessionState::Paused) {
                    self.needs_resume_control = false;
                    self.control_interrupted = false;
                    self.ramp_after_regain(now);
                    self.notify(now, "info", "Trainer control resumed. Resistance ramps in over 10 seconds.");
                }
            }
            ControlEventKind::Denied => {
                self.event(now, utc, "control_denied", detail);
                self.notify(now, "error", detail.to_string());
            }
            ControlEventKind::Lost => {
                self.event(now, utc, "control_lost", detail);
                if matches!(self.state, SessionState::Running | SessionState::Paused) && self.owner != Owner::None {
                    self.control_interrupted = true;
                    self.notify(now, "error", "Trainer control lost. Values are marked stale and recording continues. The trainer may hold its last resistance: stop pedalling if needed.");
                }
            }
            ControlEventKind::StopUnconfirmed => {
                self.event(now, utc, "stop_unconfirmed", detail);
            }
            ControlEventKind::StopConfirmed => self.event(now, utc, "stop_confirmed", ""),
            ControlEventKind::Rejected(_) | ControlEventKind::Unsupported | ControlEventKind::WriteFailed | ControlEventKind::TimedOut => {
                self.event(now, utc, "control_issue", detail);
                if self.is_active() {
                    self.notify(now, "warn", detail.to_string());
                }
            }
            ControlEventKind::UserStoppedOnTrainer | ControlEventKind::SafetyKey => {
                self.event(now, utc, "trainer_stopped", detail);
                self.notify(now, "warn", detail.to_string());
            }
            ControlEventKind::Acked | ControlEventKind::Dropped => {}
        }
    }

    fn ramp_after_regain(&mut self, now: u64) {
        self.ramp = Some((now, 0.0));
        self.last_erg_w = None;
        self.last_manual_sent = None;
        if let Some(r) = self.route.as_mut() {
            r.commander.reset(0.0);
        }
    }

    fn run_tick(&mut self, now: u64, utc: i64, dt_ms: u64, dm: &mut DeviceManager) {
        let dt = dt_ms as f64 / 1000.0;
        self.active_ms += dt_ms;
        let power = dm.telemetry.reading(Metric::Power, now);
        let cad = dm.telemetry.reading(Metric::Cadence, now);
        let hr = dm.telemetry.reading(Metric::HeartRate, now);
        let tspeed = dm.telemetry.reading(Metric::TrainerSpeed, now);
        self.track_freshness(now, utc, [&power, &cad, &hr]);
        let controlled = dm.controller.state == ControlState::Controlled;
        if self.owner != Owner::None && !controlled && !self.control_interrupted && dm.controller.state != ControlState::Requesting {
            self.control_interrupted = true;
        }
        if self.control_interrupted && dm.controller.state == ControlState::NotControlled && !self.needs_resume_control {
            self.needs_resume_control = true;
            self.event(now, utc, "trainer_reconnected", "awaiting rider confirmation");
            self.notify(now, "warn", "Trainer reconnected. Press 'Resume control' when you're ready — resistance will ramp in gently.");
        }

        // Workout progression, laps and ERG target.
        let mut target_w: Option<f64> = None;
        let mut step_idx: Option<u32> = None;
        let mut free_effort = false;
        if let Some(w) = self.workout.as_mut() {
            let was_complete = w.complete;
            w.advance(dt);
            let st = w.step_state();
            let complete = w.complete;
            if let Some(st) = st {
                step_idx = Some(st.index as u32);
                target_w = st.target_w;
                free_effort = st.free_effort;
                if self.last_step != Some(st.index) && !complete {
                    let label = st.label.clone();
                    let avg_target = {
                        let s = &w.timeline[st.index];
                        match (s.target.watts_at(0.0, w.ftp_snapshot), s.target.watts_at(1.0, w.ftp_snapshot)) {
                            (Some(a), Some(b)) if !w.rpe_mode => Some(0.5 * (a + b) * (1.0 + w.adjust_pct as f64 / 100.0)),
                            _ => None,
                        }
                    };
                    let idx = st.index;
                    if self.last_step.is_some() {
                        self.close_lap(now, utc, "step");
                    } else {
                        self.lap_open = None;
                    }
                    self.open_lap(now, utc, &label, "step", Some(idx as u32), avg_target);
                    self.event(now, utc, "step", format!("{idx}: {label}"));
                    self.last_step = Some(idx);
                }
            }
            if complete && !was_complete {
                self.event(now, utc, "workout_complete", "");
                self.notify(now, "info", "Workout complete! Finish the ride when you're ready, or keep spinning.");
            }
        }
        if self.owner == Owner::Workout && controlled && !self.needs_resume_control {
            if free_effort {
                if !self.last_free_effort {
                    // RPE step: hand resistance to the rider on a flat road.
                    let m = self.flat_model.clone();
                    if dm.controller.caps.as_ref().map(|c| c.features.supports_simulation()).unwrap_or(false) {
                        let _ = dm.controller.set_target(sim_params(0.0, &m), now);
                        self.event(now, utc, "free_effort_step", "trainer set to flat road simulation; ride by feel");
                    }
                    self.last_erg_w = None;
                }
            } else if let Some(t) = target_w {
                let mut desired = t;
                // Low-cadence protection.
                desired = self.low_cadence(now, utc, desired, &cad, dm);
                let ramping = self.ramp.is_some();
                if let Some((start, from)) = self.ramp {
                    let f = (now.saturating_sub(start) as f64 / RAMP_IN_MS as f64).min(1.0);
                    desired = from + (desired - from) * f;
                    if f >= 1.0 {
                        self.ramp = None;
                    }
                }
                let w = desired.round().clamp(0.0, 2000.0) as i16;
                let deadband = if ramping { 3 } else { 1 };
                let changed = self.last_erg_w.map(|l| (l - w).abs() >= deadband).unwrap_or(true) || self.last_free_effort;
                if changed {
                    if dm.controller.set_target(ControlCommand::SetTargetPower { watts: w }, now).is_ok() {
                        self.last_erg_w = Some(w);
                    }
                }
            }
            self.last_free_effort = free_effort;
        }
        if self.owner == Owner::Rider && controlled && !self.needs_resume_control {
            if self.last_manual_sent != Some(self.manual_level) {
                if dm.controller.set_target(ControlCommand::SetTargetResistance { level: self.manual_level }, now).is_ok() {
                    self.last_manual_sent = Some(self.manual_level);
                }
            }
        }

        // Route progression and simulation commands.
        let mut sample_route = None;
        let p_for_motion = power.value;
        let ts = tspeed.value.filter(|_| p_for_motion.is_none());
        if let Some(r) = self.route.as_mut() {
            let was_finished = r.finished;
            let cmd = r.step(dt, p_for_motion, ts, now);
            if self.owner == Owner::Route && controlled && !self.needs_resume_control {
                if let Some(g) = cmd {
                    let m = r.model.clone();
                    let _ = dm.controller.set_target(sim_params(g, &m), now);
                }
            }
            let p = r.profile.at(r.s);
            sample_route = Some((r.s, r.v, p, r.road_grade, if r.controls_resistance { r.commanded_grade() } else { None }, r.progression, r.flat_fallback_active, r.commander.saturated));
            if r.finished && !was_finished {
                self.event(now, utc, "route_finished", "");
                self.notify(now, "info", "You reached the end of the route. Finish the ride or start a new lap.");
            }
        } else if let Some(p) = p_for_motion {
            // Virtual flat-road distance for workouts without a route.
            self.flat_v = self.flat_model.step(self.flat_v, p, 0.0, dt);
            self.flat_dist += self.flat_v * dt;
        } else {
            self.flat_v = self.flat_model.step(self.flat_v, 0.0, 0.0, dt);
            self.flat_dist += self.flat_v * dt;
        }

        // 1 Hz sample on each new active second.
        let sec = (self.active_ms / 1000) as u32;
        let last = self.samples.last().map(|s| s.active_s).unwrap_or(0);
        if sec > last {
            let mut f = 0u32;
            if power.freshness != Freshness::Fresh && power.freshness != Freshness::NoSource {
                f |= flags::POWER_STALE;
            }
            if cad.freshness != Freshness::Fresh && cad.freshness != Freshness::NoSource {
                f |= flags::CADENCE_STALE;
            }
            if hr.freshness != Freshness::Fresh && hr.freshness != Freshness::NoSource {
                f |= flags::HR_STALE;
            }
            if self.control_interrupted || self.needs_resume_control {
                f |= flags::CONTROL_LOST;
            }
            if self.low_cad.active {
                f |= flags::LOW_CADENCE_RECOVERY;
            }
            if self.demo {
                f |= flags::DEMO;
            }
            let src = |r: &Reading| r.source.as_ref().map(|s| format!("{}#{}", s.device, s.service.as_str()));
            let mut s = Sample {
                t_ms: self.t_rel(now),
                utc_ms: utc,
                active_s: sec,
                power: power.value,
                cadence: cad.value,
                hr: hr.value,
                trainer_speed: tspeed.value,
                step: step_idx,
                target_w: if self.owner == Owner::Workout && controlled { self.last_erg_w.map(|w| w as f64) } else { None },
                power_src: src(&power),
                cadence_src: src(&cad),
                hr_src: src(&hr),
                ..Default::default()
            };
            match sample_route {
                Some((d, v, p, road, cmd, prog, fb, sat)) => {
                    s.distance = Some(self.dist_offset + d);
                    s.speed = Some(v);
                    s.ele = p.ele;
                    s.lat = Some(p.lat);
                    s.lon = Some(p.lon);
                    s.grade = road;
                    s.cmd_grade = cmd;
                    if prog == Progression::TrainerSpeed {
                        f |= flags::APPROX_PROGRESSION;
                    }
                    if fb {
                        f |= flags::FLAT_FALLBACK;
                    }
                    if sat {
                        f |= flags::TRAINER_SATURATED;
                    }
                }
                None => {
                    s.distance = Some(self.flat_dist);
                    s.speed = Some(self.flat_v);
                }
            }
            s.flags = f;
            if let Some(sink) = self.sink.as_mut() {
                if let Err(e) = sink.sample(&s) {
                    self.recording_error = Some(e);
                }
            }
            self.samples.push(s);
            if self.coach_cues {
                self.coach_tick(now, utc, controlled);
            }
        }
    }

    /// Ride cues, once per recorded second. Read-only with respect to control.
    fn coach_tick(&mut self, now: u64, utc: i64, controlled: bool) {
        let step = self.workout.as_ref().and_then(|w| w.step_state());
        let inp = CueInput {
            active_s: self.active_ms as f64 / 1000.0,
            workout: self.workout.as_ref(),
            step: step.as_ref(),
            route: self.route.as_ref(),
            samples: &self.samples,
            erg_active: self.owner == Owner::Workout && controlled && !self.needs_resume_control,
            low_cadence_active: self.low_cad.active,
            imperial: self.coach_imperial,
        };
        let added = self.coach.evaluate(&inp);
        for it in added {
            self.event(now, utc, "coach", format!("cue/{}: {}", it.kind, it.text));
        }
    }

    /// Compact live summary for the in-ride coach (no location).
    pub fn coach_snapshot(&self, now: u64, dm: &DeviceManager) -> Value {
        let avg = |n: usize, f: &dyn Fn(&Sample) -> Option<f64>| -> Value {
            let v: Vec<f64> = self.samples.iter().rev().take(n).filter_map(f).collect();
            if v.len() * 2 < n.min(self.samples.len()).max(1) {
                Value::Null
            } else {
                ((v.iter().sum::<f64>() / v.len() as f64).round()).into()
            }
        };
        let fresh = |m: Metric| {
            let r = dm.telemetry.reading(m, now);
            if r.freshness == Freshness::Fresh {
                r.value.map(|v| Value::from(v.round())).unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        };
        let workout = self.workout.as_ref().map(|w| {
            let st = w.step_state();
            let next = w.next_step();
            Value::obj([
                ("name", w.workout.name.clone().into()),
                ("category", rl_json::ToJson::to_json(&w.workout.category)),
                ("remaining_s", w.remaining_s().round().into()),
                ("intensity_adjust_pct", w.adjust_pct.into()),
                ("can_ease", (w.adjust_pct > crate::workout_engine::MIN_ADJUST_PCT).into()),
                ("can_add", (w.adjust_pct + 5 <= crate::workout_engine::MAX_ADJUST_PCT).into()),
                ("ftp_w", w.ftp_snapshot.map(|f| f.round()).into()),
                (
                    "step",
                    st.map(|s| {
                        Value::obj([
                            ("label", s.label.into()),
                            ("elapsed_s", s.elapsed_s.round().into()),
                            ("remaining_s", s.remaining_s.round().into()),
                            ("target_w", s.target_w.map(|x| x.round()).into()),
                            ("target_pct_ftp", s.target_pct.map(|x| x.round()).into()),
                            ("rpe", s.rpe.into()),
                            ("cadence_cue", s.cadence.map(|c| Value::from(format!("{}-{}", c.0, c.1))).unwrap_or(Value::Null)),
                        ])
                    })
                    .unwrap_or(Value::Null),
                ),
                ("next_step", next.map(|n| Value::obj([("label", n.label.clone().into()), ("duration_s", n.dur_s.into())])).unwrap_or(Value::Null)),
            ])
        });
        let route = self.route.as_ref().map(|r| {
            Value::obj([
                ("name", r.route_name.clone().into()),
                ("done_m", r.s.round().into()),
                ("total_m", r.profile.total_m.round().into()),
                ("road_grade_pct", r.road_grade.map(|g| (g * 10.0).round() / 10.0).into()),
                ("climbed_m", r.ascent_m.round().into()),
                ("trainer_follows_road", r.controls_resistance.into()),
            ])
        });
        let last_cue = self.coach.feed.iter().rev().find(|f| f.from == "cue").map(|f| f.text.clone());
        // The current interval only (small models mis-read averages that span
        // a step change, and do arithmetic poorly: the ratio is precomputed).
        let this_interval = self.workout.as_ref().and_then(|w| w.current_index()).map(|idx| {
            let win: Vec<&Sample> = self.samples.iter().rev().take_while(|s| s.step == Some(idx as u32)).take(120).collect();
            let m = |f: &dyn Fn(&Sample) -> Option<f64>| {
                let v: Vec<f64> = win.iter().filter_map(|s| f(s)).collect();
                (v.len() >= 5).then(|| v.iter().sum::<f64>() / v.len() as f64)
            };
            let (p, t) = (m(&|s| s.power), m(&|s| s.target_w));
            Value::obj([
                ("seconds_in", win.len().into()),
                ("avg_power_w", p.map(|x| x.round()).into()),
                ("avg_target_w", t.map(|x| x.round()).into()),
                ("power_vs_target_pct", (match (p, t) {
                    (Some(p), Some(t)) if t > 0.0 => Some((100.0 * p / t).round()),
                    _ => None,
                }).into()),
                ("avg_cadence_rpm", m(&|s| s.cadence).map(|x| x.round()).into()),
                ("avg_heart_rate_bpm", m(&|s| s.hr).map(|x| x.round()).into()),
            ])
        });
        Value::obj([
            ("mode", self.mode.to_json()),
            ("state", self.state.to_json()),
            ("ride_time_s", (self.active_ms / 1000).into()),
            ("now", Value::obj([("power_w", fresh(Metric::Power)), ("cadence_rpm", fresh(Metric::Cadence)), ("heart_rate_bpm", fresh(Metric::HeartRate))])),
            ("this_interval", this_interval.unwrap_or(Value::Null)),
            (
                "last_60s",
                Value::obj([
                    ("avg_power_w", avg(60, &|s| s.power)),
                    ("avg_target_w", avg(60, &|s| s.target_w)),
                    ("avg_cadence_rpm", avg(60, &|s| s.cadence)),
                    ("avg_heart_rate_bpm", avg(60, &|s| s.hr)),
                ]),
            ),
            ("last_5min", Value::obj([("avg_power_w", avg(300, &|s| s.power)), ("avg_heart_rate_bpm", avg(300, &|s| s.hr))])),
            ("low_cadence_protection_active", self.low_cad.active.into()),
            ("workout", workout.unwrap_or(Value::Null)),
            ("route", route.unwrap_or(Value::Null)),
            ("latest_cue", last_cue.into()),
        ])
    }

    fn track_freshness(&mut self, now: u64, utc: i64, r: [&Reading; 3]) {
        let names = ["power", "cadence", "heart_rate"];
        for i in 0..3 {
            let f = r[i].freshness;
            if f != self.last_freshness[i] && self.last_freshness[i] != Freshness::NoSource {
                self.event(now, utc, "sensor", format!("{} {}", names[i], f.as_str()));
            }
            self.last_freshness[i] = f;
        }
    }

    fn low_cadence(&mut self, now: u64, utc: i64, target: f64, cad: &Reading, dm: &DeviceManager) -> f64 {
        let low_w = {
            let min = dm.controller.caps.as_ref().and_then(|c| c.power_range).map(|r| r.min_w.max(0) as f64).unwrap_or(0.0);
            (target * 0.4).max(min)
        };
        let Some(c) = cad.value else {
            // No reliable cadence: no detection (never invent cadence).
            self.low_cad.below_since = None;
            return if self.low_cad.active { low_w } else { target };
        };
        if !self.low_cad.active {
            if c < LOW_CADENCE_RPM && target > low_w + 1.0 {
                let since = *self.low_cad.below_since.get_or_insert(now);
                if now.saturating_sub(since) >= LOW_CADENCE_HOLD_MS {
                    self.low_cad = LowCadence { active: true, ..Default::default() };
                    self.event(now, utc, "low_cadence", format!("{c:.0} rpm; load eased to {low_w:.0} W"));
                    self.notify(now, "warn", "Low cadence detected — resistance eased. Spin up to 60+ rpm, then press Resume (R).");
                    return low_w;
                }
            } else {
                self.low_cad.below_since = None;
            }
            target
        } else {
            if c >= RECOVER_CADENCE_RPM {
                let since = *self.low_cad.above_since.get_or_insert(now);
                if self.low_cad.rider_ack && now.saturating_sub(since) >= RECOVER_HOLD_MS {
                    self.low_cad = LowCadence::default();
                    self.ramp = Some((now, low_w));
                    self.event(now, utc, "low_cadence_recovered", "");
                    return low_w;
                }
            } else {
                self.low_cad.above_since = None;
            }
            low_w
        }
    }

    pub fn low_cadence_active(&self) -> bool {
        self.low_cad.active
    }

    // ------------------------------------------------------------ views

    pub fn to_json(&self, now: u64, dm: &DeviceManager) -> Value {
        let power = dm.telemetry.reading(Metric::Power, now);
        let cad = dm.telemetry.reading(Metric::Cadence, now);
        let hr = dm.telemetry.reading(Metric::HeartRate, now);
        let workout = self.workout.as_ref().map(|w| {
            let st = w.step_state();
            let next = w.next_step();
            Value::obj([
                ("id", w.workout.id.clone().into()),
                ("name", w.workout.name.clone().into()),
                ("pos_s", w.pos_s.into()),
                ("total_s", w.total_s.into()),
                ("remaining_s", w.remaining_s().into()),
                ("complete", w.complete.into()),
                ("adjust_pct", w.adjust_pct.into()),
                ("ftp", w.ftp_snapshot.into()),
                ("rpe_mode", w.rpe_mode.into()),
                (
                    "step",
                    st.map(|s| {
                        Value::obj([
                            ("index", s.index.into()),
                            ("label", s.label.into()),
                            ("elapsed_s", s.elapsed_s.into()),
                            ("remaining_s", s.remaining_s.into()),
                            ("target_w", s.target_w.into()),
                            ("target_pct", s.target_pct.into()),
                            ("rpe", s.rpe.into()),
                            ("rpe_words", s.rpe.map(rl_domain::workout::rpe_words).into()),
                            ("cadence", s.cadence.map(|c| Value::Arr(vec![c.0.into(), c.1.into()])).unwrap_or(Value::Null)),
                            ("text", s.text.into()),
                            ("free_effort", s.free_effort.into()),
                        ])
                    })
                    .unwrap_or(Value::Null),
                ),
                ("next", next.map(|n| n.to_json()).unwrap_or(Value::Null)),
                ("timeline", Value::Arr(w.timeline.iter().map(|s| Value::Arr(vec![s.start_s.into(), s.dur_s.into(), s.target.pct_at(0.0, w.ftp_snapshot).into(), s.target.pct_at(1.0, w.ftp_snapshot).into()])).collect())),
            ])
        });
        let route = self.route.as_ref().map(|r| {
            let p = r.profile.at(r.s);
            Value::obj([
                ("id", r.route_id.clone().into()),
                ("name", r.route_name.clone().into()),
                ("s", r.s.into()),
                ("total_m", r.profile.total_m.into()),
                ("speed", r.v.into()),
                ("lat", p.lat.into()),
                ("lon", p.lon.into()),
                ("ele", p.ele.into()),
                ("road_grade", r.road_grade.into()),
                ("requested_grade", r.requested_grade.into()),
                ("commanded_grade", (if r.controls_resistance { r.commanded_grade() } else { None }).into()),
                ("difficulty_pct", (r.difficulty * 100.0).into()),
                ("saturated", r.commander.saturated.into()),
                ("progression", r.progression.as_str().into()),
                ("finished", r.finished.into()),
                ("lap", r.lap.into()),
                ("flat_fallback_active", r.flat_fallback_active.into()),
                ("controls_resistance", r.controls_resistance.into()),
                ("segment", r.segment.into()),
                ("ascent_m", r.ascent_m.into()),
            ])
        });
        let last_s = self.samples.last();
        Value::obj([
            ("id", self.id.clone().into()),
            ("state", self.state.to_json()),
            ("mode", self.mode.to_json()),
            ("owner", self.owner.to_json()),
            ("demo", self.demo.into()),
            ("active_s", (self.active_ms as f64 / 1000.0).into()),
            ("elapsed_s", (self.t_rel(now) as f64 / 1000.0).into()),
            ("distance_m", last_s.and_then(|s| s.distance).into()),
            ("power", power.to_json()),
            ("cadence", cad.to_json()),
            ("heart_rate", hr.to_json()),
            ("power_alternatives", dm.telemetry.alternatives_json(Metric::Power, now)),
            ("target_w", self.last_erg_w.into()),
            ("workout", workout.unwrap_or(Value::Null)),
            ("route", route.unwrap_or(Value::Null)),
            ("control", dm.controller.to_json()),
            ("control_interrupted", self.control_interrupted.into()),
            ("needs_resume_control", self.needs_resume_control.into()),
            ("low_cadence_active", self.low_cad.active.into()),
            ("low_cadence_ack", self.low_cad.rider_ack.into()),
            ("manual_level", self.manual_level.into()),
            ("stop_confirmed", self.stop_confirmed.into()),
            ("recording_error", self.recording_error.clone().into()),
            ("samples", self.samples.len().into()),
            ("laps", self.laps.len().into()),
            (
                "notices",
                Value::Arr(self.notices.iter().map(|n| Value::obj([("id", n.id.into()), ("level", n.level.into()), ("text", n.text.clone().into())])).collect()),
            ),
            ("summary", self.summary.as_ref().map(|s| s.to_json()).unwrap_or(Value::Null)),
            ("coach", self.coach.feed_json(12)),
            // Recent samples for live charts (last 10 minutes, downsampled to 2 s).
            (
                "recent",
                Value::Arr(
                    self.samples
                        .iter()
                        .rev()
                        .take(600)
                        .step_by(2)
                        .map(|s| Value::Arr(vec![s.active_s.into(), s.power.into(), s.hr.into(), s.cadence.into(), s.target_w.into()]))
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect(),
                ),
            ),
        ])
    }
}
