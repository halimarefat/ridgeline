//! Training plans, planned sessions and local plan validation.
//!
//! Every plan — from the offline planner, from an AI provider, or edited by
//! the rider — passes [`validate_plan`] before it is shown as a proposal.

use crate::policy::{self, eligibility, Level, LevelRules};
use crate::rider::RiderProfile;
use crate::time::Date;
use crate::workout::{Category, Intensity, Issue, Workout};
use rl_json::{json_enum, json_struct, ToJson, Value};

pub const PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Planned,
    Completed,
    Skipped,
}
json_enum!(SessionStatus { Planned = "planned", Completed = "completed", Skipped = "skipped" });

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedSession {
    pub id: String,
    pub date: Date,
    pub workout_id: String,
    pub duration_s: u32,
    pub intensity: Intensity,
    /// "power" when FTP-based targets apply, "rpe" for perceived effort.
    pub target_basis: String,
    pub purpose: String,
    pub why: String,
    pub difficulty: u8,
    pub alternatives: Vec<String>,
    pub status: SessionStatus,
    pub activity_id: Option<String>,
    /// Rider-requested shortening, percent of the original duration kept.
    pub keep_pct: u8,
}
json_struct!(PlannedSession {
    id: "id",
    date: "date",
    workout_id: "workout_id",
    duration_s: "duration_s",
    intensity: "intensity",
    target_basis: "target_basis" = "power".to_string(),
    purpose: "purpose" = String::new(),
    why: "why" = String::new(),
    difficulty: "difficulty" = 1,
    alternatives: "alternatives" = Vec::new(),
    status: "status" = SessionStatus::Planned,
    activity_id: "activity_id",
    keep_pct: "keep_pct" = 100,
});

#[derive(Debug, Clone, PartialEq)]
pub struct WeekSummary {
    pub index: u32,
    pub start: Date,
    pub minutes: u32,
    pub sessions: u32,
    pub hard: u32,
    pub moderate: u32,
    pub easy: u32,
    pub recovery_week: bool,
}
json_struct!(WeekSummary {
    index: "index",
    start: "start",
    minutes: "minutes",
    sessions: "sessions",
    hard: "hard",
    moderate: "moderate",
    easy: "easy",
    recovery_week: "recovery_week",
});

#[derive(Debug, Clone, PartialEq)]
pub struct TrainingPlan {
    pub schema: u32,
    pub id: String,
    pub version: u32,
    pub created_utc: i64,
    pub start: Date,
    pub weeks: u32,
    pub level: Level,
    pub policy_version: String,
    /// "offline" (deterministic planner) or "ai".
    pub source: String,
    pub provider: String,
    pub model: String,
    pub rationale: String,
    pub sessions: Vec<PlannedSession>,
}
json_struct!(TrainingPlan {
    schema: "schema" = PLAN_SCHEMA_VERSION,
    id: "id",
    version: "version" = 1,
    created_utc: "created_utc" = 0,
    start: "start",
    weeks: "weeks",
    level: "level",
    policy_version: "policy_version",
    source: "source",
    provider: "provider" = String::new(),
    model: "model" = String::new(),
    rationale: "rationale" = String::new(),
    sessions: "sessions",
});

pub fn is_recovery_week(week: u32) -> bool {
    (week + 1) % policy::RECOVERY_WEEK_EVERY == 0
}

impl TrainingPlan {
    pub fn end_exclusive(&self) -> Date {
        self.start.add_days(7 * self.weeks as i64)
    }
    pub fn week_of(&self, d: Date) -> Option<u32> {
        let k = self.start.days_until(&d);
        if k < 0 || k >= 7 * self.weeks as i64 {
            None
        } else {
            Some((k / 7) as u32)
        }
    }
    pub fn week_summaries(&self) -> Vec<WeekSummary> {
        (0..self.weeks)
            .map(|wk| {
                let ss: Vec<_> = self.sessions.iter().filter(|s| self.week_of(s.date) == Some(wk)).collect();
                WeekSummary {
                    index: wk,
                    start: self.start.add_days(7 * wk as i64),
                    minutes: ss.iter().map(|s| s.duration_s / 60).sum(),
                    sessions: ss.len() as u32,
                    hard: ss.iter().filter(|s| s.intensity >= Intensity::Hard).count() as u32,
                    moderate: ss.iter().filter(|s| s.intensity == Intensity::Moderate).count() as u32,
                    easy: ss.iter().filter(|s| s.intensity == Intensity::Easy).count() as u32,
                    recovery_week: is_recovery_week(wk),
                }
            })
            .collect()
    }
    pub fn session(&self, id: &str) -> Option<&PlannedSession> {
        self.sessions.iter().find(|s| s.id == id)
    }
    pub fn to_json_full(&self) -> Value {
        let mut v = self.to_json();
        v.set("weeks_summary", self.week_summaries().to_json());
        v.set("end", self.end_exclusive().add_days(-1).to_json());
        v
    }
}

