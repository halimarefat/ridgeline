//! In-ride coach feed: deterministic ride cues plus room for coach replies.
//!
//! Cues are computed from the ride's own state (workout steps, recorded
//! samples, the route profile) once per recorded second. They are text with
//! at most one *suggested* action; nothing here sends a trainer command or
//! changes the workout. The rider applies a suggestion through the normal
//! ride controls (spec: the coach may suggest, the rider accepts).
//!
//! Some cues are marked as "moments" (a hard interval starting, halfway, the
//! last interval, a long climb ahead). The application may ask an AI coach
//! to comment on those, off the control path and rate limited.

use crate::record::Sample;
use crate::route_engine::RouteRun;
use crate::workout_engine::{StepState, WorkoutRun};
use rl_domain::workout::Basis;
use rl_json::Value;

/// Effort at or above this % of FTP counts as a hard interval for cues.
pub const HARD_PCT: f64 = 88.0;
/// Seconds before a step change at which the next step is previewed.
pub const PREVIEW_S: f64 = 15.0;
const MAX_FEED: usize = 40;

/// A suggestion the rider can apply with one press. Only these ride
/// controls can be suggested; the UI maps them to the same commands as the
/// ride buttons and keys.
#[derive(Debug, Clone, PartialEq)]
pub enum CueAction {
    /// Change workout intensity by this many percent (same as + / −).
    Intensity(i32),
    /// Open the stop dialog (the rider still confirms there).
    Stop,
}

impl CueAction {
    pub fn to_json(&self) -> Value {
        match self {
            CueAction::Intensity(d) => Value::obj([
                ("kind", "intensity".into()),
                ("delta", (*d).into()),
                ("label", (if *d < 0 { format!("Easier {}%", -d) } else { format!("Harder {d}%") }).into()),
            ]),
            CueAction::Stop => Value::obj([("kind", "stop".into()), ("label", "Stop the ride".into())]),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub id: u64,
    /// Ride timer seconds when added.
    pub at_s: f64,
    /// "cue" (rules), "ai" (model), "coach" (offline coach), "rider", "safety", "note"
    pub from: &'static str,
    pub kind: String,
    pub text: String,
    pub action: Option<CueAction>,
    /// Worth reading aloud when voice is on.
    pub speak: bool,
}

impl FeedItem {
    pub fn to_json(&self) -> Value {
        Value::obj([
            ("id", self.id.into()),
            ("at_s", self.at_s.round().into()),
            ("from", self.from.into()),
            ("kind", self.kind.clone().into()),
            ("text", self.text.clone().into()),
            ("action", self.action.as_ref().map(|a| a.to_json()).unwrap_or(Value::Null)),
            ("speak", self.speak.into()),
        ])
    }
}

/// A cue the AI coach may comment on.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    pub kind: String,
    pub text: String,
}

/// Everything the cue rules look at, gathered by the session each second.
pub struct CueInput<'a> {
    pub active_s: f64,
    pub workout: Option<&'a WorkoutRun>,
    pub step: Option<&'a StepState>,
    pub route: Option<&'a RouteRun>,
    /// Recorded 1 Hz samples so far (most recent last).
    pub samples: &'a [Sample],
    /// The workout engine currently holds ERG targets on a controlled trainer.
    pub erg_active: bool,
    pub low_cadence_active: bool,
    pub imperial: bool,
}

#[derive(Default)]
pub struct RideCoach {
    seq: u64,
    pub feed: Vec<FeedItem>,
    last_kind_s: Vec<(String, f64)>,
    previewed_step: Option<usize>,
    announced_step: Option<usize>,
    halfway_done: bool,
    /// (start_m, end_m) of climbs already announced, by route lap.
    climbs: Vec<(u32, f64, f64)>,
    topped: Vec<(u32, f64)>,
    pending_moment: Option<Moment>,
}

pub fn fmt_dur(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 90 {
        format!("{s} s")
    } else if s % 60 == 0 || s >= 600 {
        format!("{} min", (s as f64 / 60.0).round() as u64)
    } else {
        format!("{}:{:02} min", s / 60, s % 60)
    }
}

