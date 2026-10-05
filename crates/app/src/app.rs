//! Application state, the control tick and the command dispatcher.

use crate::jobs::{JobCtx, JobInfo, Jobs};
use crate::settings::Settings;
use crate::{Config, Platform};
use rl_coach::planner::PlanContext;
use rl_device::adapter::DeviceAdapter;
use rl_device::manager::DeviceManager;
use rl_device::simulator::SimulatorAdapter;
use rl_domain::library::{builtin_workouts, test_fixtures};
use rl_domain::rider::{ftp_on, FtpEntry, RiderProfile};
use rl_domain::route::RouteProfile;
use rl_domain::time::{now_utc_ms, Date};
use rl_domain::workout::Workout;
use rl_json::{parse, Value};
use rl_net::http::HttpClient;
use rl_session::coordinator::{Session, SessionState};
use rl_storage::store::{ActivityMeta, Store};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

pub type R = Result<Value, String>;

pub fn reply_json(r: R) -> String {
    match r {
        Ok(v) => Value::obj([("ok", true.into()), ("result", v)]).to_string_compact(),
        Err(e) => Value::obj([("ok", false.into()), ("error", e.into())]).to_string_compact(),
    }
}

pub struct App {
    pub cfg: Config,
    pub store: Store,
    pub settings: Settings,
    pub platform: Arc<dyn Platform>,
    pub http: Arc<dyn HttpClient>,
    pub dm: DeviceManager,
    pub sim_index: Option<usize>,
    pub session: Option<Session>,
    pub session_meta: Option<ActivityMeta>,
    pub session_finalized: bool,
    mono0: Instant,
    pub jobs: Jobs,
    pub library: Vec<Workout>,
    pub route_cache: HashMap<String, Arc<RouteProfile>>,
    pub ai_counter: (String, u32),
    pub notices: VecDeque<(u64, String, String)>,
    notice_seq: u64,
    pub self_ref: Option<Weak<Mutex<App>>>,
    keep_awake_on: bool,
    pub tz_offset_min: i32,
    pub tz_name: String,
    pub completions: Value,
    /// Test hook: added to the monotonic clock (never set in production).
    clock_offset_ms: u64,
}

impl App {
    pub fn new(cfg: Config, adapters: Vec<Box<dyn DeviceAdapter>>, http: Arc<dyn HttpClient>, platform: Arc<dyn Platform>) -> Result<App, String> {
        let store = Store::open(&cfg.data_dir)?;
        let settings = match store.read_value("settings.json") {
            Ok(Some(v)) => rl_json::FromJson::from_json(&v).unwrap_or_default(),
            _ => Settings::default(),
        };
        let completions = store.read_value("plans/completions.json").ok().flatten().unwrap_or_else(Value::empty_obj);
        let mut app = App {
            cfg,
            store,
            settings,
            platform,
            http,
            dm: DeviceManager::new(adapters),
            sim_index: None,
            session: None,
            session_meta: None,
            session_finalized: false,
            mono0: Instant::now(),
            jobs: Jobs::default(),
            library: Vec::new(),
            route_cache: HashMap::new(),
            ai_counter: (String::new(), 0),
            notices: VecDeque::new(),
            notice_seq: 0,
            self_ref: None,
            keep_awake_on: false,
            tz_offset_min: 0,
            tz_name: "UTC".into(),
            completions,
            clock_offset_ms: 0,
        };
        app.apply_trainer_settings();
        app.reload_library();
        crate::bundled::ensure_bundled_routes(&app.store)?;
        if app.settings.demo_mode {
            app.enable_demo(true);
        }
        Ok(app)
    }

    pub fn now(&self) -> u64 {
        self.mono0.elapsed().as_millis() as u64 + self.clock_offset_ms
    }

    /// Advance the monotonic clock (deterministic tests and soak runs only).
    pub fn advance_clock(&mut self, ms: u64) {
        self.clock_offset_ms += ms;
    }

    pub fn today(&self) -> Date {
        Date::from_utc_ms(now_utc_ms(), self.tz_offset_min)
    }

    pub fn notify(&mut self, level: &str, text: impl Into<String>) {
        self.notice_seq += 1;
        self.notices.push_back((self.notice_seq, level.to_string(), text.into()));
        while self.notices.len() > 20 {
            self.notices.pop_front();
        }
    }

    pub fn apply_trainer_settings(&mut self) {
        let t = self.settings.trainer.clone();
        self.dm.controller.min_target_interval_ms = t.target_interval_ms;
        self.dm.controller.ack_timeout_ms = t.ack_timeout_ms;
        let keys: Vec<String> = self.dm.devices.keys().cloned().collect();
        for k in keys {
            self.dm.telemetry.set_stale_ms(&k, t.stale_ms);
        }
    }

