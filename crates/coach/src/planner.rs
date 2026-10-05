//! Deterministic offline planner.
//!
//! Builds a plan from the reviewed template library and the coaching
//! policy only. It is labelled an "offline plan" in the UI — it is not an AI
//! conversation. The AI provider may later explain and customise this draft
//! within the same constraints.

use rl_domain::ids::new_uuid;
use rl_domain::plan::{is_recovery_week, volume_cap, PlannedSession, SessionStatus, TrainingPlan};
use rl_domain::policy::{self, eligibility, Level, LevelRules};
use rl_domain::rider::{Goal, PowerSource, RiderProfile};
use rl_domain::time::{Date, WEEKDAY_NAMES};
use rl_domain::workout::{Category, Intensity, Workout};

pub struct PlanContext<'a> {
    pub profile: &'a RiderProfile,
    pub ftp_w: Option<f64>,
    pub library: &'a [Workout],
    pub today: Date,
    pub start: Date,
    pub weeks: u32,
    /// Minutes actually ridden in the last 7 days (from history), if known.
    pub recent_actual_min: Option<u32>,
}

impl<'a> PlanContext<'a> {
    pub fn lookup(&self, id: &str) -> Option<&'a Workout> {
        self.library.iter().find(|w| w.id == id)
    }
    pub fn level(&self) -> Level {
        policy::level_for(self.profile)
    }
}

fn goal_categories(goal: Goal) -> (Vec<Category>, Vec<Category>) {
    // (moderate preferences, hard preferences) in priority order.
    use Category::*;
    match goal {
        Goal::GeneralFitness => (vec![Tempo, SweetSpot, Cadence], vec![Threshold, Vo2]),
        Goal::Endurance => (vec![Tempo, SweetSpot], vec![Threshold, OverUnder]),
        Goal::Climbing => (vec![Climbing, Cadence, SweetSpot], vec![Climbing, OverUnder, Threshold, Vo2]),
        Goal::Event => (vec![SweetSpot, Tempo], vec![Threshold, OverUnder, Vo2]),
        Goal::FtpImprovement => (vec![SweetSpot, Tempo], vec![Threshold, OverUnder, Vo2]),
        Goal::Returning => (vec![Tempo, Cadence], vec![Threshold]),
        Goal::Maintain => (vec![Tempo, SweetSpot], vec![Threshold, Vo2]),
    }
}

fn goal_label(goal: Goal) -> &'static str {
    match goal {
        Goal::GeneralFitness => "general fitness",
        Goal::Endurance => "endurance",
        Goal::Climbing => "climbing",
        Goal::Event => "your event",
        Goal::FtpImprovement => "raising your FTP",
        Goal::Returning => "returning to riding",
        Goal::Maintain => "maintaining fitness",
    }
}

/// Longest run of consecutive weekdays in a weekly-repeating pattern.
pub fn max_run(days: &[u32]) -> u32 {
    if days.len() >= 7 {
        return u32::MAX;
    }
    let mut best = 0;
    for start in 0..7u32 {
        let mut run = 0;
        while run < 7 && days.contains(&((start + run) % 7)) {
            run += 1;
        }
        best = best.max(run);
    }
    best
}

/// Choose training days: respect the rest-day minimum and the maximum run of
/// consecutive days, keep the long-ride day, and spread sessions out.
pub fn choose_days(profile: &RiderProfile, rules: &LevelRules) -> Vec<u32> {
    let avail: Vec<u32> = (0..7u32).filter(|d| profile.availability_min[*d as usize] > 0).collect();
    let max_days = 7 - rules.min_rest_days_per_week.min(6);
    if avail.is_empty() {
        return avail;
    }
    // Greedy: start with the long-ride day (or the longest day), then add the
    // day that maximises the minimum circular gap to chosen days.
    let mut chosen: Vec<u32> = Vec::new();
    let first = profile.long_ride_day.map(|d| d as u32).filter(|d| avail.contains(d)).unwrap_or_else(|| *avail.iter().max_by_key(|d| profile.availability_min[**d as usize]).unwrap());
    chosen.push(first);
    loop {
        if chosen.len() as u32 >= max_days {
            break;
        }
        let best = avail
            .iter()
            .filter(|d| !chosen.contains(d))
            .filter(|d| {
                let mut t = chosen.clone();
                t.push(**d);
                max_run(&t) <= rules.max_consecutive_days
            })
            .max_by_key(|d| {
                let gap = chosen.iter().map(|c| { let g = (**d as i32 - *c as i32).rem_euclid(7); g.min(7 - g) }).min().unwrap_or(7);
                (gap, profile.availability_min[**d as usize])
            })
            .copied();
        match best {
            Some(d) => chosen.push(d),
            None => break,
        }
    }
    chosen.sort();
    chosen
}

