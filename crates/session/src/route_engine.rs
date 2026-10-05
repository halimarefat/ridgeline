//! Free-ride route progression and grade-to-trainer command shaping.
//!
//! grade_percent   = processed road grade at s (+ optional bounded look-ahead)
//! requested_grade = grade_percent × trainer_difficulty
//! command_grade   = clamp_and_slew(requested_grade, device limits)
//!
//! Difficulty changes only the physical command; route elevation, climbing
//! totals and the virtual physics always use the real road grade.

use rl_domain::physics::BikeModel;
use rl_domain::route::RouteProfile;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct GradeLimits {
    pub min_pct: f64,
    pub max_pct: f64,
    /// Maximum change of the commanded grade per second.
    pub slew_pct_per_s: f64,
    /// Changes smaller than this are not sent.
    pub deadband_pct: f64,
    /// Resend the current grade at least this often (ms) while riding.
    pub keepalive_ms: u64,
}

impl Default for GradeLimits {
    fn default() -> Self {
        // Conservative untested defaults; tune per device from hardware tests.
        GradeLimits { min_pct: -10.0, max_pct: 20.0, slew_pct_per_s: 1.5, deadband_pct: 0.2, keepalive_ms: 10_000 }
    }
}

#[derive(Debug, Clone)]
pub struct GradeCommander {
    pub limits: GradeLimits,
    /// Slewed value (continuously updated).
    pub shaped: Option<f64>,
    pub last_sent: Option<f64>,
    last_sent_ms: u64,
    pub saturated: bool,
}

impl GradeCommander {
    pub fn new(limits: GradeLimits) -> Self {
        GradeCommander { limits, shaped: None, last_sent: None, last_sent_ms: 0, saturated: false }
    }

    /// Reset slew state, e.g. after pause or control loss; the next command
    /// ramps from `from` (0 % = an easy start).
    pub fn reset(&mut self, from: f64) {
        self.shaped = Some(from);
        self.last_sent = None;
    }

