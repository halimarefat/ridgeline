//! Original built-in workout library (30 templates, 11 categories) plus
//! technical test fixtures that are *not* part of the library count.
//!
//! All templates were written for Ridgeline. Intensities use conventional
//! %FTP training zones (see docs/coaching-policy.md for sources). They are
//! reviewed against coaching policy version [`crate::policy::POLICY_VERSION`];
//! a qualified coach should review them again before a commercial release.

use crate::policy::POLICY_VERSION;
use crate::workout::*;

fn st(kind: StepKind, dur_s: u32, target: Target) -> Step {
    Step { kind, dur_s, target, cadence: None, text: None }
}
fn warm(dur_s: u32, a: f64, b: f64) -> Step {
    st(StepKind::Warmup, dur_s, Target::ramp(a, b)).say("Ease in. Keep it conversational.")
}
fn cool(dur_s: u32, a: f64, b: f64) -> Step {
    st(StepKind::Cooldown, dur_s, Target::ramp(a, b)).say("Spin it out and let your heart rate settle.")
}
fn steady(dur_s: u32, pct: f64) -> Step {
    st(StepKind::Steady, dur_s, Target::ftp(pct))
}
fn work(dur_s: u32, pct: f64) -> Step {
    st(StepKind::Work, dur_s, Target::ftp(pct))
}
fn rest(dur_s: u32, pct: f64) -> Step {
    st(StepKind::Rest, dur_s, Target::ftp(pct))
}

trait StepExt {
    fn cad(self, lo: u16, hi: u16) -> Step;
    fn say(self, t: &str) -> Step;
}
impl StepExt for Step {
    fn cad(mut self, lo: u16, hi: u16) -> Step {
        self.cadence = Some(CadenceCue { lo, hi });
        self
    }
    fn say(mut self, t: &str) -> Step {
        self.text = Some(t.to_string());
        self
    }
}

fn one(s: Step) -> Block {
    Block { count: 1, steps: vec![s] }
}
fn rep(n: u32, steps: Vec<Step>) -> Block {
    Block { count: n, steps }
}

#[allow(clippy::too_many_arguments)]
fn w(id: &str, family: &str, name: &str, cat: Category, diff: u8, desc: &str, purpose: &str, blocks: Vec<Block>) -> Workout {
    Workout {
        id: id.into(),
        schema: WORKOUT_SCHEMA_VERSION,
        version: 1,
        name: name.into(),
        category: cat,
        difficulty: diff,
        description: desc.into(),
        purpose: purpose.into(),
        blocks,
        builtin: true,
        family: family.into(),
        policy_version: POLICY_VERSION.into(),
        created_utc: 0,
        updated_utc: 0,
    }
}

