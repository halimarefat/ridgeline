//! Recording types (1 Hz samples, laps, session events), the sink contract
//! implemented by storage, and activity summaries.
//!
//! Missing values are `None` (serialized as null), never zero; stale values
//! are not forward-filled into samples or totals. Commanded trainer targets
//! are stored separately from reported (measured) values.

use rl_domain::zones::{zone_index, POWER_ZONES};
use rl_json::{json_struct, FromJson, ToJson, Value};

pub mod flags {
    pub const POWER_STALE: u32 = 1 << 0;
    pub const CADENCE_STALE: u32 = 1 << 1;
    pub const HR_STALE: u32 = 1 << 2;
    pub const CONTROL_LOST: u32 = 1 << 3;
    pub const LOW_CADENCE_RECOVERY: u32 = 1 << 4;
    pub const APPROX_PROGRESSION: u32 = 1 << 5;
    pub const FLAT_FALLBACK: u32 = 1 << 6;
    pub const DEMO: u32 = 1 << 7;
    pub const TRAINER_SATURATED: u32 = 1 << 8;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// Monotonic milliseconds since session start (includes pauses).
    pub t_ms: u64,
    pub utc_ms: i64,
    /// Active (timer) seconds; increases by one per sample.
    pub active_s: u32,
    pub power: Option<f64>,
    pub cadence: Option<f64>,
    pub hr: Option<f64>,
    /// Virtual road speed from the physics model (m/s).
    pub speed: Option<f64>,
    pub distance: Option<f64>,
    pub ele: Option<f64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub grade: Option<f64>,
    /// Grade actually commanded to the trainer (after difficulty/limits).
    pub cmd_grade: Option<f64>,
    /// ERG target commanded (W).
    pub target_w: Option<f64>,
    pub trainer_speed: Option<f64>,
    pub step: Option<u32>,
    pub power_src: Option<String>,
    pub cadence_src: Option<String>,
    pub hr_src: Option<String>,
    pub flags: u32,
}
json_struct!(Sample {
    t_ms: "t",
    utc_ms: "u",
    active_s: "a",
    power: "p",
    cadence: "c",
    hr: "h",
    speed: "v",
    distance: "d",
    ele: "e",
    lat: "lat",
    lon: "lon",
    grade: "g",
    cmd_grade: "cg",
    target_w: "tw",
    trainer_speed: "ts",
    step: "st",
    power_src: "ps",
    cadence_src: "cs",
    hr_src: "hs",
    flags: "f" = 0,
});

#[derive(Debug, Clone, PartialEq)]
pub struct Lap {
    pub index: u32,
    pub start_t_ms: u64,
    pub end_t_ms: u64,
    pub start_active_s: u32,
    pub end_active_s: u32,
    pub start_utc_ms: i64,
    pub label: String,
    /// "step", "manual", "route_lap", "session_end"
    pub trigger: String,
    pub step: Option<u32>,
    pub target_w: Option<f64>,
}
json_struct!(Lap {
    index: "index",
    start_t_ms: "start_t_ms",
    end_t_ms: "end_t_ms",
    start_active_s: "start_active_s",
    end_active_s: "end_active_s",
    start_utc_ms: "start_utc_ms" = 0,
    label: "label",
    trigger: "trigger",
    step: "step",
    target_w: "target_w",
});

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    pub t_ms: u64,
    pub utc_ms: i64,
    pub kind: String,
    pub detail: String,
}
json_struct!(SessionEvent { t_ms: "t", utc_ms: "u", kind: "kind", detail: "detail" });

/// Persistence contract. Implementations must make samples durable within a
/// few seconds (journal + periodic fsync) so a crash loses little data.
pub trait RecordSink: Send {
    fn sample(&mut self, s: &Sample) -> Result<(), String>;
    fn event(&mut self, e: &SessionEvent) -> Result<(), String>;
    fn lap(&mut self, l: &Lap) -> Result<(), String>;
    /// Called every tick; implementations flush/fsync on their own schedule.
    fn flush(&mut self, force: bool) -> Result<(), String>;
}

/// In-memory sink for tests and demo previews.
#[derive(Default)]
pub struct MemorySink {
    pub samples: Vec<Sample>,
    pub events: Vec<SessionEvent>,
    pub laps: Vec<Lap>,
    pub fail: bool,
}
impl RecordSink for MemorySink {
    fn sample(&mut self, s: &Sample) -> Result<(), String> {
        if self.fail {
            return Err("disk full (simulated)".into());
        }
        self.samples.push(s.clone());
        Ok(())
    }
    fn event(&mut self, e: &SessionEvent) -> Result<(), String> {
        self.events.push(e.clone());
        Ok(())
    }
    fn lap(&mut self, l: &Lap) -> Result<(), String> {
        self.laps.push(l.clone());
        Ok(())
    }
    fn flush(&mut self, _force: bool) -> Result<(), String> {
        Ok(())
    }
}

