//! Versioned structured workout definitions, validation and timeline
//! flattening.
//!
//! A workout is a list of blocks; each block repeats its steps `count` times.
//! Targets are stored as % of FTP, absolute watts, or perceived effort (RPE).
//! Derived watts are computed from the FTP snapshot saved with the session so
//! historical rides never change when FTP changes.

use rl_json::{json_enum, json_struct, FromJson, JResult, ToJson, Value};

pub const WORKOUT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Recovery,
    Endurance,
    Tempo,
    SweetSpot,
    Threshold,
    OverUnder,
    Vo2,
    Anaerobic,
    Cadence,
    Climbing,
    Assessment,
    TestFixture,
}
json_enum!(Category {
    Recovery = "recovery",
    Endurance = "endurance",
    Tempo = "tempo",
    SweetSpot = "sweet_spot",
    Threshold = "threshold",
    OverUnder = "over_under",
    Vo2 = "vo2",
    Anaerobic = "anaerobic",
    Cadence = "cadence",
    Climbing = "climbing",
    Assessment = "assessment",
    TestFixture = "test_fixture",
});

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::Recovery => "Recovery",
            Category::Endurance => "Endurance",
            Category::Tempo => "Tempo",
            Category::SweetSpot => "Sweet spot",
            Category::Threshold => "Threshold",
            Category::OverUnder => "Over-unders",
            Category::Vo2 => "VO2-style intervals",
            Category::Anaerobic => "Anaerobic / sprint",
            Category::Cadence => "Cadence drills",
            Category::Climbing => "Climbing preparation",
            Category::Assessment => "Assessment",
            Category::TestFixture => "Technical test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    FtpPct,
    Watts,
    Rpe,
}
json_enum!(Basis { FtpPct = "ftp_pct", Watts = "watts", Rpe = "rpe" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Warmup,
    Steady,
    Work,
    Rest,
    Cooldown,
}
json_enum!(StepKind { Warmup = "warmup", Steady = "steady", Work = "work", Rest = "rest", Cooldown = "cooldown" });

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub basis: Basis,
    pub value: f64,
    /// When present the step is a linear ramp from `value` to `end`.
    pub end: Option<f64>,
}
json_struct!(Target { basis: "basis", value: "value", end: "end" });

#[derive(Debug, Clone, PartialEq)]
pub struct CadenceCue {
    pub lo: u16,
    pub hi: u16,
}
json_struct!(CadenceCue { lo: "lo", hi: "hi" });

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub kind: StepKind,
    pub dur_s: u32,
    pub target: Target,
    pub cadence: Option<CadenceCue>,
    pub text: Option<String>,
}
json_struct!(Step { kind: "kind", dur_s: "dur_s", target: "target", cadence: "cadence", text: "text" });

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub count: u32,
    pub steps: Vec<Step>,
}
json_struct!(Block { count: "count", steps: "steps" });

#[derive(Debug, Clone, PartialEq)]
pub struct Workout {
    pub id: String,
    pub schema: u32,
    pub version: u32,
    pub name: String,
    pub category: Category,
    /// 1 (very easy) … 5 (very hard), author-assigned.
    pub difficulty: u8,
    pub description: String,
    pub purpose: String,
    pub blocks: Vec<Block>,
    pub builtin: bool,
    /// Variant family for short/medium/long versions, e.g. "sweet-spot-intervals".
    pub family: String,
    /// Coaching-policy version the template was reviewed against.
    pub policy_version: String,
    pub created_utc: i64,
    pub updated_utc: i64,
}
json_struct!(Workout {
    id: "id",
    schema: "schema" = WORKOUT_SCHEMA_VERSION,
    version: "version" = 1,
    name: "name",
    category: "category",
    difficulty: "difficulty",
    description: "description" = String::new(),
    purpose: "purpose" = String::new(),
    blocks: "blocks",
    builtin: "builtin" = false,
    family: "family" = String::new(),
    policy_version: "policy_version" = String::new(),
    created_utc: "created_utc" = 0,
    updated_utc: "updated_utc" = 0,
});

/// Workout-level intensity class used by the planner and coaching policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Intensity {
    Easy,
    Moderate,
    Hard,
    /// Maximal efforts: assessments and anaerobic work. Always policy-gated.
    Maximal,
}
json_enum!(Intensity { Easy = "easy", Moderate = "moderate", Hard = "hard", Maximal = "maximal" });

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}
json_struct!(Issue { path: "path", message: "message" });

