//! Plan changes and rule-based adaptation.
//!
//! Every change is expressed as an explicit `Change` list applied to a copy
//! of the plan, producing a *new version* that only takes effect after the
//! rider accepts it. Missed sessions are never stacked onto later days.

use crate::planner::{alternatives, PlanContext};
use rl_domain::plan::{is_recovery_week, validate_plan, PlannedSession, SessionStatus, TrainingPlan};
use rl_domain::policy::{self, evaluate_readiness, eligibility, Readiness, ReadinessAdvice};
use rl_domain::time::Date;
use rl_domain::workout::{Intensity, Issue};
use rl_json::{json_struct, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub session_id: String,
    /// "replace", "move", "shorten", "skip", "restore"
    pub action: String,
    pub workout_id: Option<String>,
    pub date: Option<Date>,
    pub keep_pct: Option<u8>,
    pub reason: String,
}
json_struct!(Change { session_id: "session_id", action: "action", workout_id: "workout_id", date: "date", keep_pct: "keep_pct", reason: "reason" = String::new() });

#[derive(Debug, Clone, PartialEq)]
pub struct Feedback {
    pub activity_id: String,
    pub session_id: Option<String>,
    pub date: Date,
    /// Session RPE 1–10.
    pub rpe: u8,
    /// 1 (fresh) … 5 (exhausted) after the ride.
    pub fatigue: u8,
    /// 1 … 5
    pub enjoyment: u8,
    pub notes: String,
}
json_struct!(Feedback {
    activity_id: "activity_id",
    session_id: "session_id",
    date: "date",
    rpe: "rpe",
    fatigue: "fatigue" = 3,
    enjoyment: "enjoyment" = 3,
    notes: "notes" = String::new(),
});

impl Feedback {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=10).contains(&self.rpe) || !(1..=5).contains(&self.fatigue) || !(1..=5).contains(&self.enjoyment) {
            return Err("Feedback values are out of range.".into());
        }
        if self.notes.chars().count() > 1000 {
            return Err("Notes are too long.".into());
        }
        Ok(())
    }
}

/// Apply changes to a copy of the plan (version + 1). Validates the result.
pub fn apply_changes(plan: &TrainingPlan, changes: &[Change], ctx: &PlanContext) -> Result<TrainingPlan, Vec<Issue>> {
    let mut p = plan.clone();
    p.version = plan.version + 1;
    p.created_utc = rl_domain::time::now_utc_ms();
    let rules = policy::rules_for(p.level);
    let mut issues = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        let path = format!("changes[{i}]");
        let Some(idx) = p.sessions.iter().position(|s| s.id == c.session_id) else {
            issues.push(Issue { path, message: "Unknown session.".into() });
            continue;
        };
        if p.sessions[idx].status == SessionStatus::Completed {
            issues.push(Issue { path, message: "Completed sessions cannot be changed.".into() });
            continue;
        }
        let week = p.week_of(p.sessions[idx].date).unwrap_or(0);
        match c.action.as_str() {
            "replace" => {
                let Some(w) = c.workout_id.as_deref().and_then(|id| ctx.lookup(id)) else {
                    issues.push(Issue { path, message: "Unknown workout.".into() });
                    continue;
                };
                let date = p.sessions[idx].date;
                let why = format!("{} {}", c.reason.trim(), w.purpose).trim().to_string();
                let avail = ctx.profile.availability_min[date.weekday() as usize] * 60;
                let s = &mut p.sessions[idx];
                s.workout_id = w.id.clone();
                s.duration_s = w.total_s();
                s.intensity = w.intensity();
                s.difficulty = w.difficulty;
                s.purpose = w.purpose.clone();
                s.why = why.chars().take(400).collect();
                s.keep_pct = 100;
                s.alternatives = alternatives(ctx, &rules, w, week, avail);
            }
            "move" => {
                let Some(d) = c.date else {
                    issues.push(Issue { path, message: "Missing date.".into() });
                    continue;
                };
                p.sessions[idx].date = d;
            }
            "shorten" => {
                let k = c.keep_pct.unwrap_or(70).clamp(50, 100);
                let w = ctx.lookup(&p.sessions[idx].workout_id);
                let full = w.map(|w| w.total_s()).unwrap_or(p.sessions[idx].duration_s);
                p.sessions[idx].keep_pct = k;
                p.sessions[idx].duration_s = (full as u64 * k as u64 / 100) as u32;
            }
            "skip" => p.sessions[idx].status = SessionStatus::Skipped,
            "restore" => p.sessions[idx].status = SessionStatus::Planned,
            _ => issues.push(Issue { path, message: format!("Unknown action '{}'.", c.action) }),
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    p.sessions.sort_by_key(|s| s.date);
    // Skipped sessions stay in the plan and still count towards weekly limits
    // (conservative: a skipped hard day cannot be made up later that week).
    let v = validate_plan(&p, ctx.profile, ctx.today, &|id| ctx.lookup(id));
    if v.is_empty() {
        Ok(p)
    } else {
        Err(v)
    }
}

fn upcoming<'p>(plan: &'p TrainingPlan, today: Date) -> impl Iterator<Item = &'p PlannedSession> {
    plan.sessions.iter().filter(move |s| s.date >= today && s.status == SessionStatus::Planned)
}