pub fn builtin_workouts() -> Vec<Workout> {
    use Category::*;
    vec![
        // ---------------------------------------------------------- recovery
        w("recovery-spin-30", "recovery-spin", "Easy Spin 30", Recovery, 1,
          "Thirty minutes of very light pedalling. Resistance should feel almost absent.",
          "Promotes recovery between harder days without adding meaningful fatigue.",
          vec![one(warm(300, 40.0, 52.0)), one(steady(1200, 52.0).say("Light pressure on the pedals, relaxed shoulders.")), one(cool(300, 50.0, 40.0))]),
        w("recovery-spin-45", "recovery-spin", "Easy Spin 45 with Leg Speed", Recovery, 1,
          "A longer recovery spin with three short, light, quick-pedalling segments.",
          "Keeps the legs moving on a rest-oriented day and practises smooth pedalling.",
          vec![one(warm(300, 40.0, 55.0)),
               rep(3, vec![steady(540, 55.0), steady(60, 55.0).cad(100, 110).say("Quick feet, light pressure. Stay smooth on the saddle.")]),
               one(steady(300, 52.0)), one(cool(300, 50.0, 40.0))]),
        // ---------------------------------------------------------- endurance
        w("endurance-45", "endurance", "Endurance 45", Endurance, 2,
          "A steady aerobic ride at a comfortable, sustainable effort.",
          "Builds aerobic base efficiently when time is short.",
          vec![one(warm(600, 45.0, 65.0)), one(steady(1800, 68.0).say("Find a rhythm you could hold for hours.")), one(cool(300, 60.0, 45.0))]),
        w("endurance-60", "endurance", "Endurance 60", Endurance, 2,
          "One hour of zone-2 style riding split into two steady blocks.",
          "Develops aerobic capacity and fat utilisation with low stress.",
          vec![one(warm(600, 45.0, 65.0)), one(steady(1200, 70.0)), one(rest(300, 60.0)), one(steady(1200, 70.0)), one(cool(300, 60.0, 45.0))]),
        w("endurance-90", "endurance", "Endurance 90", Endurance, 2,
          "Ninety minutes of steady aerobic riding with brief easier sections.",
          "Extends aerobic durability; suits a mid-week long ride.",
          vec![one(warm(600, 45.0, 65.0)), rep(3, vec![steady(1200, 72.0), rest(180, 60.0)]), one(cool(660, 62.0, 45.0))]),
        w("endurance-120", "endurance", "Endurance 120", Endurance, 3,
          "A two-hour aerobic ride. Fuel and hydrate as you would outdoors.",
          "Long-ride endurance for event preparation and base building.",
          vec![one(warm(600, 45.0, 65.0)), rep(4, vec![steady(1440, 70.0), rest(120, 60.0)]), one(cool(360, 62.0, 45.0))]),
        // ---------------------------------------------------------- tempo
        w("tempo-3x10", "tempo", "Tempo 3×10", Tempo, 2,
          "Three ten-minute blocks at a purposeful but controlled pace.",
          "Raises sustainable power and muscular endurance with moderate stress.",
          vec![one(warm(600, 45.0, 70.0)), rep(3, vec![work(600, 82.0).say("Steady pressure. Breathing deep but controlled."), rest(240, 58.0)]), one(cool(480, 60.0, 45.0))]),
        w("tempo-2x20", "tempo", "Tempo 2×20", Tempo, 3,
          "Two twenty-minute tempo efforts with a short recovery.",
          "Builds the ability to hold a strong steady pace for longer.",
          vec![one(warm(600, 45.0, 72.0)), rep(2, vec![work(1200, 83.0), rest(300, 58.0)]), one(cool(600, 60.0, 45.0))]),
        w("tempo-40", "tempo", "Tempo 40 Continuous", Tempo, 3,
          "A single forty-minute tempo block. Mentally demanding, physically moderate.",
          "Sustained muscular endurance for long climbs and group rides.",
          vec![one(warm(900, 45.0, 75.0)), one(work(2400, 80.0).say("Settle in. Smooth, even pedalling.")), one(cool(900, 62.0, 45.0))]),
        // ---------------------------------------------------------- sweet spot
        w("sweet-spot-3x8", "sweet-spot", "Sweet Spot 3×8", SweetSpot, 3,
          "Three eight-minute efforts just below threshold.",
          "Time-efficient threshold support with manageable fatigue.",
          vec![one(warm(480, 45.0, 75.0)), rep(3, vec![work(480, 89.0).say("Strong but sustainable. You should finish each one with a little left."), rest(180, 55.0)]), one(cool(240, 60.0, 45.0))]),
        w("sweet-spot-3x12", "sweet-spot", "Sweet Spot 3×12", SweetSpot, 3,
          "Three twelve-minute sweet-spot efforts.",
          "Builds sustainable power close to threshold.",
          vec![one(warm(480, 45.0, 75.0)), rep(3, vec![work(720, 90.0), rest(180, 55.0)]), one(cool(420, 60.0, 45.0))]),
        w("sweet-spot-2x20", "sweet-spot", "Sweet Spot 2×20", SweetSpot, 4,
          "Two twenty-minute sweet-spot efforts. A staple, but a demanding one.",
          "Extends time-at-intensity for threshold development.",
          vec![one(warm(720, 45.0, 75.0)), rep(2, vec![work(1200, 90.0), rest(360, 55.0)]), one(cool(660, 60.0, 45.0))]),
        // ---------------------------------------------------------- threshold
        w("threshold-4x5", "threshold", "Threshold 4×5", Threshold, 3,
          "Four five-minute efforts at threshold with generous recovery.",
          "An accessible introduction to threshold-intensity work.",
          vec![one(warm(720, 45.0, 80.0)), rep(4, vec![work(300, 100.0).say("Hard and steady. Don't start too fast."), rest(300, 55.0)]), one(cool(480, 60.0, 45.0))]),
        w("threshold-3x10", "threshold", "Threshold 3×10", Threshold, 4,
          "Three ten-minute threshold efforts.",
          "Raises the power you can sustain for longer climbs and time trials.",
          vec![one(warm(600, 45.0, 80.0)), rep(3, vec![work(600, 97.0), rest(240, 55.0)]), one(cool(480, 60.0, 45.0))]),
        w("threshold-2x15", "threshold", "Threshold 2×15", Threshold, 4,
          "Two fifteen-minute threshold efforts.",
          "Extends time at threshold for experienced riders.",
          vec![one(warm(720, 45.0, 80.0)), rep(2, vec![work(900, 98.0), rest(480, 55.0)]), one(cool(780, 60.0, 45.0))]),
        // ---------------------------------------------------------- over-unders
        w("over-under-3x9", "over-under", "Over-Unders 3×9", OverUnder, 4,
          "Three sets alternating just-below and just-above threshold every few minutes.",
          "Trains clearing fatigue while staying near threshold, as on rolling climbs.",
          vec![one(warm(600, 45.0, 80.0)),
               rep(3, vec![work(120, 95.0).say("Under: steady and controlled."), work(60, 107.0).say("Over: lift the pace, stay seated."),
                           work(120, 95.0), work(60, 107.0), work(120, 95.0), work(60, 107.0), rest(300, 55.0)]),
               one(cool(480, 60.0, 45.0))]),
        w("over-under-4x12", "over-under", "Over-Unders 4×12", OverUnder, 5,
          "Four longer over-under sets. Only for riders with recent threshold work.",
          "Advanced threshold tolerance and pace changes.",
          vec![one(warm(720, 45.0, 80.0)),
               rep(4, vec![work(180, 94.0), work(60, 106.0), work(180, 94.0), work(60, 106.0), work(180, 94.0), work(60, 106.0), rest(300, 55.0)]),
               one(cool(600, 60.0, 45.0))]),
        // ---------------------------------------------------------- VO2-style
        w("vo2-5x3", "vo2", "VO2 5×3", Vo2, 4,
          "Five three-minute hard efforts with equal recovery.",
          "Raises aerobic ceiling; supports climbing and short hard efforts.",
          vec![one(warm(720, 45.0, 80.0)), rep(5, vec![work(180, 115.0).say("Hard, steady effort. Breathing heavily by the end."), rest(180, 50.0)]), one(cool(600, 60.0, 45.0))]),
        w("vo2-30-30", "vo2", "VO2 30/30s", Vo2, 4,
          "Two sets of ten 30-second efforts with 30-second recoveries.",
          "Accumulates time at high aerobic intensity in short, manageable pieces.",
          vec![one(warm(720, 45.0, 80.0)), rep(10, vec![work(30, 118.0), rest(30, 50.0)]), one(rest(300, 55.0)), rep(10, vec![work(30, 118.0), rest(30, 50.0)]), one(cool(480, 60.0, 45.0))]),
        w("vo2-4x4", "vo2", "VO2 4×4", Vo2, 5,
          "Four four-minute efforts with three-minute recoveries.",
          "A classic high-intensity aerobic interval format for experienced riders.",
          vec![one(warm(780, 45.0, 80.0)), rep(4, vec![work(240, 112.0), rest(180, 50.0)]), one(cool(720, 60.0, 45.0))]),
        // ---------------------------------------------------------- cadence
        w("cadence-spin-ups", "cadence", "Cadence Spin-Ups", Cadence, 2,
          "Aerobic riding with six one-minute high-cadence segments at light load.",
          "Improves pedalling smoothness and neuromuscular coordination.",
          vec![one(warm(600, 45.0, 62.0)),
               rep(6, vec![steady(60, 60.0).cad(100, 115).say("Spin up smoothly; no bouncing on the saddle."), steady(180, 62.0).cad(85, 95)]),
               one(steady(360, 65.0)), one(cool(300, 55.0, 45.0))]),
        w("cadence-low-torque", "cadence", "Low-Cadence Strength", Cadence, 3,
          "Moderate efforts at low cadence. Stop if you feel knee discomfort.",
          "Builds force at low cadence for steep climbs.",
          vec![one(warm(600, 45.0, 70.0)),
               rep(4, vec![work(300, 76.0).cad(60, 70).say("Big gear feel, seated, smooth. Ease off if your knees complain."), rest(180, 55.0).cad(85, 95)]),
               one(cool(480, 60.0, 45.0))]),
        // ---------------------------------------------------------- climbing
        w("climbing-seated-tempo", "climbing", "Seated Climbing Tempo", Climbing, 3,
          "Three ten-minute climbing-style efforts at slightly lower cadence.",
          "Prepares for long seated climbs.",
          vec![one(warm(600, 45.0, 72.0)), rep(3, vec![work(600, 85.0).cad(70, 78).say("Imagine a long steady climb. Stay seated."), rest(240, 55.0)]), one(cool(480, 60.0, 45.0))]),
        w("climbing-pyramid", "climbing", "Climbing Pyramid", Climbing, 4,
          "A pyramid of increasing then decreasing efforts, like a climb that steepens and eases.",
          "Practises pacing changing gradients.",
          vec![one(warm(600, 45.0, 75.0)),
               rep(2, vec![work(240, 85.0), work(240, 90.0), work(240, 95.0), work(240, 90.0), work(240, 85.0), rest(300, 55.0)]),
               one(cool(300, 60.0, 45.0))]),
        w("climbing-surges", "climbing", "Long Climb with Surges", Climbing, 4,
          "Two twenty-minute steady climbs with short surges every five minutes.",
          "Simulates attacks and steep ramps on a long climb.",
          vec![one(warm(720, 45.0, 75.0)),
               rep(2, vec![work(270, 87.0), work(30, 110.0).say("Surge! Then settle straight back."), work(270, 87.0), work(30, 110.0),
                           work(270, 87.0), work(30, 110.0), work(270, 87.0), work(30, 110.0), rest(360, 55.0)]),
               one(cool(480, 60.0, 45.0))]),
        // ---------------------------------------------------------- assessment (optional)
        w("assessment-ramp", "assessment", "Ramp Assessment (optional)", Assessment, 5,
          "A maximal ramp test: one-minute steps get harder until you choose to stop. Optional; never required. Only ride it when healthy and rested.",
          "Gives a provisional FTP estimate (75% of your best one-minute power). Stop at any time.",
          vec![one(warm(300, 40.0, 50.0)),
               Block { count: 1, steps: (0..10).map(|i| work(60, 50.0 + 6.0 * i as f64).say("Stop whenever you can no longer hold the target.")).collect() },
               Block { count: 1, steps: (10..20).map(|i| work(60, 50.0 + 6.0 * i as f64).say("Stop whenever you can no longer hold the target.")).collect() },
               one(cool(300, 45.0, 35.0))]),
        w("assessment-20min", "assessment", "20-Minute Assessment (optional)", Assessment, 5,
          "A well-paced maximal twenty-minute effort after a thorough warm-up. Optional; never required.",
          "Gives a provisional FTP estimate (95% of the 20-minute average).",
          vec![one(warm(900, 45.0, 75.0)), rep(3, vec![work(60, 105.0), rest(60, 55.0)]), one(rest(300, 55.0)),
               one(st(StepKind::Work, 1200, Target::rpe(9.0)).say("Pace evenly. Start conservatively and build if you can.")),
               one(cool(600, 55.0, 40.0))]),
        w("assessment-guided-effort", "assessment-guided", "Guided Effort Calibration", Endurance, 1,
          "Three six-minute stages at easy, moderate and steady perceived effort. Not a maximal test.",
          "Shows the power and heart rate that match your own sense of effort. No FTP is calculated from it.",
          vec![one(st(StepKind::Warmup, 300, Target::rpe(2.0)).say("Very easy. You could chat freely.")),
               one(st(StepKind::Steady, 360, Target::rpe(3.0)).say("Comfortable: full sentences.")), one(st(StepKind::Rest, 120, Target::rpe(2.0))),
               one(st(StepKind::Steady, 360, Target::rpe(4.0)).say("Steady: sentences get shorter.")), one(st(StepKind::Rest, 120, Target::rpe(2.0))),
               one(st(StepKind::Steady, 360, Target::rpe(5.0)).say("Moderate: a few words at a time. Not hard.")),
               one(st(StepKind::Cooldown, 300, Target::rpe(2.0)))]),
        // ---------------------------------------------------------- anaerobic (gated)
        w("anaerobic-1min", "anaerobic", "Anaerobic 6×1 min", Anaerobic, 5,
          "Six one-minute very hard efforts with long recoveries. Policy-gated to experienced riders.",
          "Develops anaerobic capacity for short steep climbs and attacks.",
          vec![one(warm(900, 45.0, 80.0)), rep(6, vec![work(60, 135.0), rest(240, 50.0)]), one(cool(600, 60.0, 45.0))]),
        w("anaerobic-sprints", "anaerobic", "Sprint Starts", Anaerobic, 5,
          "Eight 15-second seated sprints. Use simulation or manual resistance rather than ERG. Policy-gated.",
          "Neuromuscular power and sprint technique.",
          vec![one(warm(900, 45.0, 75.0)),
               rep(8, vec![st(StepKind::Work, 15, Target::rpe(10.0)).say("Seated sprint. Smooth, fast, controlled."), rest(225, 50.0)]),
               one(steady(600, 62.0)), one(cool(300, 55.0, 45.0))]),
    ]
}