/// Maximum planned minutes for week `wk` given the previous week's minutes.
pub fn volume_cap(rules: &LevelRules, profile: &RiderProfile, wk: u32, prev_minutes: Option<u32>) -> u32 {
    match prev_minutes {
        None => (profile.recent_weekly_min as f64 * rules.first_week_factor) as u32 + rules.first_week_bonus_min,
        Some(prev) if is_recovery_week(wk) => (prev as f64 * rules.recovery_week_factor) as u32 + 10,
        Some(prev) => (prev as f64 * (1.0 + rules.weekly_growth)) as u32 + 20,
    }
}

fn clean_text(s: &str) -> bool {
    s.chars().count() <= 400 && !s.chars().any(|c| c.is_control() && c != '\n')
}

/// Validate a plan against the rider's schedule and the coaching policy.
/// `today` is the rider's local date; `lookup` resolves workout ids.
pub fn validate_plan<'a>(plan: &TrainingPlan, profile: &RiderProfile, today: Date, lookup: &dyn Fn(&str) -> Option<&'a Workout>) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut add = |path: String, m: String| issues.push(Issue { path, message: m });
    let level = plan.level;
    let rules = policy::rules_for(level);
    if plan.weeks == 0 || plan.weeks > policy::MAX_PLAN_WEEKS {
        add("weeks".into(), format!("Plan length must be 1–{} weeks.", policy::MAX_PLAN_WEEKS));
        return issues;
    }
    if today.days_until(&plan.start) < -7 || today.days_until(&plan.start) > 60 {
        add("start".into(), "Plan must start within the last week or the next 60 days.".into());
    }
    if plan.policy_version != policy::POLICY_VERSION {
        add("policy_version".into(), format!("Plan was made for policy {}, current is {}.", plan.policy_version, policy::POLICY_VERSION));
    }
    if plan.rationale.chars().count() > 2000 || plan.rationale.chars().any(|c| c.is_control() && c != '\n') {
        add("rationale".into(), "Rationale is too long.".into());
    }
    let mut sessions: Vec<&PlannedSession> = plan.sessions.iter().collect();
    sessions.sort_by_key(|s| s.date);
    let mut prev_date: Option<Date> = None;
    let mut last_hard: Option<Date> = None;
    for (i, s) in sessions.iter().enumerate() {
        let p = format!("sessions[{i}]");
        let Some(wk) = plan.week_of(s.date) else {
            add(format!("{p}.date"), format!("{} is outside the plan dates.", s.date));
            continue;
        };
        if prev_date == Some(s.date) {
            add(format!("{p}.date"), format!("More than one session on {}.", s.date));
        }
        prev_date = Some(s.date);
        let wd = s.date.weekday() as usize;
        let avail = profile.availability_min.get(wd).copied().unwrap_or(0);
        if avail == 0 {
            add(format!("{p}.date"), format!("{} is a rest or unavailable day.", s.date));
        }
        let Some(w) = lookup(&s.workout_id) else {
            add(format!("{p}.workout_id"), format!("Unknown workout '{}'.", s.workout_id));
            continue;
        };
        if let Err(e) = eligibility(w, &rules, wk, is_recovery_week(wk)) {
            add(format!("{p}.workout_id"), format!("{}: {e}", w.name));
        }
        if w.intensity() != s.intensity {
            add(format!("{p}.intensity"), "Session intensity does not match the workout.".into());
        }
        let full = w.total_s();
        let expected = (full as u64 * s.keep_pct.clamp(1, 100) as u64 / 100) as u32;
        if s.keep_pct == 0 || s.keep_pct > 100 || s.duration_s.abs_diff(expected) > 60 {
            add(format!("{p}.duration_s"), "Session duration does not match the workout.".into());
        }
        if s.duration_s > avail * 60 && avail > 0 {
            add(format!("{p}.duration_s"), format!("{} min exceeds the {} min available on {}.", s.duration_s / 60, avail, crate::time::WEEKDAY_NAMES[wd]));
        }
        if profile.avoided_categories.contains(&w.category) && w.category != Category::Recovery {
            add(format!("{p}.workout_id"), format!("You asked to avoid {} sessions.", w.category.label().to_lowercase()));
        }
        if s.intensity >= Intensity::Hard {
            if let Some(h) = last_hard {
                if h.days_until(&s.date) < policy::MIN_DAYS_BETWEEN_HARD {
                    add(format!("{p}.date"), "Hard sessions need at least one easier day between them.".into());
                }
            }
            last_hard = Some(s.date);
        }
        for a in &s.alternatives {
            match lookup(a) {
                None => add(format!("{p}.alternatives"), format!("Unknown alternative '{a}'.")),
                Some(aw) if eligibility(aw, &rules, wk, is_recovery_week(wk)).is_err() => {
                    add(format!("{p}.alternatives"), format!("Alternative '{}' is not eligible.", aw.name))
                }
                _ => {}
            }
        }
        if !clean_text(&s.purpose) || !clean_text(&s.why) {
            add(format!("{p}.why"), "Session text is too long or contains control characters.".into());
        }
    }
    // Weekly checks.
    let mut prev_minutes: Option<u32> = None;
    for ws in plan.week_summaries() {
        let p = format!("week[{}]", ws.index);
        let hard_cap = if ws.recovery_week { policy::RECOVERY_WEEK_HARD } else { rules.hard_per_week };
        if ws.hard > hard_cap {
            add(p.clone(), format!("Week {} has {} hard sessions; the limit is {}.", ws.index + 1, ws.hard, hard_cap));
        }
        if ws.moderate > rules.moderate_per_week {
            add(p.clone(), format!("Week {} has {} moderate sessions; the limit is {}.", ws.index + 1, ws.moderate, rules.moderate_per_week));
        }
        let rest_days = 7 - ws.sessions.min(7);
        if rest_days < rules.min_rest_days_per_week {
            add(p.clone(), format!("Week {} needs at least {} rest days.", ws.index + 1, rules.min_rest_days_per_week));
        }
        let cap = volume_cap(&rules, profile, ws.index, prev_minutes);
        if ws.minutes > cap {
            add(p.clone(), format!("Week {} plans {} min; the progression limit is {} min.", ws.index + 1, ws.minutes, cap));
        }
        prev_minutes = Some(ws.minutes);
    }
    // Consecutive training days.
    let mut run = 0u32;
    let mut last: Option<Date> = None;
    for s in &sessions {
        run = match last {
            Some(d) if d.days_until(&s.date) == 1 => run + 1,
            _ => 1,
        };
        if run > rules.max_consecutive_days {
            add("sessions".into(), format!("More than {} training days in a row around {}.", rules.max_consecutive_days, s.date));
            break;
        }
        last = Some(s.date);
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_workouts;

    fn plan_with(sessions: Vec<(&str, &str)>, lib: &[Workout]) -> TrainingPlan {
        TrainingPlan {
            schema: 1,
            id: "p".into(),
            version: 1,
            created_utc: 0,
            start: Date::parse("2026-10-05").unwrap(),
            weeks: 4,
            level: Level::Developing,
            policy_version: policy::POLICY_VERSION.into(),
            source: "offline".into(),
            provider: String::new(),
            model: String::new(),
            rationale: String::new(),
            sessions: sessions
                .into_iter()
                .enumerate()
                .map(|(i, (d, id))| {
                    let w = lib.iter().find(|w| w.id == id).unwrap();
                    PlannedSession {
                        id: format!("s{i}"),
                        date: Date::parse(d).unwrap(),
                        workout_id: id.into(),
                        duration_s: w.total_s(),
                        intensity: w.intensity(),
                        target_basis: "power".into(),
                        purpose: String::new(),
                        why: String::new(),
                        difficulty: w.difficulty,
                        alternatives: vec![],
                        status: SessionStatus::Planned,
                        activity_id: None,
                        keep_pct: 100,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn valid_plan_passes_and_violations_fail() {
        let lib = builtin_workouts();
        let lookup = |id: &str| lib.iter().find(|w| w.id == id);
        let profile = RiderProfile::demo(); // Tue/Thu 60, Sat 90, Sun 45
        let today = Date::parse("2026-10-04").unwrap();
        let ok = plan_with(vec![("2026-10-06", "endurance-60"), ("2026-10-08", "recovery-spin-45"), ("2026-10-10", "endurance-90")], &lib);
        let issues = validate_plan(&ok, &profile, today, &lookup);
        assert!(issues.is_empty(), "{issues:?}");

        // Monday is unavailable; two hard sessions back to back; hard in intro week.
        let bad = plan_with(vec![("2026-10-05", "endurance-45"), ("2026-10-10", "threshold-4x5"), ("2026-10-11", "vo2-5x3")], &lib);
        let issues = validate_plan(&bad, &profile, today, &lookup);
        let msgs: Vec<_> = issues.iter().map(|i| i.message.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains("rest or unavailable")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("first weeks")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("easier day between")), "{msgs:?}");

        // Over the available time on Sunday (45 min) and unknown id.
        let mut bad = plan_with(vec![("2026-10-11", "endurance-90")], &lib);
        bad.sessions.push(PlannedSession { workout_id: "made-up".into(), date: Date::parse("2026-10-13").unwrap(), ..bad.sessions[0].clone() });
        let msgs: Vec<_> = validate_plan(&bad, &profile, today, &lookup).into_iter().map(|i| i.message).collect();
        assert!(msgs.iter().any(|m| m.contains("exceeds")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Unknown workout")), "{msgs:?}");
    }
}
