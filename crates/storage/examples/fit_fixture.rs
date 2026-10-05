//! Writes a synthetic FIT + CSV export pair used by scripts/verify-fit.mjs
//! (independent decoding with the Garmin FIT SDK).
//! Usage: cargo run -p rl-storage --example fit_fixture -- <out-dir>

use rl_session::record::{summarize, Lap, Sample, SessionEvent};
use rl_storage::fit::{encode_activity, FitInput};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "fixtures/fit".into());
    std::fs::create_dir_all(&out).unwrap();
    let start = 1_790_000_000_000i64; // 2026-09-21T...Z
    let mut samples = Vec::new();
    let mut t_ms = 0u64;
    let mut active = 0u32;
    // 300 s riding, 60 s pause, 300 s riding.
    for i in 0..600u32 {
        if i == 300 {
            t_ms += 60_000;
        }
        t_ms += 1000;
        active += 1;
        let hr = if i < 30 { None } else { Some(120.0 + (i % 40) as f64) };
        samples.push(Sample {
            t_ms,
            utc_ms: start + t_ms as i64,
            active_s: active,
            power: if (200..205).contains(&i) { None } else { Some(180.0 + (i % 20) as f64) },
            cadence: Some(88.0),
            hr,
            speed: Some(8.5),
            distance: Some(active as f64 * 8.5),
            ele: Some(50.0 + (i as f64 * 0.05)),
            lat: Some(47.56 + i as f64 * 1e-5),
            lon: Some(-52.71),
            grade: Some(2.5),
            ..Default::default()
        });
    }
    let laps = vec![
        Lap { index: 0, start_t_ms: 0, end_t_ms: 300_000, start_active_s: 0, end_active_s: 300, start_utc_ms: start, label: "Lap 1".into(), trigger: "manual".into(), step: None, target_w: None },
        Lap { index: 1, start_t_ms: 300_000, end_t_ms: 660_000, start_active_s: 300, end_active_s: 600, start_utc_ms: start + 300_000, label: "Lap 2".into(), trigger: "session_end".into(), step: None, target_w: None },
    ];
    let events = vec![
        SessionEvent { t_ms: 300_000, utc_ms: start + 300_500, kind: "pause".into(), detail: String::new() },
        SessionEvent { t_ms: 360_000, utc_ms: start + 360_500, kind: "resume".into(), detail: String::new() },
    ];
    let summary = summarize(&samples, &laps, 660.0, Some(250.0));
    let fit = encode_activity(&FitInput { start_utc_ms: start, tz_offset_min: -150, virtual_route: true, samples: &samples, laps: &laps, events: &events, summary: &summary });
    std::fs::write(format!("{out}/synthetic-ride.fit"), &fit).unwrap();
    std::fs::write(format!("{out}/synthetic-ride.csv"), rl_storage::csv::samples_to_csv(&samples)).unwrap();
    let expect = format!(
        "{{\"records\":{},\"laps\":{},\"timer_s\":{},\"elapsed_s\":{},\"avg_power\":{},\"missing_hr_records\":30,\"sub_sport\":\"virtualActivity\"}}",
        samples.len(),
        laps.len(),
        summary.timer_s,
        summary.elapsed_s,
        summary.power.avg.unwrap().round()
    );
    std::fs::write(format!("{out}/synthetic-ride.expect.json"), expect).unwrap();
    println!("wrote {} bytes", fit.len());
}