/// Easier replacement for a session: an eligible easier workout that fits.
fn easier(plan: &TrainingPlan, s: &PlannedSession, ctx: &PlanContext, max_intensity: Intensity) -> Option<String> {
    let rules = policy::rules_for(plan.level);
    let week = plan.week_of(s.date).unwrap_or(0);
    let avail = ctx.profile.availability_min[s.date.weekday() as usize] * 60;
    let mut c: Vec<_> = ctx
        .library
        .iter()
        .filter(|w| w.intensity() <= max_intensity && w.intensity() < s.intensity.max(Intensity::Moderate) && w.total_s() <= avail.min(s.duration_s.max(1800)))
        .filter(|w| eligibility(w, &rules, week, is_recovery_week(week)).is_ok())
        .collect();
    c.sort_by_key(|w| (std::cmp::Reverse(w.intensity()), std::cmp::Reverse(w.total_s())));
    // Prefer the session's own listed alternatives.
    for a in &s.alternatives {
        if let Some(w) = ctx.lookup(a) {
            if w.intensity() <= max_intensity && w.intensity() < s.intensity.max(Intensity::Moderate) {
                return Some(w.id.clone());
            }
        }
    }
    c.first().map(|w| w.id.clone())
}

/// Proposal for today's session from the pre-ride readiness check.
pub fn readiness_changes(plan: &TrainingPlan, today: Date, r: &Readiness, ctx: &PlanContext) -> (ReadinessAdvice, String, Vec<Change>) {
    let todays = plan.sessions.iter().find(|s| s.date == today && s.status == SessionStatus::Planned);
    let intensity = todays.map(|s| s.intensity).unwrap_or(Intensity::Easy);
    let (advice, msg) = evaluate_readiness(r, intensity);
    let Some(s) = todays else { return (advice, msg, vec![]) };
    let changes = match advice {
        ReadinessAdvice::Proceed => vec![],
        ReadinessAdvice::Shorten => vec![Change { session_id: s.id.clone(), action: "shorten".into(), workout_id: None, date: None, keep_pct: Some(70), reason: msg.clone() }],
        ReadinessAdvice::SwapEasier => match easier(plan, s, ctx, Intensity::Easy) {
            Some(w) => vec![Change { session_id: s.id.clone(), action: "replace".into(), workout_id: Some(w), date: None, keep_pct: None, reason: msg.clone() }],
            None => vec![Change { session_id: s.id.clone(), action: "shorten".into(), workout_id: None, date: None, keep_pct: Some(60), reason: msg.clone() }],
        },
        ReadinessAdvice::Rest | ReadinessAdvice::StopSeekCare => vec![Change { session_id: s.id.clone(), action: "skip".into(), workout_id: None, date: None, keep_pct: None, reason: msg.clone() }],
    };
    (advice, msg, changes)
}

