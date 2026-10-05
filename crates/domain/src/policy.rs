//! Versioned coaching policy: rider levels, intensity eligibility, recovery
//! spacing, progression limits, readiness rules and symptom exclusions.
//!
//! These are coaching rules, not medical diagnoses or guarantees. Every
//! numeric limit is listed with its rationale and source in
//! docs/coaching-policy.md. The AI coach may only explain and customise
//! *within* these limits; plan validation enforces them locally.
//!
//! Review status: limits were chosen conservatively from the cited sources by
//! the implementer. They have NOT yet been reviewed by a qualified coach or
//! clinician — that review is a release gate.

use crate::rider::{Experience, Goal, RiderProfile};
use crate::workout::{Category, Intensity, Workout};
use rl_json::{json_enum, json_struct, ToJson, Value};

pub const POLICY_VERSION: &str = "2026.10-draft.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Beginner,
    Developing,
    Returning,
    Experienced,
}
json_enum!(Level { Beginner = "beginner", Developing = "developing", Returning = "returning", Experienced = "experienced" });

impl Level {
    pub fn label(&self) -> &'static str {
        match self {
            Level::Beginner => "New to structured training",
            Level::Developing => "Building consistency",
            Level::Returning => "Returning after a break",
            Level::Experienced => "Experienced",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelRules {
    pub level: Level,
    /// Maximum hard (or maximal) sessions per week in build weeks.
    pub hard_per_week: u32,
    /// Weeks at the start of a plan with no hard sessions (adaptation).
    pub intro_weeks_without_hard: u32,
    pub moderate_per_week: u32,
    pub max_workout_difficulty: u8,
    pub allow_assessment: bool,
    pub allow_anaerobic: bool,
    pub min_rest_days_per_week: u32,
    pub max_consecutive_days: u32,
    /// Week-over-week planned-volume growth cap (fraction, 0.10 = 10%).
    pub weekly_growth: f64,
    /// First-week volume cap = recent weekly minutes × this + `first_week_bonus_min`.
    pub first_week_factor: f64,
    pub first_week_bonus_min: u32,
    /// Recovery week volume relative to the previous week.
    pub recovery_week_factor: f64,
}
json_struct!(LevelRules {
    level: "level",
    hard_per_week: "hard_per_week",
    intro_weeks_without_hard: "intro_weeks_without_hard",
    moderate_per_week: "moderate_per_week",
    max_workout_difficulty: "max_workout_difficulty",
    allow_assessment: "allow_assessment",
    allow_anaerobic: "allow_anaerobic",
    min_rest_days_per_week: "min_rest_days_per_week",
    max_consecutive_days: "max_consecutive_days",
    weekly_growth: "weekly_growth",
    first_week_factor: "first_week_factor",
    first_week_bonus_min: "first_week_bonus_min",
    recovery_week_factor: "recovery_week_factor",
});

pub fn rules_for(level: Level) -> LevelRules {
    match level {
        Level::Beginner => LevelRules {
            level,
            hard_per_week: 1,
            intro_weeks_without_hard: 2,
            moderate_per_week: 1,
            max_workout_difficulty: 3,
            allow_assessment: false,
            allow_anaerobic: false,
            min_rest_days_per_week: 2,
            max_consecutive_days: 2,
            weekly_growth: 0.10,
            first_week_factor: 1.0,
            first_week_bonus_min: 60,
            recovery_week_factor: 0.65,
        },
        Level::Developing => LevelRules {
            level,
            hard_per_week: 1,
            intro_weeks_without_hard: 1,
            moderate_per_week: 2,
            max_workout_difficulty: 4,
            allow_assessment: true,
            allow_anaerobic: false,
            min_rest_days_per_week: 2,
            max_consecutive_days: 3,
            weekly_growth: 0.10,
            first_week_factor: 1.1,
            first_week_bonus_min: 30,
            recovery_week_factor: 0.65,
        },
        Level::Returning => LevelRules {
            level,
            hard_per_week: 1,
            intro_weeks_without_hard: 2,
            moderate_per_week: 1,
            max_workout_difficulty: 3,
            allow_assessment: false,
            allow_anaerobic: false,
            min_rest_days_per_week: 2,
            max_consecutive_days: 3,
            weekly_growth: 0.10,
            first_week_factor: 1.0,
            first_week_bonus_min: 60,
            recovery_week_factor: 0.65,
        },
        Level::Experienced => LevelRules {
            level,
            hard_per_week: 2,
            intro_weeks_without_hard: 0,
            moderate_per_week: 2,
            max_workout_difficulty: 5,
            allow_assessment: true,
            allow_anaerobic: true,
            min_rest_days_per_week: 1,
            max_consecutive_days: 5,
            weekly_growth: 0.10,
            first_week_factor: 1.1,
            first_week_bonus_min: 0,
            recovery_week_factor: 0.65,
        },
    }
}

/// Minimum calendar days between two hard sessions (2 = at least one
/// non-hard day in between, i.e. ≥ 48 h).
pub const MIN_DAYS_BETWEEN_HARD: i64 = 2;
/// Hard sessions in a recovery week.
pub const RECOVERY_WEEK_HARD: u32 = 0;
/// Every Nth week of a plan is a recovery week.
pub const RECOVERY_WEEK_EVERY: u32 = 4;
/// Default plan length in weeks, and the allowed range.
pub const DEFAULT_PLAN_WEEKS: u32 = 4;
pub const MAX_PLAN_WEEKS: u32 = 12;

pub fn level_for(p: &RiderProfile) -> Level {
    if p.experience == Experience::New {
        return Level::Beginner;
    }
    if p.weeks_off >= 8 || p.goal == Goal::Returning {
        return Level::Returning;
    }
    if p.experience == Experience::Experienced && p.consistent_weeks >= 6 && p.recent_weekly_min >= 150 {
        return Level::Experienced;
    }
    if p.consistent_weeks >= 4 && p.recent_weekly_min >= 90 {
        Level::Developing
    } else {
        Level::Beginner
    }
}

/// Whether a workout may be scheduled for this rider in plan week `week`
/// (0-based). Returns the reason when not eligible.
pub fn eligibility(w: &Workout, rules: &LevelRules, week: u32, recovery_week: bool) -> Result<(), String> {
    if w.category == Category::TestFixture {
        return Err("Technical test workouts are not training sessions.".into());
    }
    if w.difficulty > rules.max_workout_difficulty {
        return Err(format!("Difficulty {} is above the limit ({}) for {}.", w.difficulty, rules.max_workout_difficulty, rules.level.label().to_lowercase()));
    }
    if w.category == Category::Anaerobic && !rules.allow_anaerobic {
        return Err("Anaerobic/sprint sessions are reserved for experienced riders.".into());
    }
    if w.category == Category::Assessment && !rules.allow_assessment {
        return Err("Maximal assessments are not scheduled at this level; the guided effort calibration is available instead.".into());
    }
    let int = w.intensity();
    if int >= Intensity::Hard && week < rules.intro_weeks_without_hard {
        return Err("The first weeks of a plan build consistency before hard sessions.".into());
    }
    if int >= Intensity::Hard && recovery_week && RECOVERY_WEEK_HARD == 0 {
        return Err("Recovery weeks contain no hard sessions.".into());
    }
    if w.category == Category::Assessment && week == 0 && rules.level != Level::Experienced {
        return Err("Assessments are offered after at least one week of consistent riding.".into());
    }
    Ok(())
}

// ------------------------------------------------------------------ readiness

#[derive(Debug, Clone, PartialEq)]
pub struct Readiness {
    /// 1 (very poor) … 5 (great)
    pub sleep: u8,
    /// 1 (fresh) … 5 (exhausted)
    pub fatigue: u8,
    /// 1 (none) … 5 (very sore)
    pub soreness: u8,
    /// 1 (calm) … 5 (very stressed)
    pub stress: u8,
    pub illness: bool,
    pub pain_or_injury: bool,
    /// Any of the warning symptoms listed in [`WARNING_SYMPTOMS`].
    pub warning_symptoms: bool,
}
json_struct!(Readiness {
    sleep: "sleep" = 3,
    fatigue: "fatigue" = 2,
    soreness: "soreness" = 1,
    stress: "stress" = 2,
    illness: "illness" = false,
    pain_or_injury: "pain_or_injury" = false,
    warning_symptoms: "warning_symptoms" = false,
});

impl Default for Readiness {
    fn default() -> Self {
        Readiness { sleep: 3, fatigue: 2, soreness: 1, stress: 2, illness: false, pain_or_injury: false, warning_symptoms: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessAdvice {
    Proceed,
    Shorten,
    SwapEasier,
    Rest,
    StopSeekCare,
}
json_enum!(ReadinessAdvice { Proceed = "proceed", Shorten = "shorten", SwapEasier = "swap_easier", Rest = "rest", StopSeekCare = "stop_seek_care" });

pub fn evaluate_readiness(r: &Readiness, planned: Intensity) -> (ReadinessAdvice, String) {
    if r.warning_symptoms {
        return (ReadinessAdvice::StopSeekCare, SAFETY_COPY.to_string());
    }
    if r.illness {
        return (ReadinessAdvice::Rest, "You reported feeling unwell. Rest today; training while ill can prolong recovery. Resume with an easy ride when you feel normal again.".into());
    }
    let flags = [r.fatigue >= 4, r.sleep <= 2, r.stress >= 4, r.soreness >= 4].iter().filter(|x| **x).count();
    if r.pain_or_injury {
        return (
            ReadinessAdvice::SwapEasier,
            "You reported pain or an injury. Only ride if it is pain-free; keep it easy and stop if pain appears. Consider asking a health professional if it persists.".into(),
        );
    }
    if flags >= 3 {
        return (ReadinessAdvice::Rest, "Several recovery signals are low today. A rest day is likely to serve you better than training.".into());
    }
    if flags >= 1 && planned >= Intensity::Hard {
        return (ReadinessAdvice::SwapEasier, "Recovery signals are low for a hard session. Swapping to an easier ride keeps the habit without digging a hole.".into());
    }
    if flags >= 2 {
        return (ReadinessAdvice::Shorten, "Recovery signals are a little low. Consider shortening today's ride.".into());
    }
    (ReadinessAdvice::Proceed, "Good to go. Listen to your body and stop if anything feels wrong.".into())
}

/// Warning symptoms that stop all performance-oriented recommendations.
/// Based on the warning signs used in exercise pre-participation screening
/// (see docs/coaching-policy.md). Detection is deliberately broad.
pub const WARNING_SYMPTOMS: &[&str] = &[
    "chest pain",
    "chest pressure",
    "chest tightness",
    "tight chest",
    "pain in my chest",
    "faint",
    "fainted",
    "passed out",
    "blacked out",
    "black out",
    "syncope",
    "palpitation",
    "irregular heartbeat",
    "heart racing",
    "heart is racing",
    "skipped beat",
    "short of breath",
    "shortness of breath",
    "can't breathe",
    "cannot breathe",
    "trouble breathing",
    "dizzy",
    "dizziness",
    "lightheaded",
    "light-headed",
    "numbness",
    "arm pain",
    "jaw pain",
];

pub const SAFETY_COPY: &str = "Stop exercising now. Chest pain or pressure, fainting, unusual shortness of breath, a racing or irregular heartbeat, or dizziness can be signs of a serious problem. If symptoms are severe or don't settle quickly, call your local emergency number. Please talk to a doctor before training again. Ridgeline will not suggest any training until you confirm you've been cleared to ride.";

/// True when free text mentions a warning symptom. Used on coach chat input
/// *before* any AI request, so the safety response never depends on a model.
pub fn mentions_warning_symptom(text: &str) -> bool {
    let t = text.to_lowercase();
    WARNING_SYMPTOMS.iter().any(|s| t.contains(s))
}

pub fn policy_json() -> Value {
    Value::obj([
        ("version", POLICY_VERSION.into()),
        ("review_status", "Draft: limits derived from cited sources by the implementer; not yet reviewed by a qualified coach or clinician.".into()),
        ("levels", Value::Arr([Level::Beginner, Level::Developing, Level::Returning, Level::Experienced].iter().map(|l| rules_for(*l).to_json()).collect())),
        ("min_days_between_hard", MIN_DAYS_BETWEEN_HARD.into()),
        ("recovery_week_every", RECOVERY_WEEK_EVERY.into()),
        ("recovery_week_hard_sessions", RECOVERY_WEEK_HARD.into()),
        ("default_plan_weeks", DEFAULT_PLAN_WEEKS.into()),
        ("max_plan_weeks", MAX_PLAN_WEEKS.into()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_workouts;

    #[test]
    fn levels() {
        let mut p = RiderProfile::demo();
        assert_eq!(level_for(&p), Level::Developing);
        p.experience = Experience::New;
        assert_eq!(level_for(&p), Level::Beginner);
        p.experience = Experience::Experienced;
        p.recent_weekly_min = 300;
        p.consistent_weeks = 10;
        assert_eq!(level_for(&p), Level::Experienced);
        p.weeks_off = 12;
        assert_eq!(level_for(&p), Level::Returning);
    }

    #[test]
    fn eligibility_gates_maximal_work() {
        let lib = builtin_workouts();
        let get = |id: &str| lib.iter().find(|w| w.id == id).unwrap().clone();
        let beg = rules_for(Level::Beginner);
        let exp = rules_for(Level::Experienced);
        assert!(eligibility(&get("anaerobic-1min"), &beg, 3, false).is_err());
        assert!(eligibility(&get("anaerobic-1min"), &exp, 1, false).is_ok());
        assert!(eligibility(&get("assessment-ramp"), &beg, 2, false).is_err());
        assert!(eligibility(&get("threshold-4x5"), &beg, 0, false).is_err(), "no hard in intro weeks");
        assert!(eligibility(&get("threshold-4x5"), &beg, 2, false).is_ok());
        assert!(eligibility(&get("threshold-4x5"), &exp, 3, true).is_err(), "recovery week");
        assert!(eligibility(&get("endurance-60"), &beg, 0, false).is_ok());
        assert!(eligibility(&get("assessment-guided-effort"), &beg, 0, false).is_ok());
    }

    #[test]
    fn readiness_and_symptoms() {
        let r = Readiness { warning_symptoms: true, ..Default::default() };
        assert_eq!(evaluate_readiness(&r, Intensity::Easy).0, ReadinessAdvice::StopSeekCare);
        let r = Readiness { sleep: 2, fatigue: 2, soreness: 1, stress: 2, ..Default::default() };
        assert_eq!(evaluate_readiness(&r, Intensity::Hard).0, ReadinessAdvice::SwapEasier);
        assert_eq!(evaluate_readiness(&r, Intensity::Easy).0, ReadinessAdvice::Proceed);
        let r = Readiness { sleep: 1, fatigue: 5, soreness: 4, stress: 2, ..Default::default() };
        assert_eq!(evaluate_readiness(&r, Intensity::Easy).0, ReadinessAdvice::Rest);
        assert!(mentions_warning_symptom("I had some Chest Pain on the last climb"));
        assert!(mentions_warning_symptom("felt dizzy"));
        assert!(!mentions_warning_symptom("legs are tired, can I do threshold?"));
    }
}
