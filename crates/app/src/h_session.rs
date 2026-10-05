//! Ride session commands and the polled state snapshot.

use crate::app::{popt_str, pstr, R};
use crate::App;
use rl_device::telemetry::Metric;
use rl_domain::time::now_utc_ms;
use rl_json::{ToJson, Value};
use rl_session::coordinator::{preflight, PrepareSpec, RideMode, Session, SessionState};
use rl_session::route_engine::GradeLimits;
use rl_storage::store::ActivityMeta;

impl App {
    pub(crate) fn get_bootstrap(&mut self, p: &Value) -> R {
        if let Some(tz) = popt_str(p, "tz_name", 64)? {
            self.tz_name = tz.chars().filter(|c| c.is_ascii_graphic()).collect();
        }
        if let Some(o) = p.opt_i64("tz_offset_min")? {
            self.tz_offset_min = o.clamp(-14 * 60, 14 * 60) as i32;
        }
        let active = self.session_meta.as_ref().map(|m| m.id.clone());
        let recovery: Vec<Value> = self
            .store
            .incomplete_activities(active.as_deref())?
            .into_iter()
            .map(|m| Value::obj([("id", m.id.into()), ("title", m.title.into()), ("start_utc", m.start_utc.into()), ("demo", m.demo.into())]))
            .collect();
        let profile = self.profile();
        Ok(Value::obj([
            ("version", self.cfg.app_version.clone().into()),
            ("platform", self.cfg.platform_name.clone().into()),
            ("onboarded", profile.is_some().into()),
            ("profile", profile.map(|p| p.to_json()).unwrap_or(Value::Null)),
            ("onboarding_draft", self.store.read_value("onboarding_draft.json")?.unwrap_or(Value::Null)),
            ("settings", self.settings.to_json()),
            ("demo", self.settings.demo_mode.into()),
            ("recovery", Value::Arr(recovery)),
            ("policy_version", rl_domain::policy::POLICY_VERSION.into()),
            ("secure_store", self.platform.has_secure_store().into()),
            ("today", self.today().to_string().into()),
            ("ble_available", (self.dm.adapters.iter().any(|a| a.kind() == "ble")).into()),
        ]))
    }

    pub(crate) fn get_state(&mut self, p: &Value) -> R {
        let now = self.now();
        let show_all = p.bool_or("show_all_devices", false);
        let sim = if self.settings.demo_mode { self.sim_json() } else { Value::Null };
        let live = Value::obj([
            ("power", self.dm.telemetry.reading(Metric::Power, now).to_json()),
            ("cadence", self.dm.telemetry.reading(Metric::Cadence, now).to_json()),
            ("heart_rate", self.dm.telemetry.reading(Metric::HeartRate, now).to_json()),
            ("trainer_speed", self.dm.telemetry.reading(Metric::TrainerSpeed, now).to_json()),
            ("power_candidates", self.dm.telemetry.alternatives_json(Metric::Power, now)),
            ("cadence_candidates", self.dm.telemetry.alternatives_json(Metric::Cadence, now)),
            ("hr_candidates", self.dm.telemetry.alternatives_json(Metric::HeartRate, now)),
            ("assigned", Value::obj([
                ("power", self.dm.telemetry.assignment(Metric::Power).map(|s| s.to_json()).unwrap_or(Value::Null)),
                ("cadence", self.dm.telemetry.assignment(Metric::Cadence).map(|s| s.to_json()).unwrap_or(Value::Null)),
                ("heart_rate", self.dm.telemetry.assignment(Metric::HeartRate).map(|s| s.to_json()).unwrap_or(Value::Null)),
            ])),
        ]);
        Ok(Value::obj([
            ("now_ms", now.into()),
            ("devices", self.dm.to_json(now, show_all)),
            ("control_log", self.dm.controller.log_json(25)),
            ("live", live),
            ("session", self.session.as_ref().map(|s| s.to_json(now, &self.dm)).unwrap_or(Value::Null)),
            ("activity_id", self.session_meta.as_ref().map(|m| m.id.clone()).into()),
            ("ride_active", self.ride_active().into()),
            ("jobs", self.jobs.to_json()),
            ("ride_coach_busy", self.jobs.running("ride_coach").is_some().into()),
            ("notices", Value::Arr(self.notices.iter().map(|(id, l, t)| Value::obj([("id", (*id).into()), ("level", l.clone().into()), ("text", t.clone().into())])).collect())),
            ("demo", self.settings.demo_mode.into()),
            ("sim", sim),
        ]))
    }