/// Rule-based adaptation from recent post-ride feedback and missed sessions.
pub fn feedback_changes(plan: &TrainingPlan, feedback: &[Feedback], today: Date, ctx: &PlanContext) -> (String, Vec<Change>) {
    let mut changes = Vec::new();
    let mut reasons = Vec::new();
    // Missed sessions: mark as skipped, never stack them later.
    let missed: Vec<&PlannedSession> = plan.sessions.iter().filter(|s| s.date < today && s.status == SessionStatus::Planned).collect();
    for m in &missed {
        changes.push(Change { session_id: m.id.clone(), action: "skip".into(), workout_id: None, date: None, keep_pct: None, reason: "Missed; not rescheduled to avoid stacking sessions.".into() });
    }
    if !missed.is_empty() {
        reasons.push(format!("{} missed session(s) are marked skipped rather than squeezed into later days.", missed.len()));
    }
    let mut recent: Vec<&Feedback> = feedback.iter().filter(|f| today.days_until(&f.date) >= -10 && f.date <= today).collect();
    recent.sort_by_key(|f| f.date);
    let last = recent.last();
    let hard_struggle = recent.iter().rev().take(2).filter(|f| f.rpe >= 9 || f.fatigue >= 4).count() >= 2;
    let very_tired = last.map(|f| f.fatigue >= 5).unwrap_or(false);
    let too_easy = recent.len() >= 2 && recent.iter().rev().take(2).all(|f| f.rpe <= 4 && f.fatigue <= 2);
    if very_tired {
        if let Some(next) = upcoming(plan, today).find(|s| today.days_until(&s.date) <= 1) {
            if let Some(w) = easier(plan, next, ctx, Intensity::Easy) {
                changes.push(Change { session_id: next.id.clone(), action: "replace".into(), workout_id: Some(w), date: None, keep_pct: None, reason: "You reported being exhausted after your last ride.".into() });
                reasons.push("Your last ride left you exhausted, so the next session becomes an easy one.".into());
            }
        }
    }
    if hard_struggle {
        if let Some(next) = upcoming(plan, today).find(|s| s.intensity >= Intensity::Hard && !changes.iter().any(|c| c.session_id == s.id)) {
            if let Some(w) = easier(plan, next, ctx, Intensity::Moderate) {
                changes.push(Change { session_id: next.id.clone(), action: "replace".into(), workout_id: Some(w), date: None, keep_pct: None, reason: "Recent sessions felt very hard.".into() });
                reasons.push("Your last two rides felt very hard, so the next key session is swapped for a more manageable one.".into());
            }
        }
    } else if too_easy {
        let rules = policy::rules_for(plan.level);
        if let Some(next) = upcoming(plan, today).find(|s| s.intensity >= Intensity::Moderate) {
            let cur = ctx.lookup(&next.workout_id);
            let week = plan.week_of(next.date).unwrap_or(0);
            let avail = ctx.profile.availability_min[next.date.weekday() as usize] * 60;
            if let Some(cur) = cur {
                let harder = ctx
                    .library
                    .iter()
                    .filter(|w| w.family == cur.family && w.id != cur.id && w.difficulty >= cur.difficulty && w.total_s() >= cur.total_s() && w.total_s() <= avail)
                    .filter(|w| eligibility(w, &rules, week, is_recovery_week(week)).is_ok())
                    .min_by_key(|w| (w.difficulty, w.total_s()));
                if let Some(h) = harder {
                    changes.push(Change { session_id: next.id.clone(), action: "replace".into(), workout_id: Some(h.id.clone()), date: None, keep_pct: None, reason: "Recent sessions felt comfortable.".into() });
                    reasons.push("Your recent sessions felt comfortable, so the next quality session steps up within the same family (still within policy limits).".into());
                }
            }
        }
    }
    (reasons.join(" "), changes)
}