fn eligible<'a>(ctx: &PlanContext<'a>, rules: &LevelRules, week: u32) -> Vec<&'a Workout> {
    let rec = is_recovery_week(week);
    ctx.library
        .iter()
        .filter(|w| eligibility(w, rules, week, rec).is_ok())
        .filter(|w| !ctx.profile.avoided_categories.contains(&w.category) || w.category == Category::Recovery)
        // Planner never auto-schedules maximal assessments or anaerobic work.
        .filter(|w| w.intensity() != Intensity::Maximal)
        .collect()
}

fn pick<'a>(cands: &[&'a Workout], cats: &[Category], intensity: Intensity, max_s: u32, week: u32, used: &[String], prefer: &[Category]) -> Option<&'a Workout> {
    let mut best: Option<(&Workout, i64)> = None;
    for w in cands {
        if w.intensity() != intensity || w.total_s() > max_s {
            continue;
        }
        let cat_rank = cats.iter().position(|c| *c == w.category).map(|i| 10 - i as i64).unwrap_or(if cats.is_empty() { 5 } else { -100 });
        if cat_rank < -50 && !cats.is_empty() {
            continue;
        }
        // Prefer longer variants as weeks progress, rotate families for variety.
        let dur_score = (w.total_s() as i64 / 60) * if week == 0 { 1 } else { 2 };
        let repeat_pen = used.iter().filter(|u| **u == w.id).count() as i64 * 40 + used.iter().filter(|u| u.starts_with(&w.family)).count() as i64 * 5;
        let pref = if prefer.contains(&w.category) { 15 } else { 0 };
        let diff_pen = (w.difficulty as i64) * if week == 0 { 6 } else { 2 };
        let score = cat_rank * 20 + dur_score / 3 + pref - repeat_pen - diff_pen;
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((w, score));
        }
    }
    best.map(|(w, _)| w)
}

fn easy_for<'a>(cands: &[&'a Workout], max_s: u32, recovery: bool, used: &[String]) -> Option<&'a Workout> {
    let mut opts: Vec<&&Workout> = cands
        .iter()
        .filter(|w| w.intensity() == Intensity::Easy && w.total_s() <= max_s)
        .filter(|w| if recovery { w.category == Category::Recovery } else { w.category != Category::Assessment })
        .collect();
    opts.sort_by_key(|w| {
        let assess = if w.family == "assessment-guided" { 1 } else { 0 };
        (assess, -(w.total_s() as i64), used.iter().filter(|u| **u == w.id).count())
    });
    opts.first().map(|w| **w)
}

fn why_text(w: &Workout, ctx: &PlanContext, level: Level, day: u32, week: u32, role: &str) -> String {
    let wk = if is_recovery_week(week) { "This is a lighter recovery week. ".to_string() } else { String::new() };
    let rpe = if ctx.ftp_w.is_none() || ctx.profile.power_source == PowerSource::None {
        " With no FTP set, ride it by perceived effort."
    } else {
        ""
    };
    format!(
        "{wk}{role} for {} on {}, sized to your {}-minute window. Chosen for {} at the '{}' level.{rpe}",
        goal_label(ctx.profile.goal),
        WEEKDAY_NAMES[day as usize],
        ctx.profile.availability_min[day as usize],
        w.category.label().to_lowercase(),
        level.label().to_lowercase()
    )
}

pub fn alternatives(ctx: &PlanContext, rules: &LevelRules, w: &Workout, week: u32, max_s: u32) -> Vec<String> {
    let mut alts: Vec<&Workout> = eligible(ctx, rules, week)
        .into_iter()
        .filter(|a| a.id != w.id && a.intensity() <= w.intensity() && a.total_s() <= max_s)
        .collect();
    alts.sort_by_key(|a| (a.intensity() != w.intensity(), a.category != w.category, (a.total_s() as i64 - w.total_s() as i64).abs()));
    alts.into_iter().take(2).map(|a| a.id.clone()).collect()
}

pub fn make_session(ctx: &PlanContext, rules: &LevelRules, w: &Workout, date: Date, week: u32, why: String) -> PlannedSession {
    let avail = ctx.profile.availability_min[date.weekday() as usize] * 60;
    PlannedSession {
        id: new_uuid(),
        date,
        workout_id: w.id.clone(),
        duration_s: w.total_s(),
        intensity: w.intensity(),
        target_basis: if ctx.ftp_w.is_some() && ctx.profile.power_source != PowerSource::None { "power".into() } else { "rpe".into() },
        purpose: w.purpose.clone(),
        why,
        difficulty: w.difficulty,
        alternatives: alternatives(ctx, rules, w, week, avail),
        status: SessionStatus::Planned,
        activity_id: None,
        keep_pct: 100,
    }
}

