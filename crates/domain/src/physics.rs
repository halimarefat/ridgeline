//! Deterministic bicycle model for virtual route progression.
//!
//! Forces (SI): gravity m·g·sinθ, rolling resistance Crr·m·g·cosθ (opposes
//! motion only), aerodynamic drag ½·ρ·CdA·(v + v_wind)², drivetrain
//! efficiency η on rider power. Integration uses a kinetic-energy update at
//! normal speeds and a bounded-force update below `V_LOW` so that power/speed
//! is never evaluated at zero. Speed is clamped to [0, V_MAX].
//!
//! Defaults (documented in docs/architecture.md): CdA 0.32 m² (hoods),
//! Crr 0.004 (road tyre on smooth tarmac), ρ 1.225 kg/m³ (sea level, 15 °C),
//! η 0.976, rotating-mass allowance 0.7 kg.

pub const G: f64 = 9.80665;
pub const V_LOW: f64 = 1.0;
pub const V_MAX: f64 = 30.0;

#[derive(Debug, Clone, PartialEq)]
pub struct BikeModel {
    pub mass_kg: f64,
    pub cda_m2: f64,
    pub crr: f64,
    pub rho: f64,
    pub efficiency: f64,
    pub rotating_mass_kg: f64,
    /// Headwind positive, m/s.
    pub wind_mps: f64,
}

impl BikeModel {
    pub fn new(system_mass_kg: f64) -> BikeModel {
        BikeModel { mass_kg: system_mass_kg.clamp(30.0, 300.0), cda_m2: 0.32, crr: 0.004, rho: 1.225, efficiency: 0.976, rotating_mass_kg: 0.7, wind_mps: 0.0 }
    }

    /// Equivalent FTMS wind-resistance coefficient ½·ρ·CdA in kg/m.
    pub fn cw_kg_per_m(&self) -> f64 {
        0.5 * self.rho * self.cda_m2
    }

    fn resist_force(&self, v: f64, grade_pct: f64) -> (f64, f64) {
        let theta = (grade_pct / 100.0).atan();
        let grav = self.mass_kg * G * theta.sin();
        let roll = self.crr * self.mass_kg * G * theta.cos();
        let va = v + self.wind_mps;
        let aero = 0.5 * self.rho * self.cda_m2 * va * va.abs();
        (grav + aero, roll)
    }

    /// Advance speed by `dt` seconds with rider power `power_w` (≥ 0) on a
    /// road of `grade_pct`. Sub-steps keep the update stable for any dt.
    pub fn step(&self, v: f64, power_w: f64, grade_pct: f64, dt: f64) -> f64 {
        let mut v = if v.is_finite() { v.clamp(0.0, V_MAX) } else { 0.0 };
        let p = if power_w.is_finite() { power_w.clamp(0.0, 3000.0) } else { 0.0 } * self.efficiency;
        let g = if grade_pct.is_finite() { grade_pct.clamp(-40.0, 40.0) } else { 0.0 };
        if !(dt.is_finite() && dt > 0.0) {
            return v;
        }
        let m_eff = self.mass_kg + self.rotating_mass_kg;
        let n = (dt / 0.05).ceil().clamp(1.0, 2000.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            let (other, roll) = self.resist_force(v, g);
            if v < V_LOW {
                // Low-speed regime: bounded propulsive force P / V_LOW.
                let drive = p / V_LOW;
                let net_wo_roll = drive - other;
                let a = if v <= 1e-9 {
                    // At rest, rolling resistance can hold the bike (static).
                    if net_wo_roll.abs() <= roll {
                        0.0
                    } else {
                        (net_wo_roll - roll * net_wo_roll.signum()) / m_eff
                    }
                } else {
                    (net_wo_roll - roll) / m_eff
                };
                v = (v + a * h).max(0.0);
            } else {
                let e = 0.5 * m_eff * v * v;
                let de = (p - (other + roll) * v) * h;
                let e2 = (e + de).max(0.0);
                v = (2.0 * e2 / m_eff).sqrt();
            }
            v = v.min(V_MAX);
        }
        v
    }

    /// Steady-state speed for constant power and grade (bisection), for tests
    /// and estimates.
    pub fn steady_speed(&self, power_w: f64, grade_pct: f64) -> f64 {
        let f = |v: f64| {
            let (o, r) = self.resist_force(v, grade_pct);
            power_w * self.efficiency - (o + r) * v
        };
        let (mut lo, mut hi) = (0.0, V_MAX);
        if f(hi) > 0.0 {
            return V_MAX;
        }
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if f(mid) > 0.0 {
                lo = mid
            } else {
                hi = mid
            }
        }
        0.5 * (lo + hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &BikeModel, p: f64, g: f64, secs: f64) -> f64 {
        let mut v = 0.0;
        let mut t = 0.0;
        while t < secs {
            v = m.step(v, p, g, 0.25);
            t += 0.25;
        }
        v
    }

    #[test]
    fn converges_to_plausible_steady_speeds() {
        let m = BikeModel::new(84.0);
        // 200 W on the flat: ~32–35 km/h for these defaults.
        let v = run(&m, 200.0, 0.0, 120.0);
        assert!((v * 3.6 - 33.5).abs() < 2.5, "{} km/h", v * 3.6);
        assert!((v - m.steady_speed(200.0, 0.0)).abs() < 0.05);
        // 250 W up 8%: ~11–13 km/h.
        let v = run(&m, 250.0, 8.0, 120.0);
        assert!((9.5..14.0).contains(&(v * 3.6)), "{} km/h", v * 3.6);
    }

    #[test]
    fn no_division_by_zero_and_bounded() {
        let m = BikeModel::new(84.0);
        assert_eq!(m.step(0.0, 0.0, 0.0, 1.0), 0.0);
        assert_eq!(m.step(0.0, 0.0, 5.0, 1.0), 0.0, "held at rest uphill (no rolling backwards)");
        assert!(m.step(0.0, 300.0, 0.0, 0.1) > 0.0, "starts from rest");
        assert!(m.step(f64::NAN, f64::INFINITY, f64::NAN, 1.0).is_finite());
        // Coasting downhill at -6 % accelerates from rest and stays bounded.
        let v = run(&m, 0.0, -6.0, 600.0);
        assert!(v > 10.0 && v <= V_MAX, "{v}");
        // Coasting on the flat decays towards zero but never below.
        let mut v = 10.0;
        for _ in 0..4000 {
            v = m.step(v, 0.0, 0.0, 0.25);
            assert!(v >= 0.0);
        }
        assert!(v < 1.0, "{v}");
        // Large dt is sub-stepped.
        let v1 = m.step(5.0, 200.0, 0.0, 10.0);
        let mut v2 = 5.0;
        for _ in 0..200 {
            v2 = m.step(v2, 200.0, 0.0, 0.05);
        }
        assert!((v1 - v2).abs() < 1e-6);
    }
}