// ------------------------------------------------------------ summaries

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stat {
    pub avg: Option<f64>,
    pub max: Option<f64>,
    /// Fraction of samples with a value (0..1).
    pub coverage: f64,
}

impl ToJson for Stat {
    fn to_json(&self) -> Value {
        Value::obj([("avg", self.avg.into()), ("max", self.max.into()), ("coverage", self.coverage.into())])
    }
}
impl FromJson for Stat {
    fn from_json(v: &Value) -> rl_json::JResult<Self> {
        Ok(Stat { avg: rl_json::field(v, "avg")?, max: rl_json::field(v, "max")?, coverage: rl_json::field_or(v, "coverage", || 0.0)? })
    }
}

pub fn stat(vals: impl Iterator<Item = Option<f64>>, exclude_zero_from_avg: bool) -> Stat {
    let mut n = 0usize;
    let mut have = 0usize;
    let mut sum = 0.0;
    let mut cnt = 0usize;
    let mut max: Option<f64> = None;
    for v in vals {
        n += 1;
        if let Some(x) = v {
            have += 1;
            max = Some(max.map_or(x, |m: f64| m.max(x)));
            if !(exclude_zero_from_avg && x <= 0.0) {
                sum += x;
                cnt += 1;
            }
        }
    }
    Stat { avg: if cnt > 0 { Some(sum / cnt as f64) } else { None }, max, coverage: if n > 0 { have as f64 / n as f64 } else { 0.0 } }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LapSummary {
    pub index: u32,
    pub label: String,
    pub duration_s: u32,
    pub power: Stat,
    pub hr: Stat,
    pub cadence: Stat,
    pub target_w: Option<f64>,
    /// |avg power − target| / target, when both exist.
    pub target_error: Option<f64>,
    pub distance_m: Option<f64>,
}
json_struct!(LapSummary {
    index: "index",
    label: "label",
    duration_s: "duration_s",
    power: "power",
    hr: "hr",
    cadence: "cadence",
    target_w: "target_w",
    target_error: "target_error",
    distance_m: "distance_m",
});

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub elapsed_s: f64,
    pub timer_s: f64,
    pub moving_s: f64,
    pub distance_m: Option<f64>,
    pub ascent_m: Option<f64>,
    pub power: Stat,
    pub hr: Stat,
    pub cadence: Stat,
    pub speed: Stat,
    pub work_kj: Option<f64>,
    /// Estimate only: mechanical work in kJ ≈ kcal metabolic (≈24 % efficiency).
    pub energy_kcal_estimate: Option<f64>,
    pub time_in_zones_s: Vec<f64>,
    pub zone_names: Vec<String>,
    /// Fraction of ERG-target samples within ±10 % of target.
    pub adherence: Option<f64>,
    pub laps: Vec<LapSummary>,
    pub gaps: u32,
    pub stale_power_s: u32,
    pub control_lost_s: u32,
    pub samples: u32,
}
json_struct!(Summary {
    elapsed_s: "elapsed_s",
    timer_s: "timer_s",
    moving_s: "moving_s",
    distance_m: "distance_m",
    ascent_m: "ascent_m",
    power: "power",
    hr: "hr",
    cadence: "cadence",
    speed: "speed",
    work_kj: "work_kj",
    energy_kcal_estimate: "energy_kcal_estimate",
    time_in_zones_s: "time_in_zones_s",
    zone_names: "zone_names",
    adherence: "adherence",
    laps: "laps",
    gaps: "gaps" = 0,
    stale_power_s: "stale_power_s" = 0,
    control_lost_s: "control_lost_s" = 0,
    samples: "samples" = 0,
});

