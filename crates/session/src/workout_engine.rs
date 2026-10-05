//! Structured workout execution against the session's active (pause-free)
//! time. Targets come from the FTP snapshot captured when the session was
//! prepared; rider intensity adjustments are stored separately and bounded.

use rl_domain::workout::{Basis, TimelineStep, Workout};

pub const MIN_ADJUST_PCT: i32 = -20;
pub const MAX_ADJUST_PCT: i32 = 10;

#[derive(Debug, Clone)]
pub struct WorkoutRun {
    pub workout: Workout,
    pub timeline: Vec<TimelineStep>,
    pub total_s: f64,
    pub ftp_snapshot: Option<f64>,
    /// Execute by perceived effort only (no ERG targets).
    pub rpe_mode: bool,
    /// Position in the workout timeline (seconds). Advanced by active time;
    /// skip jumps it forward.
    pub pos_s: f64,
    pub adjust_pct: i32,
    pub complete: bool,
    pub skipped: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepState {
    pub index: usize,
    pub label: String,
    pub elapsed_s: f64,
    pub remaining_s: f64,
    pub target_w: Option<f64>,
    pub target_pct: Option<f64>,
    pub rpe: Option<u8>,
    pub cadence: Option<(u16, u16)>,
    pub text: Option<String>,
    /// True when the step is executed by feel (RPE) rather than ERG watts.
    pub free_effort: bool,
}

impl WorkoutRun {
    pub fn new(workout: Workout, ftp_snapshot: Option<f64>, rpe_mode: bool) -> WorkoutRun {
        let timeline = workout.timeline();
        let total_s = workout.total_s() as f64;
        WorkoutRun { workout, timeline, total_s, ftp_snapshot, rpe_mode, pos_s: 0.0, adjust_pct: 0, complete: false, skipped: vec![] }
    }

    pub fn advance(&mut self, dt_s: f64) {
        if self.complete || !(dt_s > 0.0) {
            return;
        }
        self.pos_s = (self.pos_s + dt_s).min(self.total_s);
        if self.pos_s >= self.total_s - 1e-6 {
            self.complete = true;
        }
    }

    pub fn current_index(&self) -> Option<usize> {
        if self.timeline.is_empty() {
            return None;
        }
        let p = self.pos_s.min(self.total_s - 1e-6);
        let i = self.timeline.partition_point(|s| (s.start_s as f64) <= p);
        Some(i.saturating_sub(1).min(self.timeline.len() - 1))
    }

    pub fn step_state(&self) -> Option<StepState> {
        let i = self.current_index()?;
        let s = &self.timeline[i];
        let elapsed = (self.pos_s - s.start_s as f64).clamp(0.0, s.dur_s as f64);
        let f = if s.dur_s > 0 { elapsed / s.dur_s as f64 } else { 0.0 };
        let factor = 1.0 + self.adjust_pct as f64 / 100.0;
        let free = s.target.basis == Basis::Rpe || self.rpe_mode;
        let target_w = if free { None } else { s.target.watts_at(f, self.ftp_snapshot).map(|w| (w * factor).max(0.0)) };
        let pct = s.target.pct_at(f, self.ftp_snapshot).map(|p| if s.target.basis == Basis::Rpe { p } else { p * factor });
        let rpe = match s.target.basis {
            Basis::Rpe => Some(s.target.value.round() as u8),
            _ => pct.map(rl_domain::workout::pct_to_rpe),
        };
        Some(StepState {
            index: i,
            label: s.label.clone(),
            elapsed_s: elapsed,
            remaining_s: (s.dur_s as f64 - elapsed).max(0.0),
            target_w,
            target_pct: pct,
            rpe,
            cadence: s.cadence.as_ref().map(|c| (c.lo, c.hi)),
            text: s.text.clone(),
            free_effort: free,
        })
    }

    pub fn next_step(&self) -> Option<&TimelineStep> {
        self.current_index().and_then(|i| self.timeline.get(i + 1))
    }

    /// Jump to the start of the next step. Returns the skipped index.
    pub fn skip(&mut self) -> Option<usize> {
        let i = self.current_index()?;
        self.skipped.push(i);
        match self.timeline.get(i + 1) {
            Some(n) => self.pos_s = n.start_s as f64,
            None => {
                self.pos_s = self.total_s;
                self.complete = true;
            }
        }
        Some(i)
    }

    pub fn adjust(&mut self, delta: i32) -> i32 {
        self.adjust_pct = (self.adjust_pct + delta).clamp(MIN_ADJUST_PCT, MAX_ADJUST_PCT);
        self.adjust_pct
    }

    pub fn remaining_s(&self) -> f64 {
        (self.total_s - self.pos_s).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_domain::library::{builtin_workouts, test_fixtures};

    #[test]
    fn steps_targets_skip_and_adjust() {
        let w = test_fixtures().into_iter().find(|w| w.id == "test-erg-short").unwrap();
        let mut r = WorkoutRun::new(w, Some(250.0), false);
        assert_eq!(r.step_state().unwrap().target_w, Some(100.0));
        r.advance(61.0);
        assert_eq!(r.step_state().unwrap().target_w, Some(150.0));
        assert_eq!(r.adjust(5), 5);
        assert_eq!(r.step_state().unwrap().target_w, Some(157.5));
        assert_eq!(r.adjust(50), MAX_ADJUST_PCT);
        r.skip();
        assert_eq!(r.step_state().unwrap().index, 2);
        r.advance(100.0);
        assert!(r.complete);
    }

    #[test]
    fn ramps_and_unknown_ftp() {
        let w = builtin_workouts().into_iter().find(|w| w.id == "endurance-45").unwrap();
        let mut r = WorkoutRun::new(w.clone(), Some(200.0), false);
        r.advance(300.0); // halfway through 600 s warm-up 45→65 %
        let t = r.step_state().unwrap().target_w.unwrap();
        assert!((t - 110.0).abs() < 0.5, "{t}");
        let r = WorkoutRun::new(w, None, false);
        let st = r.step_state().unwrap();
        assert_eq!(st.target_w, None, "no FTP: no watts invented");
        assert!(st.rpe.is_some(), "perceived-effort guidance still available");
    }
}