    fn build_spec(&mut self, p: &Value) -> Result<PrepareSpec, String> {
        let mode = RideMode::parse(pstr(p, "mode", 20)?).ok_or("Unknown ride mode.")?;
        let workout = match popt_str(p, "workout_id", 64)? {
            Some(id) => Some(self.workout(id).cloned().ok_or("Unknown workout.")?),
            None => None,
        };
        let route = match popt_str(p, "route_id", 64)? {
            Some(id) => {
                let rec = self.store.route(id)?.ok_or("Unknown route.")?;
                let prof = self.route_profile(id)?;
                Some((id.to_string(), rec.name, prof))
            }
            None => None,
        };
        let profile = self.profile();
        let t = self.settings.trainer.clone();
        let difficulty = p.opt_f64("difficulty_pct")?.unwrap_or(t.difficulty_pct);
        Ok(PrepareSpec {
            mode,
            workout,
            route,
            ftp_w: self.current_ftp().map(|f| f.watts),
            rpe_mode: p.bool_or("rpe_mode", false),
            difficulty_pct: rl_json::check_range("difficulty_pct", difficulty, 0.0, 100.0)?,
            lookahead_m: t.lookahead_m,
            system_mass_kg: profile.as_ref().map(|p| p.system_mass_kg()).unwrap_or(84.0),
            grade_limits: GradeLimits { min_pct: t.grade_min_pct, max_pct: t.grade_max_pct, slew_pct_per_s: t.slew_pct_per_s, ..GradeLimits::default() },
            manual_level: p.opt_f64("manual_level")?.unwrap_or(20.0).clamp(0.0, 100.0),
            demo: self.settings.demo_mode || self.dm.trainer_key.as_deref().map(|k| k.starts_with("simulator:")).unwrap_or(false),
            plan_session_id: popt_str(p, "plan_session_id", 64)?.map(|s| s.to_string()),
            tz_name: self.tz_name.clone(),
            tz_offset_min: self.tz_offset_min,
        })
    }

    pub(crate) fn preflight(&mut self, p: &Value) -> R {
        let spec = self.build_spec(p)?;
        let now = self.now();
        let items = preflight(&spec, &self.dm, now);
        let blocked = items.iter().any(|i| i.status == "block");
        Ok(Value::obj([("items", Value::Arr(items.iter().map(|i| i.to_json()).collect())), ("ok", (!blocked).into())]))
    }

    pub(crate) fn start_session(&mut self, p: &Value) -> R {
        if let Some(s) = &self.session {
            if s.is_active() {
                return Err("A ride is already in progress.".into());
            }
            if s.state == SessionState::Finished {
                self.session = None;
                self.session_meta = None;
            }
        }
        let spec = self.build_spec(p)?;
        let now = self.now();
        let items = preflight(&spec, &self.dm, now);
        if let Some(b) = items.iter().find(|i| i.status == "block") {
            return Err(b.message.clone());
        }
        let id = rl_domain::ids::new_uuid();
        let utc = now_utc_ms();
        let title = {
            let base = match (&spec.workout, &spec.route) {
                (Some(w), Some((_, rn, _))) => format!("{} · {}", w.name, rn),
                (Some(w), None) => w.name.clone(),
                (None, Some((_, rn, _))) => rn.clone(),
                (None, None) => match spec.mode {
                    RideMode::Manual => "Manual resistance ride".into(),
                    _ => "Ride".into(),
                },
            };
            if spec.demo {
                format!("[Demo] {base}")
            } else {
                base
            }
        };
        let meta = ActivityMeta {
            schema: 1,
            id: id.clone(),
            status: "recording".into(),
            demo: spec.demo,
            mode: spec.mode.as_str().into(),
            title,
            start_utc: utc,
            end_utc: None,
            tz_name: spec.tz_name.clone(),
            tz_offset_min: spec.tz_offset_min,
            workout_id: spec.workout.as_ref().map(|w| w.id.clone()),
            workout_name: spec.workout.as_ref().map(|w| w.name.clone()),
            workout: spec.workout.as_ref().map(|w| w.to_json()),
            route_id: spec.route.as_ref().map(|r| r.0.clone()),
            route_name: spec.route.as_ref().map(|r| r.1.clone()),
            ftp_w: spec.ftp_w,
            plan_session_id: spec.plan_session_id.clone(),
            virtual_route: spec.route.is_some(),
            app_version: self.cfg.app_version.clone(),
            created_utc: utc,
        };
        let journal = self.store.begin_activity(&meta)?;
        let mut s = Session::new(id.clone(), spec, utc, Some(Box::new(journal)));
        s.coach_cues = self.settings.ride_coach.cues;
        s.coach_imperial = self.settings.units == rl_domain::rider::Units::Imperial;
        self.ride_ai_last_ms = None;
        if let Err(e) = s.start(now, utc, &mut self.dm) {
            let _ = self.store.delete_activity(&id);
            return Err(e);
        }
        self.session = Some(s);
        self.session_meta = Some(meta);
        self.session_finalized = false;
        Ok(Value::obj([("id", id.into())]))
    }