/// Short technical workouts for hardware checks (acceptance test A04).
/// They are fixtures, not training content.
pub fn test_fixtures() -> Vec<Workout> {
    use Category::TestFixture;
    vec![
        w("test-erg-short", "test", "Test: short ERG check", TestFixture, 1,
          "Three minutes: 100 W, 150 W, 100 W. Verifies the trainer accepts ERG targets and transitions.",
          "Hardware check, not training.",
          vec![one(st(StepKind::Warmup, 60, Target::watts(100.0))), one(st(StepKind::Work, 60, Target::watts(150.0))), one(st(StepKind::Cooldown, 60, Target::watts(100.0)))]),
        w("test-erg-pause", "test", "Test: ERG pause/resume", TestFixture, 1,
          "Five minutes of 120–160 W steps for checking pause, resume and finish.",
          "Hardware check, not training.",
          vec![rep(5, vec![st(StepKind::Work, 30, Target::watts(160.0)), st(StepKind::Rest, 30, Target::watts(120.0))])]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn library_meets_release_requirements() {
        let lib = builtin_workouts();
        assert!(lib.len() >= 20, "need ≥20 workouts, have {}", lib.len());
        let cats: HashSet<_> = lib.iter().map(|w| w.category).collect();
        assert!(cats.len() >= 8, "need ≥8 categories, have {}", cats.len());
        let ids: HashSet<_> = lib.iter().map(|w| w.id.clone()).collect();
        assert_eq!(ids.len(), lib.len(), "duplicate ids");
        for w in &lib {
            assert!(crate::ids::is_slug(&w.id), "{}", w.id);
            let issues = w.validate();
            assert!(issues.is_empty(), "{}: {:?}", w.id, issues);
            assert!(!w.description.is_empty() && !w.purpose.is_empty());
        }
        // Anaerobic content is classified maximal (and therefore policy-gated).
        for w in lib.iter().filter(|w| matches!(w.category, Category::Anaerobic | Category::Assessment)) {
            assert_eq!(w.intensity(), Intensity::Maximal, "{}", w.id);
        }
        // Distinct content: no two workouts share the same timeline.
        let mut seen = HashSet::new();
        for w in &lib {
            let sig: Vec<_> = w.timeline().iter().map(|s| (s.dur_s, (s.target.value * 10.0) as i64, s.target.end.map(|e| (e * 10.0) as i64))).collect();
            assert!(seen.insert(sig), "duplicate timeline {}", w.id);
        }
    }

    #[test]
    fn intensity_classes_are_sensible() {
        let lib = builtin_workouts();
        let get = |id: &str| lib.iter().find(|w| w.id == id).unwrap().intensity();
        assert_eq!(get("recovery-spin-30"), Intensity::Easy);
        assert_eq!(get("endurance-90"), Intensity::Easy);
        assert_eq!(get("tempo-2x20"), Intensity::Moderate);
        assert_eq!(get("threshold-3x10"), Intensity::Hard);
        assert_eq!(get("vo2-5x3"), Intensity::Hard);
        assert_eq!(get("assessment-guided-effort"), Intensity::Easy);
    }

    #[test]
    fn fixtures_valid() {
        for w in test_fixtures() {
            assert!(w.validate().is_empty(), "{}", w.id);
        }
    }
}

#[cfg(test)]
mod dump {
    #[test]
    #[ignore]
    fn print_durations() {
        for w in super::builtin_workouts() {
            println!("{:28} {:>4} min {:?} d{}", w.id, w.total_s() / 60, w.intensity(), w.difficulty);
        }
    }
}