pub fn fmt_dist(m: f64, imperial: bool) -> String {
    if imperial {
        let ft = m / 0.3048;
        if ft < 1000.0 {
            format!("{} ft", ((ft / 50.0).round() * 50.0) as u64)
        } else {
            format!("{:.1} mi", m / 1609.344)
        }
    } else if m < 1000.0 {
        format!("{} m", ((m / 50.0).round() * 50.0) as u64)
    } else {
        format!("{:.1} km", m / 1000.0)
    }
}

fn mean(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (n, sum) = v.fold((0usize, 0.0), |(n, s), x| (n + 1, s + x));
    (n > 0).then(|| sum / n as f64)
}

/// Is timeline step `i` a hard effort of at least a minute?
fn step_is_hard(w: &WorkoutRun, i: usize) -> bool {
    let s = &w.timeline[i];
    if s.dur_s < 60 || s.target.basis == Basis::Rpe || w.rpe_mode {
        return false;
    }
    let f = 1.0 + w.adjust_pct as f64 / 100.0;
    s.target.pct_at(0.5, w.ftp_snapshot).map(|p| p * f >= HARD_PCT).unwrap_or(false)
}

/// Describe a step's target in rider terms ("250 W", "effort 6/10").
fn step_target_text(w: &WorkoutRun, i: usize) -> Option<String> {
    let s = &w.timeline[i];
    let f = 1.0 + w.adjust_pct as f64 / 100.0;
    if s.target.basis == Basis::Rpe || w.rpe_mode {
        let rpe = match s.target.basis {
            Basis::Rpe => Some(s.target.value.round() as u8),
            _ => s.target.pct_at(0.5, w.ftp_snapshot).map(rl_domain::workout::pct_to_rpe),
        }?;
        return Some(format!("effort {rpe}/10"));
    }
    let (a, b) = (s.target.watts_at(0.0, w.ftp_snapshot)? * f, s.target.watts_at(1.0, w.ftp_snapshot)? * f);
    if (a - b).abs() >= 10.0 {
        Some(format!("{:.0}→{:.0} W", a, b))
    } else {
        Some(format!("{:.0} W", 0.5 * (a + b)))
    }
}

impl RideCoach {
    pub fn push(&mut self, at_s: f64, from: &'static str, kind: impl Into<String>, text: impl Into<String>, action: Option<CueAction>, speak: bool) -> &FeedItem {
        self.seq += 1;
        self.feed.push(FeedItem { id: self.seq, at_s, from, kind: kind.into(), text: text.into(), action, speak });
        if self.feed.len() > MAX_FEED {
            self.feed.remove(0);
        }
        self.feed.last().expect("just pushed")
    }

    /// Take the latest moment worth an AI comment, if any (older ones are
    /// superseded: a late comment on a past interval is not useful).
    pub fn take_moment(&mut self) -> Option<Moment> {
        self.pending_moment.take()
    }

    fn ready(&self, kind: &str, now_s: f64, every_s: f64) -> bool {
        self.last_kind_s.iter().find(|(k, _)| k == kind).map(|(_, t)| now_s - t >= every_s).unwrap_or(true)
    }

    fn mark(&mut self, kind: &str, now_s: f64) {
        match self.last_kind_s.iter_mut().find(|(k, _)| k == kind) {
            Some(e) => e.1 = now_s,
            None => self.last_kind_s.push((kind.to_string(), now_s)),
        }
    }

    fn cue(&mut self, inp: &CueInput, kind: &str, text: String, action: Option<CueAction>, moment: bool) -> Vec<FeedItem> {
        let item = self.push(inp.active_s, "cue", kind, text.clone(), action, true).clone();
        self.mark(kind, inp.active_s);
        if moment {
            self.pending_moment = Some(Moment { kind: kind.into(), text });
        }
        vec![item]
    }

    /// Evaluate the cue rules. Returns the items added this time.
    pub fn evaluate(&mut self, inp: &CueInput) -> Vec<FeedItem> {
        let mut out = Vec::new();
        if let (Some(w), Some(st)) = (inp.workout, inp.step) {
            out.extend(self.workout_cues(inp, w, st));
        }
        if let Some(r) = inp.route {
            out.extend(self.route_cues(inp, r));
        }
        out
    }

