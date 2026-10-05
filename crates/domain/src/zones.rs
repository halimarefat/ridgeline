//! Power zones as % of FTP (seven-zone model popularised by Allen & Coggan;
//! see docs/coaching-policy.md). Used for time-in-zone summaries only.

pub const POWER_ZONES: [(&str, f64, f64); 7] = [
    ("Z1 Active recovery", 0.0, 55.0),
    ("Z2 Endurance", 55.0, 75.0),
    ("Z3 Tempo", 75.0, 90.0),
    ("Z4 Threshold", 90.0, 105.0),
    ("Z5 VO2max", 105.0, 120.0),
    ("Z6 Anaerobic", 120.0, 150.0),
    ("Z7 Neuromuscular", 150.0, f64::INFINITY),
];

pub fn zone_index(watts: f64, ftp: f64) -> Option<usize> {
    if !(ftp > 0.0) || !watts.is_finite() || watts < 0.0 {
        return None;
    }
    let pct = 100.0 * watts / ftp;
    POWER_ZONES.iter().position(|(_, lo, hi)| pct >= *lo && pct < *hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zones() {
        assert_eq!(zone_index(100.0, 200.0), Some(0));
        assert_eq!(zone_index(200.0, 200.0), Some(3));
        assert_eq!(zone_index(400.0, 200.0), Some(6));
        assert_eq!(zone_index(100.0, 0.0), None);
    }
}