pub mod limits {
    pub const MIN_TOTAL_S: u32 = 5 * 60;
    pub const MAX_TOTAL_S: u32 = 6 * 3600;
    pub const MIN_STEP_S: u32 = 5;
    pub const MAX_STEP_S: u32 = 3 * 3600;
    pub const MAX_REPEAT: u32 = 30;
    pub const MAX_STEPS_PER_BLOCK: usize = 10;
    pub const MAX_BLOCKS: usize = 40;
    pub const MAX_FLAT_STEPS: usize = 400;
    pub const MAX_FTP_PCT: f64 = 200.0;
    pub const MAX_WATTS: f64 = 2000.0;
    pub const CADENCE_MIN: u16 = 40;
    pub const CADENCE_MAX: u16 = 140;
    /// Steps above this %FTP make a workout anaerobic and policy-gated.
    pub const ANAEROBIC_PCT: f64 = 120.0;
}

/// Approximate %FTP equivalent of an RPE value (Borg CR10-style 1–10), used
/// only for classification and for showing guidance, never for ERG targets.
pub fn rpe_to_pct(rpe: f64) -> f64 {
    match rpe.round() as i64 {
        i64::MIN..=1 => 40.0,
        2 => 50.0,
        3 => 62.0,
        4 => 72.0,
        5 => 82.0,
        6 => 90.0,
        7 => 98.0,
        8 => 108.0,
        9 => 125.0,
        _ => 150.0,
    }
}

/// Perceived-effort guidance for a %FTP target (riders without reliable power).
pub fn pct_to_rpe(pct: f64) -> u8 {
    match pct {
        p if p < 45.0 => 1,
        p if p < 56.0 => 2,
        p if p < 68.0 => 3,
        p if p < 77.0 => 4,
        p if p < 86.0 => 5,
        p if p < 94.0 => 6,
        p if p < 103.0 => 7,
        p if p < 115.0 => 8,
        p if p < 135.0 => 9,
        _ => 10,
    }
}

pub fn rpe_words(rpe: u8) -> &'static str {
    match rpe {
        0..=1 => "very easy, effortless spinning",
        2 => "easy, could chat freely",
        3 => "comfortable, full sentences",
        4 => "steady, sentences get shorter",
        5 => "moderate, a few words at a time",
        6 => "comfortably hard, focused breathing",
        7 => "hard, only short phrases",
        8 => "very hard, a word or two",
        9 => "near maximal",
        _ => "maximal",
    }
}

