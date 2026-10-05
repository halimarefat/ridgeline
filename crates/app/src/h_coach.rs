//! Plans, proposals (accept / reject / undo), readiness, feedback and coach chat.

use crate::app::{popt_str, pstr, R};
use crate::App;
use rl_coach::adapt::{changes_json, feedback_changes, readiness_changes, Change, Feedback};
use rl_coach::coach::{chat, compact_summary, propose_plan, ActivityBrief, AiLimits};
use rl_coach::planner::PlanContext;
use rl_coach::provider::{AiProvider, ChatMessage, OpenAiCompatible};
use rl_domain::plan::{SessionStatus, TrainingPlan};
use rl_domain::policy::{self, Readiness};
use rl_domain::rider::RiderProfile;
use rl_domain::time::{now_utc_ms, Date};
use rl_json::{FromJson, ToJson, Value};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct ProposalInput {
    pub kind: &'static str,
    pub plan: TrainingPlan,
    pub source: String,
    pub explanation: String,
    pub fallback_reason: Option<String>,
    pub base: Option<(String, u32)>,
    pub changes: Value,
    pub input_summary: Value,
    pub provider: String,
    pub model: String,
    pub repaired: bool,
    pub first_issues: Value,
}

impl App {
    pub(crate) fn ai_limits(&self) -> AiLimits {
        AiLimits { timeout: Duration::from_secs(self.settings.ai.timeout_s as u64), max_tokens: self.settings.ai.max_tokens, max_attempts: 2 }
    }

    /// The configured provider, or `None` for the offline coach.
    pub(crate) fn ai_provider(&self) -> Result<Option<OpenAiCompatible>, String> {
        let a = &self.settings.ai;
        match a.provider.as_str() {
            "offline" => Ok(None),
            "local" => {
                let label = match a.preset.as_str() {
                    "ollama" => "Ollama (local)",
                    "lmstudio" => "LM Studio (local)",
                    _ => "Local model server",
                };
                Ok(Some(OpenAiCompatible { label: label.into(), base_url: a.base_url.clone(), model: a.model.clone(), api_key: None }))
            }
            "remote" => {
                if !a.remote_enabled {
                    return Err("Remote AI is disabled. It may incur charges with some providers; enable it yourself in Settings if you choose to.".into());
                }
                let key = self.platform.secret_get("ai_api_key")?;
                Ok(Some(OpenAiCompatible { label: "Remote (owner-configured)".into(), base_url: a.base_url.clone(), model: a.model.clone(), api_key: key }))
            }
            _ => Ok(None),
        }
    }

    /// Consent and daily request limit. Counts one request.
    pub(crate) fn ai_gate(&mut self, profile: &RiderProfile) -> Result<(), String> {
        if !profile.ai_consent.enabled {
            return Err("AI coaching is off. Turn on consent in Settings → AI coach to send your compact summary to the AI service.".into());
        }
        let day = self.today().to_string();
        if self.ai_counter.0 != day {
            self.ai_counter = (day, 0);
        }
        if self.ai_counter.1 >= self.settings.ai.max_requests_per_day {
            return Err(format!("Daily AI request limit reached ({}). The offline coach is still available.", self.settings.ai.max_requests_per_day));
        }
        self.ai_counter.1 += 2; // a request may include one repair call
        Ok(())
    }

    fn recent_briefs(&self) -> Vec<ActivityBrief> {
        let today = self.today();
        self.store
            .list_activities()
            .unwrap_or_default()
            .into_iter()
            .filter(|(m, _)| !m.demo && m.status != "recording")
            .filter_map(|(m, s)| {
                let d = Date::from_utc_ms(m.start_utc, m.tz_offset_min);
                if today.days_until(&d) < -28 {
                    return None;
                }
                let fb = self.store.read_value(&format!("activities/{}/feedback.json", m.id)).ok().flatten();
                Some(ActivityBrief {
                    date: d,
                    minutes: s.as_ref().map(|s| (s.timer_s / 60.0) as u32).unwrap_or(0),
                    mode: m.mode.clone(),
                    avg_power: s.as_ref().and_then(|s| s.power.avg),
                    rpe: fb.as_ref().and_then(|f| f.get("rpe")).and_then(|x| x.as_i64()).map(|x| x as u8),
                    fatigue: fb.as_ref().and_then(|f| f.get("fatigue")).and_then(|x| x.as_i64()).map(|x| x as u8),
                })
            })
            .collect()
    }