pub fn changes_json(changes: &[Change], plan: &TrainingPlan, ctx: &PlanContext) -> Value {
    Value::Arr(
        changes
            .iter()
            .map(|c| {
                let s = plan.session(&c.session_id);
                let from_name = s.and_then(|s| ctx.lookup(&s.workout_id)).map(|w| w.name.clone());
                let to_name = c.workout_id.as_deref().and_then(|id| ctx.lookup(id)).map(|w| w.name.clone());
                let mut v = rl_json::ToJson::to_json(c);
                v.set("date_before", Value::from(s.map(|s| s.date.to_string())));
                v.set("workout_before", Value::from(from_name));
                v.set("workout_after", Value::from(to_name));
                v
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::build_plan;
    use rl_domain::library::builtin_workouts;
    use rl_domain::rider::RiderProfile;

    fn setup() -> (Vec<rl_domain::workout::Workout>, RiderProfile, Date) {
        let mut p = RiderProfile::demo();
        p.experience = rl_domain::rider::Experience::Experienced;
        p.consistent_weeks = 10;
        p.recent_weekly_min = 300;
        (builtin_workouts(), p, Date::parse("2026-10-05").unwrap())
    }

    #[test]
    fn readiness_swaps_hard_session() {
        let (lib, p, today) = setup();
        let ctx = PlanContext { profile: &p, ftp_w: Some(250.0), library: &lib, today, start: today, weeks: 4, recent_actual_min: None };
        let plan = build_plan(&ctx);
        let hard = plan.sessions.iter().find(|s| s.intensity >= Intensity::Hard).expect("experienced rider has hard sessions").clone();
        let ctx2 = PlanContext { today: hard.date, ..ctx };
        let r = Readiness { sleep: 2, ..Default::default() };
        let (advice, _, changes) = readiness_changes(&plan, hard.date, &r, &ctx2);
        assert_eq!(advice, ReadinessAdvice::SwapEasier);
        assert_eq!(changes.len(), 1);
        let newp = apply_changes(&plan, &changes, &ctx2).unwrap();
        assert_eq!(newp.version, plan.version + 1);
        assert!(newp.session(&hard.id).unwrap().intensity < Intensity::Hard);
        assert_eq!(plan.session(&hard.id).unwrap().intensity, hard.intensity, "original untouched until accepted");
    }

    #[test]
    fn missed_sessions_are_skipped_not_stacked() {
        let (lib, p, today) = setup();
        let ctx = PlanContext { profile: &p, ftp_w: Some(250.0), library: &lib, today, start: today, weeks: 2, recent_actual_min: None };
        let plan = build_plan(&ctx);
        let later = today.add_days(8);
        let ctx2 = PlanContext { today: later, ..ctx };
        let (_, changes) = feedback_changes(&plan, &[], later, &ctx2);
        let n_missed = plan.sessions.iter().filter(|s| s.date < later).count();
        assert_eq!(changes.iter().filter(|c| c.action == "skip").count(), n_missed);
        assert!(changes.iter().all(|c| c.action != "move"));
        let newp = apply_changes(&plan, &changes, &ctx2).unwrap();
        assert_eq!(newp.sessions.len(), plan.sessions.len(), "nothing added later");
    }

    #[test]
    fn invalid_changes_are_rejected() {
        let (lib, p, today) = setup();
        let ctx = PlanContext { profile: &p, ftp_w: Some(250.0), library: &lib, today, start: today, weeks: 2, recent_actual_min: None };
        let plan = build_plan(&ctx);
        let s = &plan.sessions[0];
        // Move onto an unavailable day (Monday).
        let mon = s.date.week_start().add_days(7);
        let bad = vec![Change { session_id: s.id.clone(), action: "move".into(), workout_id: None, date: Some(mon), keep_pct: None, reason: String::new() }];
        assert!(apply_changes(&plan, &bad, &ctx).is_err());
        let bad = vec![Change { session_id: s.id.clone(), action: "replace".into(), workout_id: Some("nope".into()), date: None, keep_pct: None, reason: String::new() }];
        assert!(apply_changes(&plan, &bad, &ctx).is_err());
    }
}