impl Target {
    pub fn ftp(v: f64) -> Target {
        Target { basis: Basis::FtpPct, value: v, end: None }
    }
    pub fn ramp(a: f64, b: f64) -> Target {
        Target { basis: Basis::FtpPct, value: a, end: Some(b) }
    }
    pub fn watts(v: f64) -> Target {
        Target { basis: Basis::Watts, value: v, end: None }
    }
    pub fn rpe(v: f64) -> Target {
        Target { basis: Basis::Rpe, value: v, end: None }
    }
    /// Value at fraction `f` (0..1) of the step.
    pub fn value_at(&self, f: f64) -> f64 {
        match self.end {
            Some(e) => self.value + (e - self.value) * f.clamp(0.0, 1.0),
            None => self.value,
        }
    }
    /// %FTP equivalent at fraction `f`, using `ftp` for watt targets.
    pub fn pct_at(&self, f: f64, ftp: Option<f64>) -> Option<f64> {
        let v = self.value_at(f);
        match self.basis {
            Basis::FtpPct => Some(v),
            Basis::Watts => ftp.filter(|f| *f > 0.0).map(|f| 100.0 * v / f),
            Basis::Rpe => Some(rpe_to_pct(v)),
        }
    }
    /// Derived watts. `None` for RPE targets or when FTP is unknown for %FTP targets.
    pub fn watts_at(&self, f: f64, ftp: Option<f64>) -> Option<f64> {
        let v = self.value_at(f);
        match self.basis {
            Basis::FtpPct => ftp.filter(|x| *x > 0.0).map(|x| x * v / 100.0),
            Basis::Watts => Some(v),
            Basis::Rpe => None,
        }
    }
    pub fn max_value(&self) -> f64 {
        self.end.map(|e| e.max(self.value)).unwrap_or(self.value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineStep {
    pub index: usize,
    pub block: usize,
    pub rep: u32,
    pub reps: u32,
    pub start_s: u32,
    pub dur_s: u32,
    pub kind: StepKind,
    pub target: Target,
    pub cadence: Option<CadenceCue>,
    pub text: Option<String>,
    pub label: String,
}

impl ToJson for TimelineStep {
    fn to_json(&self) -> Value {
        Value::obj([
            ("index", self.index.into()),
            ("block", self.block.into()),
            ("rep", self.rep.into()),
            ("reps", self.reps.into()),
            ("start_s", self.start_s.into()),
            ("dur_s", self.dur_s.into()),
            ("kind", self.kind.to_json()),
            ("target", self.target.to_json()),
            ("cadence", self.cadence.to_json()),
            ("text", self.text.clone().into()),
            ("label", self.label.clone().into()),
        ])
    }
}

fn kind_label(k: StepKind) -> &'static str {
    match k {
        StepKind::Warmup => "Warm-up",
        StepKind::Steady => "Steady",
        StepKind::Work => "Interval",
        StepKind::Rest => "Recovery",
        StepKind::Cooldown => "Cool-down",
    }
}

impl Workout {
    pub fn total_s(&self) -> u32 {
        self.blocks
            .iter()
            .map(|b| b.count.saturating_mul(b.steps.iter().map(|s| s.dur_s).fold(0u32, |a, d| a.saturating_add(d))))
            .fold(0u32, |a, d| a.saturating_add(d))
    }

    pub fn timeline(&self) -> Vec<TimelineStep> {
        let mut out = Vec::new();
        let mut t = 0u32;
        let work_total: u32 = self
            .blocks
            .iter()
            .map(|b| b.count * b.steps.iter().filter(|s| s.kind == StepKind::Work).count() as u32)
            .sum();
        let mut work_n = 0;
        for (bi, b) in self.blocks.iter().enumerate() {
            for rep in 0..b.count {
                for s in &b.steps {
                    let label = if s.kind == StepKind::Work {
                        work_n += 1;
                        format!("Interval {work_n}/{work_total}")
                    } else if b.count > 1 {
                        format!("{} {}/{}", kind_label(s.kind), rep + 1, b.count)
                    } else {
                        kind_label(s.kind).to_string()
                    };
                    out.push(TimelineStep {
                        index: out.len(),
                        block: bi,
                        rep,
                        reps: b.count,
                        start_s: t,
                        dur_s: s.dur_s,
                        kind: s.kind,
                        target: s.target.clone(),
                        cadence: s.cadence.clone(),
                        text: s.text.clone(),
                        label,
                    });
                    t = t.saturating_add(s.dur_s);
                }
            }
        }
        out
    }

    pub fn uses_basis(&self, basis: Basis) -> bool {
        self.blocks.iter().any(|b| b.steps.iter().any(|s| s.target.basis == basis))
    }
    pub fn has_cadence_cues(&self) -> bool {
        self.blocks.iter().any(|b| b.steps.iter().any(|s| s.cadence.is_some()))
    }
    pub fn max_pct(&self, ftp: Option<f64>) -> f64 {
        self.timeline()
            .iter()
            .map(|s| s.target.pct_at(1.0, ftp).unwrap_or(0.0).max(s.target.pct_at(0.0, ftp).unwrap_or(0.0)))
            .fold(0.0, f64::max)
    }

    /// Seconds spent at or above `pct` %FTP (ramps sampled each second).
    pub fn seconds_at_or_above(&self, pct: f64) -> u32 {
        let mut n = 0;
        for s in self.timeline() {
            for i in 0..s.dur_s {
                let f = if s.dur_s > 1 { i as f64 / (s.dur_s - 1) as f64 } else { 0.0 };
                if s.target.pct_at(f, None).unwrap_or(0.0) >= pct {
                    n += 1;
                }
            }
        }
        n
    }

    /// Content-based intensity class (same rule for built-in and custom workouts).
    pub fn intensity(&self) -> Intensity {
        if matches!(self.category, Category::Assessment | Category::Anaerobic) || self.max_pct(None) > limits::ANAEROBIC_PCT {
            return Intensity::Maximal;
        }
        let above_95 = self.seconds_at_or_above(95.0);
        let above_76 = self.seconds_at_or_above(76.0);
        if above_95 >= 6 * 60 || self.max_pct(None) >= 110.0 {
            Intensity::Hard
        } else if above_76 >= 15 * 60 {
            Intensity::Moderate
        } else {
            Intensity::Easy
        }
    }

    /// Structural validation. Returns all issues (empty = valid).
    pub fn validate(&self) -> Vec<Issue> {
        use limits::*;
        let mut v = Vec::new();
        let mut add = |path: String, m: String| v.push(Issue { path, message: m });
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            add("name".into(), "Name must be 1–80 characters.".into());
        }
        if self.description.chars().count() > 800 {
            add("description".into(), "Description must be at most 800 characters.".into());
        }
        if !(1..=5).contains(&self.difficulty) {
            add("difficulty".into(), "Difficulty must be 1–5.".into());
        }
        if self.schema != WORKOUT_SCHEMA_VERSION {
            add("schema".into(), format!("Unsupported workout schema {}.", self.schema));
        }
        if self.blocks.is_empty() {
            add("blocks".into(), "A workout needs at least one step.".into());
        }
        if self.blocks.len() > MAX_BLOCKS {
            add("blocks".into(), format!("At most {MAX_BLOCKS} blocks."));
        }
        let mut flat = 0usize;
        for (bi, b) in self.blocks.iter().enumerate() {
            let bp = format!("blocks[{bi}]");
            if b.count == 0 || b.count > MAX_REPEAT {
                add(format!("{bp}.count"), format!("Repeat count must be 1–{MAX_REPEAT}."));
            }
            if b.steps.is_empty() || b.steps.len() > MAX_STEPS_PER_BLOCK {
                add(format!("{bp}.steps"), format!("Each block needs 1–{MAX_STEPS_PER_BLOCK} steps."));
            }
            flat += b.count as usize * b.steps.len();
            for (si, s) in b.steps.iter().enumerate() {
                let sp = format!("{bp}.steps[{si}]");
                if s.dur_s < MIN_STEP_S || s.dur_s > MAX_STEP_S {
                    add(format!("{sp}.dur_s"), format!("Step duration must be {MIN_STEP_S} s – 3 h."));
                }
                let t = &s.target;
                let (lo, hi, unit) = match t.basis {
                    Basis::FtpPct => (0.0, MAX_FTP_PCT, "% FTP"),
                    Basis::Watts => (0.0, MAX_WATTS, "W"),
                    Basis::Rpe => (1.0, 10.0, "RPE"),
                };
                for (nm, x) in [("value", Some(t.value)), ("end", t.end)] {
                    if let Some(x) = x {
                        if !x.is_finite() || x < lo || x > hi {
                            add(format!("{sp}.target.{nm}"), format!("Target must be {lo}–{hi} {unit}."));
                        }
                    }
                }
                if t.basis == Basis::Rpe && t.end.is_some() {
                    add(format!("{sp}.target.end"), "Perceived-effort steps cannot ramp.".into());
                }
                if let Some(c) = &s.cadence {
                    if c.lo < CADENCE_MIN || c.hi > CADENCE_MAX || c.lo > c.hi {
                        add(format!("{sp}.cadence"), format!("Cadence cue must be within {CADENCE_MIN}–{CADENCE_MAX} rpm, low ≤ high."));
                    }
                }
                if let Some(tx) = &s.text {
                    if tx.chars().count() > 200 {
                        add(format!("{sp}.text"), "Step text must be at most 200 characters.".into());
                    }
                }
            }
        }
        if flat > MAX_FLAT_STEPS {
            add("blocks".into(), format!("Too many steps after expanding repeats (max {MAX_FLAT_STEPS})."));
        }
        let total = self.total_s();
        if (total < MIN_TOTAL_S || total > MAX_TOTAL_S) && self.category != Category::TestFixture {
            add("duration".into(), "Total duration must be between 5 minutes and 6 hours.".into());
        }
        if self.max_pct(None) > ANAEROBIC_PCT && !matches!(self.category, Category::Anaerobic | Category::Assessment | Category::TestFixture) {
            add(
                "category".into(),
                format!("Targets above {ANAEROBIC_PCT}% FTP are only allowed in Anaerobic or Assessment workouts, which the coaching policy gates."),
            );
        }
        v
    }

    pub fn summary_json(&self) -> Value {
        Value::obj([
            ("id", self.id.clone().into()),
            ("name", self.name.clone().into()),
            ("category", self.category.to_json()),
            ("category_label", self.category.label().into()),
            ("difficulty", self.difficulty.into()),
            ("duration_s", self.total_s().into()),
            ("intensity", self.intensity().to_json()),
            ("builtin", self.builtin.into()),
            ("family", self.family.clone().into()),
            ("description", self.description.clone().into()),
            ("purpose", self.purpose.clone().into()),
            ("uses_power", (self.uses_basis(Basis::FtpPct) || self.uses_basis(Basis::Watts)).into()),
            ("uses_rpe", self.uses_basis(Basis::Rpe).into()),
            ("cadence_cues", self.has_cadence_cues().into()),
            ("version", self.version.into()),
            // Compact profile for list thumbnails: (start_s, dur_s, pct_start, pct_end)
            (
                "profile",
                Value::Arr(
                    self.timeline()
                        .iter()
                        .map(|s| {
                            Value::Arr(vec![
                                s.start_s.into(),
                                s.dur_s.into(),
                                s.target.pct_at(0.0, None).unwrap_or(0.0).into(),
                                s.target.pct_at(1.0, None).unwrap_or(0.0).into(),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

pub fn parse_workout(v: &Value) -> JResult<Workout> {
    Workout::from_json(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple() -> Workout {
        Workout {
            id: "t".into(),
            schema: 1,
            version: 1,
            name: "Test".into(),
            category: Category::Threshold,
            difficulty: 3,
            description: String::new(),
            purpose: String::new(),
            blocks: vec![
                Block { count: 1, steps: vec![Step { kind: StepKind::Warmup, dur_s: 600, target: Target::ramp(45.0, 75.0), cadence: None, text: None }] },
                Block {
                    count: 3,
                    steps: vec![
                        Step { kind: StepKind::Work, dur_s: 300, target: Target::ftp(100.0), cadence: Some(CadenceCue { lo: 85, hi: 95 }), text: None },
                        Step { kind: StepKind::Rest, dur_s: 120, target: Target::ftp(55.0), cadence: None, text: None },
                    ],
                },
                Block { count: 1, steps: vec![Step { kind: StepKind::Cooldown, dur_s: 300, target: Target::ramp(60.0, 40.0), cadence: None, text: None }] },
            ],
            builtin: false,
            family: String::new(),
            policy_version: String::new(),
            created_utc: 0,
            updated_utc: 0,
        }
    }

    #[test]
    fn timeline_and_targets() {
        let w = simple();
        assert_eq!(w.total_s(), 600 + 3 * 420 + 300);
        let tl = w.timeline();
        assert_eq!(tl.len(), 8);
        assert_eq!(tl[1].label, "Interval 1/3");
        assert_eq!(tl[2].label, "Recovery 1/3");
        assert_eq!(tl[7].start_s, 600 + 1260);
        assert_eq!(tl[0].target.watts_at(0.5, Some(200.0)), Some(120.0));
        assert_eq!(Target::rpe(5.0).watts_at(0.0, Some(250.0)), None);
        assert_eq!(Target::ftp(90.0).watts_at(0.0, None), None);
        assert_eq!(Target::watts(180.0).watts_at(0.0, None), Some(180.0));
        assert!(w.validate().is_empty(), "{:?}", w.validate());
        assert_eq!(w.intensity(), Intensity::Hard);
    }

    #[test]
    fn validation_catches_errors() {
        let mut w = simple();
        w.blocks[1].count = 0;
        w.blocks[1].steps[0].target.value = f64::NAN;
        w.blocks[1].steps[0].cadence = Some(CadenceCue { lo: 100, hi: 90 });
        w.blocks[0].steps[0].dur_s = 2;
        let issues = w.validate();
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"blocks[1].count"));
        assert!(paths.contains(&"blocks[1].steps[0].target.value"));
        assert!(paths.contains(&"blocks[1].steps[0].cadence"));
        assert!(paths.contains(&"blocks[0].steps[0].dur_s"));
        let mut w = simple();
        w.blocks[1].steps[0].target = Target::ftp(150.0);
        assert!(w.validate().iter().any(|i| i.path == "category"));
    }

    #[test]
    fn json_roundtrip() {
        let w = simple();
        let j = w.to_json().to_string_compact();
        let back = Workout::from_json(&rl_json::parse(&j).unwrap()).unwrap();
        assert_eq!(back, w);
    }
}