    pub(crate) fn session_cmd(&mut self, cmd: &str, p: &Value) -> R {
        let now = self.now();
        let utc = now_utc_ms();
        let s = self.session.as_mut().ok_or("No ride in progress.")?;
        match cmd {
            "pause" => s.pause(now, utc, &mut self.dm),
            "resume" => s.resume(now, utc, &mut self.dm),
            "stop" => s.stop(now, utc, &mut self.dm, p.bool_or("save", true)),
            "skip" => s.skip(now, utc),
            "adjust" => {
                let d = p.req_i64("delta")?.clamp(-10, 10) as i32;
                let v = s.adjust_intensity(d, now, utc);
                // A coach suggestion the rider accepted: confirm it in the feed.
                if let (Some(v), Some("coach")) = (v, popt_str(p, "via", 16)?) {
                    let what = if d < 0 { format!("easier {}%", -d) } else { format!("harder {d}%") };
                    s.coach_say(now, utc, "note", "accepted", &format!("You chose {what}. Intensity is now {v:+}%."), None, false);
                }
                return Ok(v.map(Value::from).unwrap_or(Value::Null));
            }
            "manual_level" => s.set_manual_level(p.req_f64("level")?, now, utc),
            "lap" => s.manual_lap(now, utc),
            "resume_control" => s.resume_control(now, utc, &mut self.dm)?,
            "ack_low_cadence" => s.acknowledge_low_cadence(now, utc),
            "switch_mode" => {
                let m = RideMode::parse(pstr(p, "mode", 20)?).ok_or("Unknown mode.")?;
                s.switch_mode(m, now, utc, &mut self.dm)?
            }
            "new_route_lap" => s.new_route_lap(now, utc),
            "difficulty" => {
                let d = rl_json::check_range("difficulty_pct", p.req_f64("difficulty_pct")?, 0.0, 100.0)?;
                if let Some(r) = s.route.as_mut() {
                    r.difficulty = d / 100.0;
                }
                s.notify(now, "info", format!("Trainer difficulty {d:.0}%. Route grade and climbing are unchanged; only the trainer's resistance scales."));
            }
            _ => return Err("Unknown session command.".into()),
        }
        Ok(Value::Null)
    }

    pub(crate) fn close_session(&mut self, _p: &Value) -> R {
        match &self.session {
            Some(s) if s.is_active() => Err("The ride is still in progress.".into()),
            Some(_) => {
                self.session = None;
                self.session_meta = None;
                Ok(Value::Null)
            }
            None => Ok(Value::Null),
        }
    }

    /// Persist a finished ride: summary, final meta and plan completion.
    pub(crate) fn finalize_session(&mut self) -> Result<(), String> {
        let (Some(s), Some(meta)) = (self.session.as_ref(), self.session_meta.as_mut()) else { return Ok(()) };
        let Some(summary) = s.summary.clone() else { return Ok(()) };
        meta.status = "complete".into();
        meta.end_utc = Some(s.samples.last().map(|x| x.utc_ms).unwrap_or(meta.start_utc));
        let meta = meta.clone();
        self.store.finalize_activity(&meta, &summary)?;
        if let Some(ps) = &meta.plan_session_id {
            if summary.timer_s >= 60.0 && !meta.demo {
                self.completions.set(ps, meta.id.clone());
                self.store.write_value("plans/completions.json", &self.completions)?;
            }
        }
        let stop_note = match s.stop_confirmed {
            Some(false) => " The trainer did not confirm the stop; stop pedalling if resistance remains.",
            _ => "",
        };
        let msg = format!("Ride saved.{stop_note}");
        self.notify("info", msg);
        Ok(())
    }
}