/// Build the deterministic plan. Always returns a plan that passes
/// `validate_plan` (asserted in tests over many synthetic profiles).
pub fn build_plan(ctx: &PlanContext) -> TrainingPlan {
    let level = ctx.level();
    let rules = policy::rules_for(level);
    let days = choose_days(ctx.profile, &rules);
    let (mod_cats, hard_cats) = goal_categories(ctx.profile.goal);
    let prefer = ctx.profile.preferred_categories.clone();
    let mut sessions: Vec<PlannedSession> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    let mut prev_minutes: Option<u32> = None;
    let mut last_hard: Option<Date> = sessions.iter().rev().find(|s: &&PlannedSession| s.intensity >= Intensity::Hard).map(|s| s.date);
    for week in 0..ctx.weeks {
        let week_start = ctx.start.add_days(7 * week as i64);
        let cands = eligible(ctx, &rules, week);
        let cap = volume_cap(&rules, ctx.profile, week, prev_minutes);
        let recovery = is_recovery_week(week);
        let mut hard_left = if recovery { policy::RECOVERY_WEEK_HARD } else if week < rules.intro_weeks_without_hard { 0 } else { rules.hard_per_week };
        let mut mod_left = rules.moderate_per_week;
        // Dates in this week, in order, that are training days and not in the past.
        let dates: Vec<Date> = (0..7).map(|i| week_start.add_days(i)).filter(|d| days.contains(&d.weekday()) && *d >= ctx.today).collect();
        let long_day = ctx.profile.long_ride_day.map(|d| d as u32);
        let mut week_sessions: Vec<PlannedSession> = Vec::new();
        let mut minutes = 0u32;
        let mut day_after_hard = false;
        for (i, d) in dates.iter().enumerate() {
            let wd = d.weekday();
            let avail_s = ctx.profile.availability_min[wd as usize] * 60;
            let remaining_budget = cap.saturating_sub(minutes) * 60;
            let max_s = avail_s.min(remaining_budget);
            if max_s < 30 * 60 {
                continue;
            }
            let is_long = long_day == Some(wd);
            let spacing_ok = last_hard.map(|h| h.days_until(d) >= policy::MIN_DAYS_BETWEEN_HARD).unwrap_or(true);
            // Hard sessions need an easier following day when possible: avoid the day before the long ride.
            let next_is_long = dates.get(i + 1).map(|n| long_day == Some(n.weekday()) && d.days_until(n) == 1).unwrap_or(false);
            let mut chosen: Option<(&Workout, &str)> = None;
            if hard_left > 0 && !is_long && spacing_ok && !next_is_long {
                if let Some(w) = pick(&cands, &hard_cats, Intensity::Hard, max_s, week, &used, &prefer) {
                    chosen = Some((w, "Key session"));
                    hard_left -= 1;
                }
            }
            if chosen.is_none() && mod_left > 0 && !is_long && !day_after_hard {
                if let Some(w) = pick(&cands, &mod_cats, Intensity::Moderate, max_s, week, &used, &prefer) {
                    chosen = Some((w, "Quality aerobic session"));
                    mod_left -= 1;
                }
            }
            if chosen.is_none() {
                let recov = day_after_hard && !is_long;
                if let Some(w) = easy_for(&cands, max_s, recov, &used).or_else(|| easy_for(&cands, max_s, false, &used)) {
                    chosen = Some((w, if is_long { "Long ride" } else if recov { "Recovery ride" } else { "Aerobic base ride" }));
                }
            }
            let Some((w, role)) = chosen else { continue };
            if w.intensity() >= Intensity::Hard {
                last_hard = Some(*d);
            }
            day_after_hard = w.intensity() >= Intensity::Hard;
            minutes += w.total_s() / 60;
            used.push(w.id.clone());
            let why = why_text(w, ctx, level, wd, week, role);
            week_sessions.push(make_session(ctx, &rules, w, *d, week, why));
        }
        prev_minutes = Some(minutes);
        sessions.extend(week_sessions);
    }
    let plan = TrainingPlan {
        schema: rl_domain::plan::PLAN_SCHEMA_VERSION,
        id: new_uuid(),
        version: 1,
        created_utc: rl_domain::time::now_utc_ms(),
        start: ctx.start,
        weeks: ctx.weeks,
        level,
        policy_version: policy::POLICY_VERSION.into(),
        source: "offline".into(),
        provider: "Ridgeline offline planner".into(),
        model: String::new(),
        rationale: format!(
            "Offline plan built from the template library and coaching policy {}. Level: {}. {} training days per week ({}), {} hard session(s) per build week after {} introductory week(s); every {}th week is lighter.",
            policy::POLICY_VERSION,
            level.label(),
            days.len(),
            days.iter().map(|d| WEEKDAY_NAMES[*d as usize]).collect::<Vec<_>>().join(", "),
            rules.hard_per_week,
            rules.intro_weeks_without_hard,
            policy::RECOVERY_WEEK_EVERY
        ),
        sessions,
    };
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_domain::library::builtin_workouts;
    use rl_domain::plan::validate_plan;
    use rl_domain::rider::{Experience, RiderProfile};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[(self.next() % v.len() as u64) as usize]
        }
    }

    #[test]
    fn demo_profile_plan_is_valid_and_sensible() {
        let lib = builtin_workouts();
        let p = RiderProfile::demo();
        let today = Date::parse("2026-10-05").unwrap();
        let ctx = PlanContext { profile: &p, ftp_w: Some(230.0), library: &lib, today, start: today, weeks: 4, recent_actual_min: None };
        let plan = build_plan(&ctx);
        let issues = validate_plan(&plan, &p, today, &|id| ctx.lookup(id));
        assert!(issues.is_empty(), "{issues:?}");
        assert!(plan.sessions.len() >= 10, "{}", plan.sessions.len());
        let ws = plan.week_summaries();
        assert_eq!(ws[3].hard, 0, "recovery week");
        assert!(ws[3].minutes < ws[2].minutes);
        assert!(plan.sessions.iter().all(|s| s.alternatives.len() <= 2 && !s.why.is_empty()));
    }

    #[test]
    fn property_random_profiles_always_validate() {
        let lib = builtin_workouts();
        let mut rng = Rng(0x9E3779B97F4A7C15);
        let today = Date::parse("2026-10-05").unwrap();
        for _ in 0..400 {
            let mut p = RiderProfile::demo();
            p.goal = rng.pick(&[Goal::GeneralFitness, Goal::Endurance, Goal::Climbing, Goal::Event, Goal::FtpImprovement, Goal::Returning, Goal::Maintain]);
            p.experience = rng.pick(&[Experience::New, Experience::Some, Experience::Experienced]);
            p.recent_weekly_min = rng.pick(&[0, 30, 90, 150, 300, 600]);
            p.consistent_weeks = rng.pick(&[0, 2, 6, 12]);
            p.weeks_off = rng.pick(&[0, 0, 4, 20]);
            p.availability_min = (0..7).map(|_| rng.pick(&[0, 0, 30, 45, 60, 90, 120, 180])).collect();
            if p.availability_min.iter().all(|m| *m == 0) {
                p.availability_min[5] = 60;
            }
            let avail: Vec<u8> = (0..7u8).filter(|d| p.availability_min[*d as usize] > 0).collect();
            p.long_ride_day = if rng.next() % 2 == 0 { Some(rng.pick(&avail)) } else { None };
            if rng.next() % 4 == 0 {
                p.avoided_categories = vec![rng.pick(&[Category::Vo2, Category::Threshold, Category::Tempo])];
            }
            let ftp = if rng.next() % 3 == 0 { None } else { Some(200.0) };
            let start = today.add_days((rng.next() % 3) as i64);
            let weeks = rng.pick(&[1, 2, 4, 6, 8]);
            let ctx = PlanContext { profile: &p, ftp_w: ftp, library: &lib, today, start, weeks, recent_actual_min: None };
            let plan = build_plan(&ctx);
            let issues = validate_plan(&plan, &p, today, &|id| ctx.lookup(id));
            assert!(issues.is_empty(), "profile {:?}\nissues {:?}", p.availability_min, issues);
            for s in &plan.sessions {
                assert!(s.duration_s <= p.availability_min[s.date.weekday() as usize] * 60);
            }
        }
    }

    #[test]
    fn tight_schedule_and_no_ftp() {
        let lib = builtin_workouts();
        let mut p = RiderProfile::demo();
        p.availability_min = vec![0, 0, 30, 0, 0, 0, 0];
        p.long_ride_day = None;
        p.experience = Experience::New;
        let today = Date::parse("2026-10-05").unwrap();
        let ctx = PlanContext { profile: &p, ftp_w: None, library: &lib, today, start: today, weeks: 4, recent_actual_min: None };
        let plan = build_plan(&ctx);
        assert!(validate_plan(&plan, &p, today, &|id| ctx.lookup(id)).is_empty());
        assert!(plan.sessions.iter().all(|s| s.date.weekday() == 2 && s.duration_s <= 1800 && s.target_basis == "rpe"));
    }
}