    fn all_feedback(&self) -> Vec<Feedback> {
        self.store
            .list_activities()
            .unwrap_or_default()
            .into_iter()
            .filter(|(m, _)| !m.demo)
            .filter_map(|(m, _)| self.store.read_value(&format!("activities/{}/feedback.json", m.id)).ok().flatten())
            .filter_map(|v| Feedback::from_json(&v).ok())
            .collect()
    }

    pub(crate) fn save_proposal(&mut self, inp: ProposalInput) -> Result<String, String> {
        let id = rl_domain::ids::new_uuid();
        let v = Value::obj([
            ("id", id.clone().into()),
            ("created_utc", now_utc_ms().into()),
            ("kind", inp.kind.into()),
            ("status", "pending".into()),
            ("source", inp.source.into()),
            ("provider", inp.provider.into()),
            ("model", inp.model.into()),
            ("explanation", inp.explanation.into()),
            ("fallback_reason", inp.fallback_reason.into()),
            ("base", inp.base.map(|(i, v)| Value::obj([("plan_id", i.into()), ("version", v.into())])).unwrap_or(Value::Null)),
            ("changes", inp.changes),
            ("plan", inp.plan.to_json_full()),
            ("input_summary", inp.input_summary),
            ("repaired", inp.repaired.into()),
            ("first_issues", inp.first_issues),
            ("policy_version", policy::POLICY_VERSION.into()),
        ]);
        self.store.save_proposal(&id, &v)?;
        Ok(id)
    }

    fn session_view(&self, plan: &TrainingPlan) -> Value {
        Value::Arr(
            plan.sessions
                .iter()
                .map(|s| {
                    let mut v = s.to_json();
                    let w = self.workout(&s.workout_id);
                    v.set("workout_name", w.map(|w| w.name.clone()));
                    v.set("category", w.map(|w| w.category.label()));
                    v.set("profile", w.map(|w| w.summary_json().get("profile").cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null));
                    let done = self.completions.get(&s.id).and_then(|a| a.as_str()).map(|a| a.to_string());
                    let status = if done.is_some() {
                        "completed"
                    } else if s.status == SessionStatus::Skipped {
                        "skipped"
                    } else if s.date < self.today() {
                        "missed"
                    } else {
                        "planned"
                    };
                    v.set("display_status", status);
                    v.set("completed_activity", done);
                    v.set("alternative_names", Value::Arr(s.alternatives.iter().map(|a| Value::from(self.workout(a).map(|w| w.name.clone()).unwrap_or_else(|| a.clone()))).collect()));
                    v
                })
                .collect(),
        )
    }