    pub fn reload_library(&mut self) {
        let mut lib = builtin_workouts();
        lib.extend(test_fixtures());
        if let Ok(custom) = self.store.custom_workouts() {
            lib.extend(custom);
        }
        self.library = lib;
    }

    pub fn workout(&self, id: &str) -> Option<&Workout> {
        self.library.iter().find(|w| w.id == id)
    }

    pub fn profile(&self) -> Option<RiderProfile> {
        self.store.profile().ok().flatten()
    }

    pub fn ftp_history(&self) -> Vec<FtpEntry> {
        self.store.ftp_history().unwrap_or_default()
    }

    pub fn current_ftp(&self) -> Option<FtpEntry> {
        let h = self.ftp_history();
        ftp_on(&h, self.today()).cloned()
    }

    pub fn plan_ctx<'a>(&'a self, profile: &'a RiderProfile, start: Date, weeks: u32) -> PlanContext<'a> {
        PlanContext { profile, ftp_w: self.current_ftp().map(|f| f.watts), library: &self.library, today: self.today(), start, weeks, recent_actual_min: None }
    }

    pub fn ride_active(&self) -> bool {
        self.session.as_ref().map(|s| s.is_active()).unwrap_or(false)
    }

    pub fn route_profile(&mut self, id: &str) -> Result<Arc<RouteProfile>, String> {
        if let Some(p) = self.route_cache.get(id) {
            return Ok(p.clone());
        }
        let p = self.store.route_profile(id)?.ok_or("This route has not been processed yet.")?;
        let p = Arc::new(p);
        self.route_cache.insert(id.to_string(), p.clone());
        Ok(p)
    }

    // ------------------------------------------------------------ demo

    pub fn enable_demo(&mut self, on: bool) {
        let now = self.now();
        if on {
            let idx = match self.sim_index {
                Some(i) => i,
                None => {
                    let i = self.dm.add_adapter(Box::new(SimulatorAdapter::new()));
                    self.sim_index = Some(i);
                    i
                }
            };
            let _ = self.dm.start_scan(Some(idx), now);
            self.settings.demo_mode = true;
        } else {
            let keys: Vec<String> = self.dm.devices.keys().filter(|k| k.starts_with("simulator:")).cloned().collect();
            for k in keys {
                self.dm.forget(&k, now);
            }
            if let Some(i) = self.sim_index {
                self.dm.adapters[i].stop_scan();
            }
            self.settings.demo_mode = false;
        }
        let _ = self.store.write_value("settings.json", &rl_json::ToJson::to_json(&self.settings));
    }

    pub fn sim(&mut self) -> Option<&mut SimulatorAdapter> {
        let i = self.sim_index?;
        self.dm.adapters[i].as_any_mut().downcast_mut::<SimulatorAdapter>()
    }

    // ------------------------------------------------------------ tick

    pub fn tick(&mut self) {
        let now = self.now();
        let utc = now_utc_ms();
        self.dm.tick(now);
        // Demo: auto-connect simulated devices once discovered.
        if self.settings.demo_mode {
            let keys: Vec<String> = self
                .dm
                .devices
                .iter()
                .filter(|(k, d)| k.starts_with("simulator:") && d.state == rl_device::manager::ConnState::Disconnected && !d.want_connected && d.error.is_none() && !k.ends_with("sim-power"))
                .map(|(k, _)| k.clone())
                .collect();
            for k in keys {
                let _ = self.dm.connect(&k, now);
                self.dm.telemetry.set_stale_ms(&k, self.settings.trainer.stale_ms);
            }
        }
        match self.session.as_mut() {
            Some(s) => s.tick(now, utc, &mut self.dm),
            None => {
                for e in self.dm.controller.drain_events() {
                    use rl_device::controller::ControlEventKind::*;
                    match e.kind {
                        Granted => self.notify("info", "Trainer control granted."),
                        Denied | Lost | StopUnconfirmed => self.notify("warn", e.detail),
                        _ => {}
                    }
                }
            }
        }
        self.after_session_tick();
        let active = self.ride_active();
        if active != self.keep_awake_on {
            self.keep_awake_on = active;
            self.platform.keep_awake(active);
        }
    }

    fn after_session_tick(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        match s.state {
            SessionState::Finished if !self.session_finalized => {
                self.session_finalized = true;
                if let Err(e) = self.finalize_session() {
                    self.notify("error", format!("The ride could not be saved: {e}. It will be offered for recovery on next launch."));
                }
            }
            SessionState::Discarded => {
                if let Some(m) = self.session_meta.take() {
                    let _ = self.store.delete_activity(&m.id);
                }
                self.session = None;
            }
            _ => {}
        }
    }