    fn workout_cues(&mut self, inp: &CueInput, w: &WorkoutRun, st: &StepState) -> Vec<FeedItem> {
        let mut out = Vec::new();
        if w.complete {
            return out;
        }
        let i = st.index;
        let last_hard = (0..w.timeline.len()).rev().find(|&j| step_is_hard(w, j) && !w.skipped.contains(&j));
        // Step start.
        if self.announced_step != Some(i) {
            self.announced_step = Some(i);
            let s = &w.timeline[i];
            let hard = step_is_hard(w, i);
            let target = step_target_text(w, i).map(|t| format!(" at {t}")).unwrap_or_default();
            let cad = s.cadence.as_ref().map(|c| format!(", {}–{} rpm", c.lo, c.hi)).unwrap_or_default();
            let lead = if hard && last_hard == Some(i) && (0..i).any(|j| step_is_hard(w, j)) { "Last hard one! " } else { "" };
            let mut text = format!("{lead}{}: {}{target}{cad}.", s.label, fmt_dur(s.dur_s as f64));
            if let Some(t) = s.text.as_ref().filter(|t| !t.trim().is_empty()) {
                text.push(' ');
                text.push_str(t.trim());
            }
            out.extend(self.cue(inp, "interval_start", text, None, hard));
        }
        // Preview of the next step.
        if let Some(n) = w.timeline.get(i + 1) {
            if st.remaining_s <= PREVIEW_S && st.remaining_s > 1.0 && n.dur_s >= 30 && self.previewed_step != Some(i + 1) && st.elapsed_s >= 5.0 {
                self.previewed_step = Some(i + 1);
                let target = step_target_text(w, i + 1).map(|t| format!(" at {t}")).unwrap_or_default();
                let text = format!("In {}: {} — {}{target}.", fmt_dur(st.remaining_s), n.label, fmt_dur(n.dur_s as f64));
                out.extend(self.cue(inp, "interval_soon", text, None, false));
            }
        }
        // Halfway through a workout of 20 minutes or more.
        if !self.halfway_done && w.total_s >= 1200.0 && w.pos_s >= w.total_s / 2.0 {
            self.halfway_done = true;
            let text = format!("Halfway — {} to go.", fmt_dur(w.remaining_s()));
            out.extend(self.cue(inp, "halfway", text, None, true));
        }
        let recent = |n: usize| inp.samples.iter().rev().take(n.min(st.elapsed_s as usize));
        // Cadence drifting outside the step's cue.
        if let Some((lo, hi)) = st.cadence {
            if st.elapsed_s >= 20.0 && self.ready("cadence", inp.active_s, 60.0) {
                let c = mean(recent(15).filter_map(|s| s.cadence));
                if let Some(c) = c.filter(|_| recent(15).filter(|s| s.cadence.is_some()).count() >= 10) {
                    if c < lo as f64 - 3.0 {
                        out.extend(self.cue(inp, "cadence", format!("Cadence {c:.0} rpm — spin up to {lo}–{hi}."), None, false));
                    } else if c > hi as f64 + 3.0 {
                        out.extend(self.cue(inp, "cadence", format!("Cadence {c:.0} rpm — settle back to {lo}–{hi}."), None, false));
                    }
                }
            }
        }
        // Well under an ERG target for 45 s: suggest easing (never harder).
        if inp.erg_active && !st.free_effort && !inp.low_cadence_active && st.elapsed_s >= 45.0 && self.ready("under_target", inp.active_s, 180.0) {
            let win: Vec<&Sample> = recent(45).collect();
            let with_power = win.iter().filter(|s| s.power.is_some() && s.target_w.is_some()).count();
            if win.len() >= 40 && with_power >= 36 {
                let p = mean(win.iter().filter_map(|s| s.power)).unwrap_or(0.0);
                let t = mean(win.iter().filter_map(|s| s.target_w)).unwrap_or(0.0);
                if t > 0.0 && p < 0.85 * t {
                    let short = ((1.0 - p / t) * 100.0).round();
                    let can_ease = w.adjust_pct > crate::workout_engine::MIN_ADJUST_PCT;
                    let text = if can_ease {
                        format!("You've been about {short:.0}% under target for 45 s. Ease the intensity 5%? A steady effort beats a fade.")
                    } else {
                        format!("You've been about {short:.0}% under target for 45 s. Intensity is already at its lowest; skip the interval or keep spinning easily.")
                    };
                    out.extend(self.cue(inp, "under_target", text, can_ease.then_some(CueAction::Intensity(-5)), false));
                }
            }
        }
        out
    }