    pub(crate) fn get_plan(&mut self, _p: &Value) -> R {
        let plan = self.store.current_plan()?;
        let proposals: Vec<Value> = self
            .store
            .proposals()?
            .into_iter()
            .filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("pending"))
            .map(|mut p| {
                // Listing omits the bulky plan copy.
                if let Value::Obj(o) = &mut p {
                    o.retain(|(k, _)| k != "plan" && k != "input_summary");
                }
                p
            })
            .collect();
        let log: Vec<Value> = self.store.plan_log().into_iter().rev().take(20).collect();
        let can_undo = self.undo_target()?.is_some();
        Ok(Value::obj([
            ("plan", plan.as_ref().map(|p| {
                let mut v = p.to_json_full();
                v.set("sessions", self.session_view(p));
                v
            }).unwrap_or(Value::Null)),
            ("pending", Value::Arr(proposals)),
            ("log", Value::Arr(log)),
            ("can_undo", can_undo.into()),
            ("today", self.today().to_string().into()),
            ("ai_mode", self.settings.ai.provider.clone().into()),
        ]))
    }

    pub(crate) fn get_proposal(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let mut v = self.store.proposal(id)?.ok_or("Unknown proposal.")?;
        if let Some(plan) = v.get("plan").and_then(|pl| TrainingPlan::from_json(pl).ok()) {
            if let Value::Obj(_) = v {
                let mut pv = plan.to_json_full();
                pv.set("sessions", self.session_view(&plan));
                v.set("plan", pv);
            }
        }
        Ok(v)
    }

    pub(crate) fn propose_plan(&mut self, p: &Value) -> R {
        let profile = self.profile().ok_or("Complete onboarding first.")?;
        let weeks = p.opt_i64("weeks")?.unwrap_or(policy::DEFAULT_PLAN_WEEKS as i64).clamp(1, policy::MAX_PLAN_WEEKS as i64) as u32;
        let start = match popt_str(p, "start", 10)? {
            Some(s) => Date::parse(s).ok_or("Invalid start date.")?,
            None => self.today(),
        };
        let use_ai = p.bool_or("use_ai", true);
        let mut provider = if use_ai { self.ai_provider()? } else { None };
        let mut gate_note: Option<String> = None;
        if provider.is_some() {
            if let Err(e) = self.ai_gate(&profile) {
                gate_note = Some(format!("Offline plan used: {e}"));
                provider = None;
            }
        }
        let inline = provider.is_none() || p.bool_or("sync", false);
        let ftp = self.current_ftp();
        let recent = if profile.ai_consent.share_activity_summaries { self.recent_briefs() } else { vec![] };
        let library = self.library.clone();
        let today = self.today();
        let http = self.http.clone();
        let limits = self.ai_limits();
        let base = self.store.current_plan_pointer()?;
        let run = move |cancel: &AtomicBool| {
            let ctx = PlanContext { profile: &profile, ftp_w: ftp.as_ref().map(|f| f.watts), library: &library, today, start, weeks, recent_actual_min: None };
            let ai: Option<(&dyn AiProvider, &dyn rl_net::http::HttpClient)> = provider.as_ref().map(|p| (p as &dyn AiProvider, http.as_ref()));
            propose_plan(&ctx, ftp.as_ref(), &recent, ai, &limits, cancel)
        };
        let finish = move |app: &mut App, r: rl_coach::coach::PlanResult| -> R {
            let fallback = r.fallback_reason.clone().or(gate_note.clone());
            let id = app.save_proposal(ProposalInput {
                kind: "initial",
                provider: r.plan.provider.clone(),
                model: r.plan.model.clone(),
                plan: r.plan,
                source: r.source,
                explanation: r.explanation,
                fallback_reason: fallback,
                base: base.clone(),
                changes: Value::Arr(vec![]),
                input_summary: r.input_summary,
                repaired: r.repaired,
                first_issues: r.first_issues.to_json(),
            })?;
            Ok(Value::obj([("proposal_id", id.into())]))
        };
        if inline {
            let r = run(&AtomicBool::new(false));
            return finish(self, r);
        }
        let job = self.spawn_job("propose_plan", move |ctx| {
            ctx.progress(0, 1, "Asking the AI coach…");
            let r = run(&ctx.cancel);
            if ctx.cancelled() {
                return Err("Cancelled.".into());
            }
            ctx.with_app(|a| finish(a, r)).unwrap_or_else(|| Err("App closed.".into()))
        })?;
        Ok(Value::obj([("job_id", job.into())]))
    }

    /// The accepted-plan log entry that `undoPlan` would revert.
    fn undo_target(&self) -> Result<Option<Value>, String> {
        let cur = self.store.current_plan_pointer()?;
        let Some((cid, cv)) = cur else { return Ok(None) };
        Ok(self
            .store
            .plan_log()
            .into_iter()
            .rev()
            .find(|e| {
                e.get("action").and_then(|a| a.as_str()) == Some("accept")
                    && e.get("new").and_then(|n| n.get("plan_id")).and_then(|x| x.as_str()) == Some(cid.as_str())
                    && e.get("new").and_then(|n| n.get("version")).and_then(|x| x.as_i64()) == Some(cv as i64)
            }))
    }

    pub(crate) fn accept_proposal(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let mut prop = self.store.proposal(id)?.ok_or("Unknown proposal.")?;
        if prop.get("status").and_then(|s| s.as_str()) != Some("pending") {
            return Err("This proposal is no longer pending.".into());
        }
        let plan = TrainingPlan::from_json(prop.req("plan")?)?;
        let cur = self.store.current_plan_pointer()?;
        if let Some(b) = prop.get("base").filter(|b| !b.is_null()) {
            let base = (b.req_str("plan_id")?.to_string(), b.req_i64("version")? as u32);
            if prop.get("kind").and_then(|k| k.as_str()) != Some("initial") && cur.as_ref() != Some(&base) {
                return Err("Your plan changed since this proposal was made. Create a new proposal.".into());
            }
        }
        // Re-validate at acceptance time (policy or schedule may have changed).
        let profile = self.profile().ok_or("No profile.")?;
        let today = self.today();
        let issues = {
            let lib = &self.library;
            rl_domain::plan::validate_plan(&plan, &profile, today, &|wid| lib.iter().find(|w| w.id == wid))
        };
        if !issues.is_empty() {
            return Err(format!("This proposal no longer passes validation: {}", issues[0].message));
        }
        self.store.save_plan_version(&plan)?;
        self.store.set_current_plan(Some((&plan.id, plan.version)))?;
        prop.set("status", "accepted");
        prop.set("decided_utc", now_utc_ms());
        self.store.save_proposal(id, &prop)?;
        self.store.append_plan_log(&Value::obj([
            ("action", "accept".into()),
            ("utc", now_utc_ms().into()),
            ("proposal_id", id.into()),
            ("prev", cur.map(|(i, v)| Value::obj([("plan_id", i.into()), ("version", v.into())])).unwrap_or(Value::Null)),
            ("new", Value::obj([("plan_id", plan.id.clone().into()), ("version", plan.version.into())])),
            ("explanation", prop.get("explanation").cloned().unwrap_or(Value::Null)),
        ]))?;
        // Supersede other pending proposals built on the old plan.
        for mut other in self.store.proposals()? {
            if other.get("status").and_then(|s| s.as_str()) == Some("pending") {
                if let Some(oid) = other.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()) {
                    if oid != id && other.get("kind").and_then(|k| k.as_str()) != Some("initial") {
                        other.set("status", "superseded");
                        self.store.save_proposal(&oid, &other)?;
                    }
                }
            }
        }
        Ok(Value::Null)
    }

    pub(crate) fn reject_proposal(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let mut prop = self.store.proposal(id)?.ok_or("Unknown proposal.")?;
        prop.set("status", "rejected");
        prop.set("decided_utc", now_utc_ms());
        self.store.save_proposal(id, &prop)?;
        self.store.append_plan_log(&Value::obj([("action", "reject".into()), ("utc", now_utc_ms().into()), ("proposal_id", id.into())]))?;
        Ok(Value::Null)
    }

    pub(crate) fn undo_plan(&mut self, _p: &Value) -> R {
        let e = self.undo_target()?.ok_or("Nothing to undo.")?;
        let prev = e.get("prev").filter(|v| !v.is_null()).map(|v| (v.str_or("plan_id", "").to_string(), v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as u32));
        match &prev {
            Some((i, v)) => self.store.set_current_plan(Some((i, *v)))?,
            None => self.store.set_current_plan(None)?,
        }
        let cur = e.get("new").cloned().unwrap_or(Value::Null);
        self.store.append_plan_log(&Value::obj([("action", "undo".into()), ("utc", now_utc_ms().into()), ("undone", cur), ("restored", e.get("prev").cloned().unwrap_or(Value::Null))]))?;
        // Re-open: mark the undone proposal so the history shows it.
        Ok(Value::Null)
    }

    fn adaptation_proposal(&mut self, kind: &'static str, plan: &TrainingPlan, changes: Vec<Change>, explanation: String, source: &str) -> Result<Option<String>, String> {
        if changes.is_empty() {
            return Ok(None);
        }
        let profile = self.profile().ok_or("No profile.")?;
        let today = self.today();
        let newp = {
            let ctx = PlanContext { profile: &profile, ftp_w: self.current_ftp().map(|f| f.watts), library: &self.library, today, start: plan.start, weeks: plan.weeks, recent_actual_min: None };
            let r = rl_coach::adapt::apply_changes(plan, &changes, &ctx);
            match r {
                Ok(np) => (np, changes_json(&changes, plan, &ctx)),
                Err(issues) => return Err(format!("That change isn't allowed: {}", issues.first().map(|i| i.message.clone()).unwrap_or_default())),
            }
        };
        let (mut np, cj) = newp;
        np.rationale = explanation.clone();
        let id = self.save_proposal(ProposalInput {
            kind,
            plan: np,
            source: source.into(),
            explanation,
            fallback_reason: None,
            base: Some((plan.id.clone(), plan.version)),
            changes: cj,
            input_summary: Value::Null,
            provider: String::new(),
            model: String::new(),
            repaired: false,
            first_issues: Value::Arr(vec![]),
        })?;
        Ok(Some(id))
    }

    pub(crate) fn preview_change(&mut self, p: &Value) -> R {
        let plan = self.store.current_plan()?.ok_or("No active plan.")?;
        let changes: Vec<Change> = rl_json::field(p, "changes")?;
        if changes.is_empty() || changes.len() > 20 {
            return Err("Provide 1–20 changes.".into());
        }
        let why = popt_str(p, "reason", 300)?.unwrap_or("Your edit.").to_string();
        let id = self.adaptation_proposal("edit", &plan, changes, why, "rider")?;
        Ok(Value::obj([("proposal_id", id.into())]))
    }

    pub(crate) fn readiness_check(&mut self, p: &Value) -> R {
        let r = Readiness::from_json(p.req("readiness")?)?;
        for x in [r.sleep, r.fatigue, r.soreness, r.stress] {
            if !(1..=5).contains(&x) {
                return Err("Readiness values must be 1–5.".into());
            }
        }
        let today = self.today();
        let profile = self.profile().ok_or("No profile.")?;
        let Some(plan) = self.store.current_plan()? else {
            let (adv, msg) = policy::evaluate_readiness(&r, rl_domain::workout::Intensity::Easy);
            return Ok(Value::obj([("advice", adv.to_json()), ("message", msg.into()), ("proposal_id", Value::Null)]));
        };
        let (adv, msg, changes) = {
            let ctx = PlanContext { profile: &profile, ftp_w: self.current_ftp().map(|f| f.watts), library: &self.library, today, start: plan.start, weeks: plan.weeks, recent_actual_min: None };
            readiness_changes(&plan, today, &r, &ctx)
        };
        let pid = self.adaptation_proposal("readiness", &plan, changes, msg.clone(), "rules")?;
        Ok(Value::obj([("advice", adv.to_json()), ("message", msg.into()), ("proposal_id", pid.into()), ("safety", (adv == policy::ReadinessAdvice::StopSeekCare).into())]))
    }

    pub(crate) fn submit_feedback(&mut self, p: &Value) -> R {
        let act = pstr(p, "activity_id", 64)?.to_string();
        let meta = self.store.activity_meta(&act)?.ok_or("Unknown activity.")?;
        let fb = Feedback {
            activity_id: act.clone(),
            session_id: meta.plan_session_id.clone(),
            date: Date::from_utc_ms(meta.start_utc, meta.tz_offset_min),
            rpe: p.req_i64("rpe")?.clamp(0, 255) as u8,
            fatigue: p.req_i64("fatigue")?.clamp(0, 255) as u8,
            enjoyment: p.req_i64("enjoyment")?.clamp(0, 255) as u8,
            notes: p.str_or("notes", "").chars().take(1000).collect(),
        };
        fb.validate()?;
        let symptoms = policy::mentions_warning_symptom(&fb.notes);
        self.store.save_feedback(&act, &fb.to_json())?;
        if symptoms {
            return Ok(Value::obj([("proposal_id", Value::Null), ("safety", policy::SAFETY_COPY.into())]));
        }
        if meta.demo {
            return Ok(Value::obj([("proposal_id", Value::Null), ("note", "Demo rides don't adapt your plan.".into())]));
        }
        let pid = self.adapt_now()?;
        Ok(Value::obj([("proposal_id", pid.into())]))
    }

    fn adapt_now(&mut self) -> Result<Option<String>, String> {
        let Some(plan) = self.store.current_plan()? else { return Ok(None) };
        let profile = self.profile().ok_or("No profile.")?;
        let today = self.today();
        let fb = self.all_feedback();
        let (reason, changes) = {
            let ctx = PlanContext { profile: &profile, ftp_w: self.current_ftp().map(|f| f.watts), library: &self.library, today, start: plan.start, weeks: plan.weeks, recent_actual_min: None };
            feedback_changes(&plan, &fb, today, &ctx)
        };
        // Completed sessions are not "missed".
        let changes: Vec<Change> = changes.into_iter().filter(|c| !(c.action == "skip" && self.completions.get(&c.session_id).is_some())).collect();
        if changes.is_empty() {
            return Ok(None);
        }
        self.adaptation_proposal("adaptation", &plan, changes, reason, "rules")
    }

    pub(crate) fn adapt_plan(&mut self, _p: &Value) -> R {
        let id = self.adapt_now()?;
        Ok(Value::obj([("proposal_id", id.clone().into()), ("message", if id.is_none() { "No changes suggested right now." } else { "Review the suggested changes." }.into())]))
    }

    pub(crate) fn coach_history(&mut self, _p: &Value) -> R {
        Ok(Value::Arr(self.store.chat_history(100)))
    }

    pub(crate) fn coach_chat(&mut self, p: &Value) -> R {
        let msg = pstr(p, "message", 2000)?.trim().to_string();
        if msg.is_empty() {
            return Err("Type a message.".into());
        }
        let profile = self.profile().ok_or("Complete onboarding first.")?;
        self.store.append_chat(&Value::obj([("role", "rider".into()), ("text", msg.clone().into()), ("utc", now_utc_ms().into())]))?;
        let history: Vec<(String, String)> = self
            .store
            .chat_history(12)
            .iter()
            .rev()
            .skip(1)
            .rev()
            .map(|v| (v.str_or("role", "").to_string(), v.str_or("text", "").to_string()))
            .collect();
        let plan = self.store.current_plan()?;
        let mut provider = self.ai_provider().unwrap_or(None);
        let mut gate_note: Option<String> = None;
        if provider.is_some() && !policy::mentions_warning_symptom(&msg) {
            if let Err(e) = self.ai_gate(&profile) {
                gate_note = Some(e);
                provider = None;
            }
        }
        let inline = provider.is_none() || policy::mentions_warning_symptom(&msg);
        let library = self.library.clone();
        let today = self.today();
        let ftp = self.current_ftp().map(|f| f.watts);
        let http = self.http.clone();
        let limits = self.ai_limits();
        let feedback = self.all_feedback();
        let plan_c = plan.clone();
        let run = move |cancel: &AtomicBool| {
            let (start, weeks) = plan_c.as_ref().map(|p| (p.start, p.weeks)).unwrap_or((today, 4));
            let ctx = PlanContext { profile: &profile, ftp_w: ftp, library: &library, today, start, weeks, recent_actual_min: None };
            let ai: Option<(&dyn AiProvider, &dyn rl_net::http::HttpClient)> = provider.as_ref().map(|p| (p as &dyn AiProvider, http.as_ref()));
            chat(&ctx, plan_c.as_ref(), &feedback, &msg, &history, ai, &limits, cancel)
        };
        let finish = move |app: &mut App, r: rl_coach::coach::ChatResult| -> R {
            let mut pid: Option<String> = None;
            let mut note = r.error.clone().or(gate_note.clone());
            if !r.changes.is_empty() {
                if let Some(pl) = app.store.current_plan()? {
                    match app.adaptation_proposal("chat", &pl, r.changes.clone(), if r.change_reason.is_empty() { r.reply.clone() } else { r.change_reason.clone() }, &r.source) {
                        Ok(id) => pid = id,
                        Err(e) => note = Some(format!("The suggested change was not valid and was discarded: {e}")),
                    }
                }
            }
            app.store.append_chat(&Value::obj([
                ("role", "coach".into()),
                ("text", r.reply.clone().into()),
                ("source", r.source.clone().into()),
                ("proposal_id", pid.clone().into()),
                ("note", note.clone().into()),
                ("safety", r.safety_stop.into()),
                ("utc", now_utc_ms().into()),
            ]))?;
            Ok(Value::obj([("reply", r.reply.into()), ("source", r.source.into()), ("proposal_id", pid.into()), ("note", note.into()), ("safety", r.safety_stop.into())]))
        };
        if inline {
            let r = run(&AtomicBool::new(false));
            return finish(self, r);
        }
        let job = self.spawn_job("coach_chat", move |ctx| {
            ctx.progress(0, 1, "The coach is thinking…");
            let r = run(&ctx.cancel);
            ctx.with_app(|a| finish(a, r)).unwrap_or_else(|| Err("App closed.".into()))
        })?;
        Ok(Value::obj([("job_id", job.into())]))
    }

    pub(crate) fn test_ai(&mut self, _p: &Value) -> R {
        let provider = self.ai_provider()?.ok_or("AI coach is set to offline.")?;
        let http = self.http.clone();
        let job = self.spawn_job("test_ai", move |ctx| {
            ctx.progress(0, 2, "Listing models…");
            let models = provider.list_models(http.as_ref()).map_err(|e| e.user_message())?;
            ctx.progress(1, 2, "Sending a tiny test request…");
            let msgs = [ChatMessage::system("Reply with JSON only."), ChatMessage::user("Return {\"ok\":true}")];
            let r = provider.complete(http.as_ref(), &msgs, true, 30, Duration::from_secs(120), &ctx.cancel).map_err(|e| e.user_message())?;
            Ok(Value::obj([
                ("models", Value::Arr(models.into_iter().map(Value::from).collect())),
                ("model", r.model.into()),
                ("latency_ms", r.latency_ms.into()),
                ("reply", r.text.chars().take(200).collect::<String>().into()),
                ("local", provider.is_local().into()),
            ]))
        })?;
        Ok(Value::obj([("job_id", job.into())]))
    }

    pub(crate) fn summary_preview(&self) -> Value {
        let Some(profile) = self.profile() else { return Value::Null };
        let today = self.today();
        let ctx = PlanContext { profile: &profile, ftp_w: self.current_ftp().map(|f| f.watts), library: &self.library, today, start: today, weeks: 4, recent_actual_min: None };
        let recent = if profile.ai_consent.share_activity_summaries { self.recent_briefs() } else { vec![] };
        compact_summary(&ctx, self.current_ftp().as_ref(), &recent, None)
    }
}