    pub fn shutdown(&mut self) {
        let now = self.now();
        let utc = now_utc_ms();
        if let Some(s) = self.session.as_mut() {
            if matches!(s.state, SessionState::Running | SessionState::Paused) {
                s.stop(now, utc, &mut self.dm, true);
            }
        }
        // Give the stop path a moment to be acknowledged.
        let deadline = Instant::now() + std::time::Duration::from_millis(5000);
        while self.session.as_ref().map(|s| s.state == SessionState::Stopping).unwrap_or(false) && Instant::now() < deadline {
            self.tick();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.tick();
        self.platform.keep_awake(false);
        for (_, j) in self.jobs.map.iter() {
            j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    // ------------------------------------------------------------ jobs

    pub fn spawn_job(&mut self, kind: &str, f: impl FnOnce(&JobCtx) -> R + Send + 'static) -> Result<String, String> {
        if let Some(j) = self.jobs.running(kind) {
            return Ok(j.id.clone());
        }
        let id = rl_domain::ids::new_uuid();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(JobInfo {
            id: id.clone(),
            kind: kind.into(),
            state: "running".into(),
            progress: None,
            message: String::new(),
            result: None,
            error: None,
            cancel: cancel.clone(),
            created_ms: self.now(),
            finished_ms: None,
        });
        let weak = self.self_ref.clone().unwrap_or_default();
        let ctx = JobCtx { id: id.clone(), cancel, app: weak };
        std::thread::Builder::new()
            .name(format!("job-{kind}"))
            .spawn(move || {
                let r = f(&ctx);
                let cancelled = ctx.cancelled();
                ctx.with_app(|a| {
                    let now = a.now();
                    if let Some(j) = a.jobs.map.get_mut(&ctx.id) {
                        j.finished_ms = Some(now);
                        match r {
                            _ if cancelled => j.state = "cancelled".into(),
                            Ok(v) => {
                                j.state = "done".into();
                                j.result = Some(v);
                            }
                            Err(e) => {
                                j.state = "failed".into();
                                j.error = Some(e);
                            }
                        }
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    // ------------------------------------------------------------ dispatch

    pub fn rpc(&mut self, method: &str, params_json: &str) -> R {
        if params_json.len() > 25 * 1024 * 1024 {
            return Err("Request too large.".into());
        }
        let p = if params_json.trim().is_empty() { Value::empty_obj() } else { parse(params_json).map_err(|e| format!("Invalid request: {e}"))? };
        if !matches!(p, Value::Obj(_)) {
            return Err("Request parameters must be an object.".into());
        }
        match method {
            // general
            "getBootstrap" => self.get_bootstrap(&p),
            "getState" => self.get_state(&p),
            "setDemoMode" => {
                let on = p.req_bool("on")?;
                if !on && self.ride_active() {
                    return Err("Finish the ride before leaving demo mode.".into());
                }
                self.enable_demo(on);
                Ok(Value::Null)
            }
            "cancelJob" => Ok(self.jobs.cancel(p.req_str("id")?).into()),
            "getJob" => Ok(self.jobs.map.get(p.req_str("id")?).map(|j| j.to_json()).unwrap_or(Value::Null)),
            "dismissNotice" => {
                let id = p.req_i64("id")? as u64;
                self.notices.retain(|n| n.0 != id);
                Ok(Value::Null)
            }
            // devices
            "scanDevices" => self.scan_devices(&p),
            "stopScan" => {
                self.dm.stop_scan();
                Ok(Value::Null)
            }
            "connectDevice" => self.connect_device(&p),
            "disconnectDevice" => self.disconnect_device(&p),
            "forgetDevice" => self.forget_device(&p),
            "setTrainer" => self.set_trainer(&p),
            "assignSource" => self.assign_source(&p),
            "requestControl" => self.request_control(&p),
            "simRider" => self.sim_rider(&p),
            "simFault" => self.sim_fault(&p),
            // session
            "preflight" => self.preflight(&p),
            "startSession" => self.start_session(&p),
            "pauseSession" => self.session_cmd("pause", &p),
            "resumeSession" => self.session_cmd("resume", &p),
            "stopSession" => self.session_cmd("stop", &p),
            "skipInterval" => self.session_cmd("skip", &p),
            "adjustIntensity" => self.session_cmd("adjust", &p),
            "setManualLevel" => self.session_cmd("manual_level", &p),
            "lap" => self.session_cmd("lap", &p),
            "resumeControl" => self.session_cmd("resume_control", &p),
            "ackLowCadence" => self.session_cmd("ack_low_cadence", &p),
            "switchMode" => self.session_cmd("switch_mode", &p),
            "newRouteLap" => self.session_cmd("new_route_lap", &p),
            "setDifficulty" => self.session_cmd("difficulty", &p),
            "closeSession" => self.close_session(&p),
            // profile
            "saveOnboardingDraft" => self.save_onboarding_draft(&p),
            "completeOnboarding" => self.complete_onboarding(&p),
            "startDemo" => self.start_demo(&p),
            "getProfile" => self.get_profile(&p),
            "saveProfile" => self.save_profile(&p),
            "addFtp" => self.add_ftp(&p),
            "deleteFtp" => self.delete_ftp(&p),
            // workouts
            "listWorkouts" => self.list_workouts(&p),
            "getWorkout" => self.get_workout(&p),
            "saveWorkout" => self.save_workout(&p),
            "validateWorkout" => self.validate_workout(&p),
            "duplicateWorkout" => self.duplicate_workout(&p),
            "deleteWorkout" => self.delete_workout(&p),
            // plan / coach
            "getPlan" => self.get_plan(&p),
            "proposePlan" => self.propose_plan(&p),
            "getProposal" => self.get_proposal(&p),
            "acceptProposal" => self.accept_proposal(&p),
            "rejectProposal" => self.reject_proposal(&p),
            "undoPlan" => self.undo_plan(&p),
            "previewChange" => self.preview_change(&p),
            "readinessCheck" => self.readiness_check(&p),
            "submitFeedback" => self.submit_feedback(&p),
            "adaptPlan" => self.adapt_plan(&p),
            "coachChat" => self.coach_chat(&p),
            "coachHistory" => self.coach_history(&p),
            "clearCoachHistory" => {
                self.store.clear_chat()?;
                Ok(Value::Null)
            }
            "getPolicy" => Ok(rl_domain::policy::policy_json()),
            "testAi" => self.test_ai(&p),
            "acceptFtpEstimate" => self.accept_ftp_estimate(&p),
            // routes
            "listRoutes" => self.list_routes(&p),
            "getRoute" => self.get_route(&p),
            "importGpx" => self.import_gpx(&p),
            "buildRoute" => self.build_route(&p),
            "fetchElevation" => self.fetch_elevation(&p),
            "setFlatFallback" => self.set_flat_fallback(&p),
            "addCorrection" => self.add_correction(&p),
            "clearCorrections" => self.clear_corrections(&p),
            "reverseRoute" => self.reverse_route(&p),
            "renameRoute" => self.rename_route(&p),
            "deleteRoute" => self.delete_route(&p),
            "exportRouteGpx" => self.export_route_gpx(&p),
            // history
            "listActivities" => self.list_activities(&p),
            "getActivity" => self.get_activity(&p),
            "exportActivity" => self.export_activity(&p),
            "deleteActivity" => self.delete_activity(&p),
            "recoverActivity" => self.recover_activity(&p),
            "discardRecovery" => self.discard_recovery(&p),
            "renameActivity" => self.rename_activity(&p),
            // settings / data
            "getSettings" => self.get_settings(&p),
            "saveSettings" => self.save_settings(&p),
            "setSecret" => self.set_secret(&p),
            "clearSecret" => self.clear_secret(&p),
            "storageInfo" => self.storage_info(&p),
            "deleteAllData" => self.delete_all_data(&p),
            "exportAllData" => self.export_all_data(&p),
            "exportDiagnostics" => self.export_diagnostics(&p),
            "getAbout" => self.get_about(&p),
            _ => Err(format!("Unknown command '{}'.", method.chars().take(40).collect::<String>())),
        }
    }
}

/// Bounded string parameter.
pub fn pstr<'a>(p: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    let s = p.req_str(key).map_err(|e| e.0)?;
    if s.chars().count() > max {
        return Err(format!("'{key}' is too long."));
    }
    Ok(s)
}

pub fn popt_str<'a>(p: &'a Value, key: &str, max: usize) -> Result<Option<&'a str>, String> {
    match p.opt_str(key).map_err(|e| e.0)? {
        Some(s) if s.chars().count() > max => Err(format!("'{key}' is too long.")),
        o => Ok(o),
    }
}

pub fn pnum(p: &Value, key: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let v = p.req_f64(key).map_err(|e| e.0)?;
    rl_json::check_range(key, v, lo, hi).map_err(|e| e.0)
}