pub fn summarize(samples: &[Sample], laps: &[Lap], elapsed_s: f64, ftp: Option<f64>) -> Summary {
    let timer_s = samples.len() as f64;
    let moving_s = samples
        .iter()
        .filter(|s| s.power.unwrap_or(0.0) > 0.0 || s.cadence.unwrap_or(0.0) > 0.0 || s.speed.unwrap_or(0.0) > 0.3)
        .count() as f64;
    let distance = samples.iter().rev().find_map(|s| s.distance);
    let mut ascent: Option<f64> = None;
    let mut ref_e: Option<f64> = None;
    for e in samples.iter().filter_map(|s| s.ele) {
        let a = ascent.get_or_insert(0.0);
        match ref_e {
            None => ref_e = Some(e),
            Some(r) if e - r >= 0.5 => {
                *a += e - r;
                ref_e = Some(e);
            }
            Some(r) if r - e >= 0.5 => ref_e = Some(e),
            _ => {}
        }
    }
    let power = stat(samples.iter().map(|s| s.power), false);
    let work: f64 = samples.iter().filter_map(|s| s.power).map(|p| p.max(0.0)).sum::<f64>() / 1000.0;
    let mut zones = vec![0.0; POWER_ZONES.len()];
    if let Some(f) = ftp {
        for p in samples.iter().filter_map(|s| s.power) {
            if let Some(z) = zone_index(p, f) {
                zones[z] += 1.0;
            }
        }
    }
    let with_target: Vec<&Sample> = samples.iter().filter(|s| s.target_w.is_some() && s.power.is_some()).collect();
    let adherence = if with_target.len() >= 10 {
        let ok = with_target
            .iter()
            .filter(|s| {
                let t = s.target_w.unwrap();
                t > 0.0 && (s.power.unwrap() - t).abs() / t <= 0.10
            })
            .count();
        Some(ok as f64 / with_target.len() as f64)
    } else {
        None
    };
    let mut gaps = 0;
    for w in samples.windows(2) {
        if w[1].t_ms.saturating_sub(w[0].t_ms) > 2500 && w[1].active_s == w[0].active_s + 1 {
            gaps += 1;
        }
    }
    let lap_summaries = laps
        .iter()
        .map(|l| {
            let ss: Vec<&Sample> = samples.iter().filter(|s| s.active_s > l.start_active_s && s.active_s <= l.end_active_s).collect();
            let p = stat(ss.iter().map(|s| s.power), false);
            let target_error = match (p.avg, l.target_w) {
                (Some(a), Some(t)) if t > 0.0 => Some((a - t).abs() / t),
                _ => None,
            };
            let d = match (ss.first().and_then(|s| s.distance), ss.last().and_then(|s| s.distance)) {
                (Some(a), Some(b)) => Some((b - a).max(0.0)),
                _ => None,
            };
            LapSummary {
                index: l.index,
                label: l.label.clone(),
                duration_s: l.end_active_s.saturating_sub(l.start_active_s),
                power: p,
                hr: stat(ss.iter().map(|s| s.hr), false),
                cadence: stat(ss.iter().map(|s| s.cadence), true),
                target_w: l.target_w,
                target_error,
                distance_m: d,
            }
        })
        .collect();
    Summary {
        elapsed_s,
        timer_s,
        moving_s,
        distance_m: distance,
        ascent_m: ascent,
        power: power.clone(),
        hr: stat(samples.iter().map(|s| s.hr), false),
        cadence: stat(samples.iter().map(|s| s.cadence), true),
        speed: stat(samples.iter().map(|s| s.speed), true),
        work_kj: if power.coverage > 0.0 { Some(work) } else { None },
        energy_kcal_estimate: if power.coverage > 0.0 { Some(work) } else { None },
        time_in_zones_s: if ftp.is_some() { zones } else { vec![] },
        zone_names: POWER_ZONES.iter().map(|z| z.0.to_string()).collect(),
        adherence,
        laps: lap_summaries,
        gaps,
        stale_power_s: samples.iter().filter(|s| s.flags & flags::POWER_STALE != 0).count() as u32,
        control_lost_s: samples.iter().filter(|s| s.flags & flags::CONTROL_LOST != 0).count() as u32,
        samples: samples.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_is_not_zero() {
        let samples: Vec<Sample> = (0..10)
            .map(|i| Sample { t_ms: i * 1000, active_s: i as u32 + 1, power: if i < 5 { Some(200.0) } else { None }, cadence: Some(if i % 2 == 0 { 0.0 } else { 90.0 }), ..Default::default() })
            .collect();
        let s = summarize(&samples, &[], 10.0, Some(250.0));
        assert_eq!(s.power.avg, Some(200.0), "missing power excluded from average");
        assert!((s.power.coverage - 0.5).abs() < 1e-9);
        assert_eq!(s.cadence.avg, Some(90.0), "zero cadence excluded from average");
        assert_eq!(s.work_kj, Some(1.0));
        assert_eq!(s.hr.avg, None);
        assert_eq!(s.time_in_zones_s[2], 5.0); // 80 % FTP = tempo
    }

    #[test]
    fn sample_json_roundtrip_keeps_nulls() {
        let s = Sample { t_ms: 1000, utc_ms: 5, active_s: 1, power: Some(0.0), hr: None, ..Default::default() };
        let j = s.to_json().to_string_compact();
        assert!(j.contains("\"p\":0") && j.contains("\"h\":null"), "{j}");
        assert_eq!(Sample::from_json(&rl_json::parse(&j).unwrap()).unwrap(), s);
    }
}