    /// Update with the requested grade; returns a command to send, if any.
    pub fn update(&mut self, requested: f64, dt_s: f64, now_ms: u64) -> Option<f64> {
        if !requested.is_finite() {
            return None;
        }
        let clamped = requested.clamp(self.limits.min_pct, self.limits.max_pct);
        self.saturated = (clamped - requested).abs() > 1e-9;
        let prev = self.shaped.unwrap_or(0.0);
        let max_step = self.limits.slew_pct_per_s * dt_s.max(0.0);
        let next = prev + (clamped - prev).clamp(-max_step, max_step);
        self.shaped = Some(next);
        let q = (next * 100.0).round() / 100.0;
        let due = match self.last_sent {
            None => true,
            Some(l) => (q - l).abs() >= self.limits.deadband_pct || now_ms.saturating_sub(self.last_sent_ms) >= self.limits.keepalive_ms,
        };
        if due {
            self.last_sent = Some(q);
            self.last_sent_ms = now_ms;
            Some(q)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progression {
    /// Measured power through the bicycle model.
    Power,
    /// Trainer-reported speed: approximate, labelled as such.
    TrainerSpeed,
    /// No usable input: coasting under the model.
    Coasting,
}

impl Progression {
    pub fn as_str(&self) -> &'static str {
        match self {
            Progression::Power => "power",
            Progression::TrainerSpeed => "trainer_speed_approx",
            Progression::Coasting => "coasting",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteRun {
    pub route_id: String,
    pub route_name: String,
    pub profile: Arc<RouteProfile>,
    pub model: BikeModel,
    pub s: f64,
    pub v: f64,
    pub difficulty: f64,
    pub lookahead_m: f64,
    pub commander: GradeCommander,
    pub finished: bool,
    pub lap: u32,
    pub progression: Progression,
    pub road_grade: Option<f64>,
    pub requested_grade: Option<f64>,
    pub flat_fallback_active: bool,
    /// Simulation control: false for a workout-with-map ride.
    pub controls_resistance: bool,
    pub segment: u32,
    pub ascent_m: f64,
    /// Distance ridden over all laps (m).
    pub total_distance_m: f64,
    last_ele: Option<f64>,
}

impl RouteRun {
    #[allow(clippy::too_many_arguments)]
    pub fn new(route_id: String, route_name: String, profile: Arc<RouteProfile>, model: BikeModel, difficulty_pct: f64, lookahead_m: f64, limits: GradeLimits, controls_resistance: bool) -> RouteRun {
        let start_ele = profile.at(0.0).ele;
        RouteRun {
            route_id,
            route_name,
            profile,
            model,
            s: 0.0,
            v: 0.0,
            difficulty: (difficulty_pct / 100.0).clamp(0.0, 1.0),
            lookahead_m: lookahead_m.clamp(0.0, 30.0),
            commander: GradeCommander::new(limits),
            finished: false,
            lap: 1,
            progression: Progression::Coasting,
            road_grade: None,
            requested_grade: None,
            flat_fallback_active: false,
            controls_resistance,
            segment: 0,
            ascent_m: 0.0,
            total_distance_m: 0.0,
            last_ele: start_ele,
        }
    }

    /// Advance the rider along the route. Returns a grade command to send
    /// (if this run controls resistance and a command is due).
    pub fn step(&mut self, dt_s: f64, power_w: Option<f64>, trainer_speed_kmh: Option<f64>, now_ms: u64) -> Option<f64> {
        let (g_here, fb) = self.profile.sim_grade(self.s);
        self.flat_fallback_active = fb;
        let physics_grade = g_here.unwrap_or(0.0);
        if !self.finished {
            match (power_w, trainer_speed_kmh) {
                (Some(p), _) => {
                    self.progression = Progression::Power;
                    self.v = self.model.step(self.v, p, physics_grade, dt_s);
                }
                (None, Some(kmh)) => {
                    self.progression = Progression::TrainerSpeed;
                    self.v = (kmh / 3.6).clamp(0.0, rl_domain::physics::V_MAX);
                }
                (None, None) => {
                    self.progression = Progression::Coasting;
                    self.v = self.model.step(self.v, 0.0, physics_grade, dt_s);
                }
            }
            let before = self.s;
            self.s += self.v * dt_s;
            if self.s >= self.profile.total_m {
                self.s = self.profile.total_m;
                self.v = 0.0;
                self.finished = true;
            }
            self.total_distance_m += self.s - before;
            let p = self.profile.at(self.s);
            self.segment = p.seg;
            if let (Some(e), Some(l)) = (p.ele, self.last_ele) {
                if e - l >= 0.5 {
                    self.ascent_m += e - l;
                    self.last_ele = Some(e);
                } else if l - e >= 0.5 {
                    self.last_ele = Some(e);
                }
            } else if p.ele.is_some() {
                self.last_ele = p.ele;
            }
        }
        let ahead = self.lookahead_m.min(self.v * 1.5);
        let (g, _) = self.profile.sim_grade(self.s + ahead);
        self.road_grade = self.profile.sim_grade(self.s).0;
        if self.finished {
            // Ease to flat at the finish.
            self.requested_grade = Some(0.0);
            return if self.controls_resistance { self.commander.update(0.0, dt_s, now_ms) } else { None };
        }
        match g {
            Some(g) => {
                let req = g * self.difficulty;
                self.requested_grade = Some(req);
                if self.controls_resistance {
                    self.commander.update(req, dt_s, now_ms)
                } else {
                    None
                }
            }
            None => {
                // Missing elevation without accepted fallback: hold the last
                // command (no new, invented gradient).
                self.requested_grade = None;
                None
            }
        }
    }

    pub fn new_lap(&mut self) {
        self.s = 0.0;
        self.v = 0.0;
        self.finished = false;
        self.lap += 1;
        self.last_ele = self.profile.at(0.0).ele;
    }

    pub fn commanded_grade(&self) -> Option<f64> {
        self.commander.last_sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_domain::geo::LatLon;
    use rl_domain::route::{process, synthetic, ElevationSource, ProfileConfig, RouteInput};

    pub fn a05() -> Arc<RouteProfile> {
        let pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(1000.0, 0.0), (1000.0, 5.0), (500.0, 0.0), (1000.0, -3.0), (500.0, 0.0)], false);
        let segs = vec![pts];
        Arc::new(process(&RouteInput { segments: &segs, source: ElevationSource::of("synthetic"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap())
    }

    #[test]
    fn a05_signed_commands_follow_route_position() {
        let mut r = RouteRun::new("r".into(), "A05".into(), a05(), BikeModel::new(84.0), 100.0, 0.0, GradeLimits::default(), true);
        let mut t = 0u64;
        let mut cmds: Vec<(f64, f64)> = Vec::new(); // (s, grade)
        while !r.finished && t < 3_600_000 {
            t += 250;
            if let Some(g) = r.step(0.25, Some(250.0), None, t) {
                cmds.push((r.s, g));
            }
        }
        assert!(r.finished);
        let at = |lo: f64, hi: f64| cmds.iter().filter(|(s, _)| *s > lo && *s < hi).map(|c| c.1).collect::<Vec<_>>();
        assert!(at(100.0, 900.0).iter().all(|g| g.abs() < 0.3), "flat section");
        let climb = at(1300.0, 1900.0);
        assert!(!climb.is_empty() && climb.iter().all(|g| (*g - 5.0).abs() < 0.3), "{climb:?}");
        let desc = at(2900.0, 3400.0);
        assert!(!desc.is_empty() && desc.iter().all(|g| (*g + 3.0).abs() < 0.3), "{desc:?}");
        // Bounded command count (deadband + keepalive), not an unbounded stream.
        assert!(cmds.len() < 400, "{}", cmds.len());
        // Climbing stats from the route, unaffected by difficulty.
        assert!((r.ascent_m - 50.0).abs() < 3.0, "{}", r.ascent_m);
    }

    #[test]
    fn difficulty_scales_command_not_road() {
        let mut r = RouteRun::new("r".into(), "A05".into(), a05(), BikeModel::new(84.0), 50.0, 0.0, GradeLimits::default(), true);
        r.s = 1500.0;
        r.v = 3.0;
        let mut last = None;
        for i in 0..40 {
            if let Some(g) = r.step(0.25, Some(250.0), None, i * 250) {
                last = Some(g);
            }
        }
        assert!((r.road_grade.unwrap() - 5.0).abs() < 0.2);
        assert!((last.unwrap() - 2.5).abs() < 0.3, "{last:?}");
    }

    #[test]
    fn clamp_and_slew() {
        let mut c = GradeCommander::new(GradeLimits::default());
        let first = c.update(25.0, 0.1, 0).unwrap();
        assert!(first.abs() < 0.2, "slew from 0: {first}");
        let mut last = first;
        for i in 1..400 {
            if let Some(g) = c.update(25.0, 0.1, i * 100) {
                assert!((g - last).abs() <= 1.5 + 0.21, "step too large");
                last = g;
            }
        }
        assert_eq!(last, 20.0, "clamped to max");
        assert!(c.saturated);
        assert!(c.update(f64::NAN, 0.1, 50_000).is_none());
    }

    #[test]
    fn workout_map_does_not_control_resistance() {
        let mut r = RouteRun::new("r".into(), "A05".into(), a05(), BikeModel::new(84.0), 100.0, 0.0, GradeLimits::default(), false);
        for i in 0..100 {
            assert!(r.step(0.25, Some(200.0), None, i * 250).is_none());
        }
        assert!(r.s > 0.0, "map still advances");
    }

    #[test]
    fn coasting_continues_downhill() {
        let mut r = RouteRun::new("r".into(), "A05".into(), a05(), BikeModel::new(84.0), 100.0, 0.0, GradeLimits::default(), true);
        r.s = 2600.0; // on the -3 % descent
        r.v = 8.0;
        let s0 = r.s;
        for i in 0..20 {
            r.step(0.25, Some(0.0), None, i * 250);
        }
        assert!(r.s > s0 + 30.0, "coasting downhill continues with zero power");
    }
}
