//! CSV export of 1 Hz records. Units are in the header; missing values are
//! empty cells (never zero).

use rl_domain::time::iso_utc;
use rl_session::record::Sample;

pub const HEADER: &str = "timestamp_utc,elapsed_s,timer_s,power_w,cadence_rpm,heart_rate_bpm,speed_mps,distance_m,elevation_m,grade_pct,commanded_grade_pct,target_power_w,trainer_speed_kmh,latitude_deg,longitude_deg,workout_step,power_source,cadence_source,heart_rate_source,flags";

fn num(v: Option<f64>, digits: usize) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{x:.digits$}"),
        _ => String::new(),
    }
}

fn text(v: &Option<String>) -> String {
    match v {
        None => String::new(),
        Some(s) => {
            let clean: String = s.chars().filter(|c| !c.is_control()).collect();
            // Neutralise spreadsheet formula injection and quote.
            let clean = if clean.starts_with(['=', '+', '-', '@']) { format!("'{clean}") } else { clean };
            format!("\"{}\"", clean.replace('"', "\"\""))
        }
    }
}

pub fn samples_to_csv(samples: &[Sample]) -> String {
    let mut out = String::with_capacity(samples.len() * 120 + HEADER.len() + 2);
    out.push_str(HEADER);
    out.push('\n');
    for s in samples {
        let row = [
            iso_utc(s.utc_ms),
            format!("{:.3}", s.t_ms as f64 / 1000.0),
            s.active_s.to_string(),
            num(s.power, 0),
            num(s.cadence, 1),
            num(s.hr, 0),
            num(s.speed, 3),
            num(s.distance, 2),
            num(s.ele, 2),
            num(s.grade, 2),
            num(s.cmd_grade, 2),
            num(s.target_w, 0),
            num(s.trainer_speed, 2),
            num(s.lat, 7),
            num(s.lon, 7),
            s.step.map(|x| x.to_string()).unwrap_or_default(),
            text(&s.power_src),
            text(&s.cadence_src),
            text(&s.hr_src),
            s.flags.to_string(),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_values_are_empty_and_units_in_header() {
        let s = vec![Sample { t_ms: 1000, utc_ms: 0, active_s: 1, power: Some(0.0), hr: None, power_src: Some("=cmd".into()), ..Default::default() }];
        let c = samples_to_csv(&s);
        let lines: Vec<&str> = c.lines().collect();
        assert!(lines[0].contains("power_w") && lines[0].contains("speed_mps"));
        let cells: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(cells[3], "0", "measured zero kept");
        assert_eq!(cells[5], "", "missing heart rate is empty");
        assert!(lines[1].contains("\"'=cmd\""));
    }
}