    fn route_cues(&mut self, inp: &CueInput, r: &RouteRun) -> Vec<FeedItem> {
        let mut out = Vec::new();
        if r.finished {
            return out;
        }
        let prof = &r.profile;
        let lap = r.lap;
        // Top of an announced climb.
        if let Some(&(_, _, end)) = self.climbs.iter().find(|(l, a, e)| *l == lap && r.s >= *e && r.s - *e < 200.0 && *a < r.s && !self.topped.contains(&(lap, *e))) {
            self.topped.push((lap, end));
            out.extend(self.cue(inp, "climb_top", "Top of the climb. Shift down a gear and recover.".into(), None, false));
        }
        // A climb ahead: average ≥ 3% over at least 300 m starting 100–600 m ahead.
        if !self.ready("climb_ahead", inp.active_s, 30.0) {
            return out;
        }
        let g = |s: f64| prof.at(s).grade;
        let ele = |s: f64| prof.at(s).ele;
        let mut start = None;
        let mut x = r.s + 100.0;
        while x <= (r.s + 600.0).min(prof.total_m) {
            if g(x).map(|v| v >= 3.5).unwrap_or(false) {
                start = Some(x);
                break;
            }
            x += 10.0;
        }
        let Some(a) = start else { return out };
        if self.climbs.iter().any(|(l, s0, e0)| *l == lap && a >= *s0 - 50.0 && a <= *e0 + 50.0) {
            return out;
        }
        // Extend while the next 100 m still climbs on average.
        let mut b = a;
        while b < prof.total_m && b - a < 15_000.0 {
            let ahead = mean((0..10).filter_map(|k| g((b + 10.0 * k as f64).min(prof.total_m))));
            if ahead.map(|v| v < 2.0).unwrap_or(true) {
                break;
            }
            b += 20.0;
        }
        let len = b - a;
        let (Some(ea), Some(eb)) = (ele(a), ele(b)) else { return out };
        let avg = 100.0 * (eb - ea) / len.max(1.0);
        if len < 300.0 || avg < 3.0 {
            return out;
        }
        let steepest = (0..=(len / 20.0) as usize).filter_map(|k| g(a + 20.0 * k as f64)).fold(f64::MIN, f64::max);
        self.climbs.push((lap, a, b));
        let text = format!(
            "Climb in {}: {} at {:.1}% average{}.{}",
            fmt_dist(a - r.s, inp.imperial),
            fmt_dist(len, inp.imperial),
            avg,
            if steepest >= avg + 2.0 { format!(", up to {steepest:.0}%") } else { String::new() },
            if r.controls_resistance && r.difficulty < 0.999 { format!(" Trainer difficulty is {:.0}%.", r.difficulty * 100.0) } else { String::new() }
        );
        out.extend(self.cue(inp, "climb_ahead", text, None, len >= 800.0));
        out
    }

    pub fn feed_json(&self, last: usize) -> Value {
        Value::Arr(self.feed.iter().rev().take(last).rev().map(|f| f.to_json()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(fmt_dur(30.0), "30 s");
        assert_eq!(fmt_dur(300.0), "5 min");
        assert_eq!(fmt_dur(150.0), "2:30 min");
        assert_eq!(fmt_dist(420.0, false), "400 m");
        assert_eq!(fmt_dist(1240.0, false), "1.2 km");
        assert_eq!(fmt_dist(300.0, true), "1000 ft");
        assert_eq!(fmt_dist(2000.0, true), "1.2 mi");
    }

    #[test]
    fn feed_is_bounded_and_moments_supersede() {
        let mut c = RideCoach::default();
        for i in 0..100 {
            c.push(i as f64, "cue", "x", format!("{i}"), None, false);
        }
        assert_eq!(c.feed.len(), MAX_FEED);
        assert_eq!(c.feed.last().unwrap().id, 100);
        c.pending_moment = Some(Moment { kind: "a".into(), text: "1".into() });
        c.pending_moment = Some(Moment { kind: "b".into(), text: "2".into() });
        assert_eq!(c.take_moment().unwrap().kind, "b");
        assert!(c.take_moment().is_none());
    }
}
