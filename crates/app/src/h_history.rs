//! Activity history: list, detail, export (FIT/CSV/GPX), delete, recovery.

use crate::app::{pstr, R};
use crate::App;
use rl_domain::gpx::{to_gpx, RawPoint};
use rl_domain::rider::{ftp_from_ramp, ftp_from_twenty_minute, FtpMethod};
use rl_json::{ToJson, Value};
use rl_storage::store::LoadedActivity;

pub fn file_stem(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let t = t.trim_matches('-').to_string();
    let t: String = t.split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    if t.is_empty() {
        "ridgeline".into()
    } else {
        t.chars().take(60).collect()
    }
}

/// Best rolling average of `w` seconds; requires ≥ 90 % power coverage.
fn best_avg(samples: &[rl_session::record::Sample], w: usize) -> Option<f64> {
    if samples.len() < w {
        return None;
    }
    let mut best: Option<f64> = None;
    for win in samples.windows(w) {
        let vals: Vec<f64> = win.iter().filter_map(|s| s.power).collect();
        if vals.len() * 10 < w * 9 {
            continue;
        }
        let avg = vals.iter().sum::<f64>() / vals.len() as f64;
        best = Some(best.map_or(avg, |b: f64| b.max(avg)));
    }
    best
}

/// Provisional FTP estimate for completed optional assessments.
pub fn ftp_estimate(a: &LoadedActivity) -> Option<(f64, FtpMethod)> {
    match a.meta.workout_id.as_deref()? {
        "assessment-ramp" => best_avg(&a.samples, 60).map(|b| (ftp_from_ramp(b), FtpMethod::RampTest)),
        "assessment-20min" => best_avg(&a.samples, 1200).map(|b| (ftp_from_twenty_minute(b), FtpMethod::TwentyMinuteTest)),
        _ => None,
    }
    .filter(|(w, _)| (40.0..=600.0).contains(w))
}

impl App {
    pub(crate) fn write_export(&self, name: &str, bytes: &[u8]) -> Result<String, String> {
        let dir = self.cfg.export_dir.clone();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create the export folder: {e}"))?;
        let mut path = dir.join(name);
        let mut n = 1;
        while path.exists() && n < 1000 {
            let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
            path = dir.join(format!("{stem}-{n}.{ext}"));
            n += 1;
        }
        rl_storage::fsx::atomic_write(&path, bytes)?;
        Ok(path.display().to_string())
    }

    pub(crate) fn list_activities(&mut self, _p: &Value) -> R {
        let rows: Vec<Value> = self
            .store
            .list_activities()?
            .into_iter()
            .map(|(m, s)| {
                let fb = self.store.read_value(&format!("activities/{}/feedback.json", m.id)).ok().flatten();
                Value::obj([("meta", m.to_json()), ("summary", s.map(|s| s.to_json()).unwrap_or(Value::Null)), ("feedback", fb.unwrap_or(Value::Null))])
            })
            .collect();
        Ok(Value::Arr(rows))
    }

    pub(crate) fn get_activity(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let a = self.store.load_activity(id)?.ok_or("Unknown activity.")?;
        let n = a.samples.len();
        let stride = n.div_ceil(1500).max(1);
        let series: Vec<Value> = a
            .samples
            .iter()
            .enumerate()
            .filter(|(i, _)| i % stride == 0 || *i == n - 1)
            .map(|(_, s)| Value::Arr(vec![s.active_s.into(), s.power.into(), s.hr.into(), s.cadence.into(), s.speed.into(), s.ele.into(), s.target_w.into(), s.grade.into(), s.flags.into()]))
            .collect();
        let line: Vec<Value> = a
            .samples
            .iter()
            .enumerate()
            .filter(|(i, _)| i % stride == 0)
            .filter_map(|(_, s)| Some(Value::Arr(vec![s.lon?.into(), s.lat?.into()])))
            .collect();
        let est = ftp_estimate(&a).map(|(w, m)| Value::obj([("watts", w.into()), ("method", m.to_json())])).unwrap_or(Value::Null);
        Ok(Value::obj([
            ("meta", a.meta.to_json()),
            ("summary", a.summary.map(|s| s.to_json()).unwrap_or(Value::Null)),
            ("laps", a.laps.to_json()),
            ("events", a.events.to_json()),
            ("series", Value::Arr(series)),
            ("series_fields", Value::Arr(["t", "power", "hr", "cadence", "speed", "ele", "target_w", "grade", "flags"].iter().map(|s| Value::from(*s)).collect())),
            ("line", Value::Arr(line)),
            ("feedback", a.feedback.unwrap_or(Value::Null)),
            ("corrupt_lines", a.corrupt_lines.into()),
            ("ftp_estimate", est),
        ]))
    }

    pub(crate) fn export_activity(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let fmt = pstr(p, "format", 8)?;
        let a = self.store.load_activity(id)?.ok_or("Unknown activity.")?;
        if a.meta.status == "recording" {
            return Err("This ride is still recording or needs recovery first.".into());
        }
        let date = rl_domain::time::iso_utc(a.meta.start_utc)[..10].to_string();
        let stem = format!("{}-{}", date, file_stem(&a.meta.title));
        let path = match fmt {
            "fit" => {
                let summary = a.summary.clone().ok_or("This ride has no summary.")?;
                let bytes = rl_storage::fit::encode_activity(&rl_storage::fit::FitInput {
                    start_utc_ms: a.meta.start_utc,
                    tz_offset_min: a.meta.tz_offset_min,
                    virtual_route: a.meta.virtual_route,
                    samples: &a.samples,
                    laps: &a.laps,
                    events: &a.events,
                    summary: &summary,
                });
                self.write_export(&format!("{stem}.fit"), &bytes)?
            }
            "csv" => self.write_export(&format!("{stem}.csv"), rl_storage::csv::samples_to_csv(&a.samples).as_bytes())?,
            "gpx" => {
                let pts: Vec<RawPoint> = a.samples.iter().filter_map(|s| Some(RawPoint { lat: s.lat?, lon: s.lon?, ele: s.ele })).collect();
                if pts.len() < 2 {
                    return Err("This ride has no route geometry (GPX export covers route geometry only).".into());
                }
                self.write_export(&format!("{stem}.gpx"), to_gpx(&a.meta.title, &[pts]).as_bytes())?
            }
            _ => return Err("Unknown export format.".into()),
        };
        Ok(Value::obj([("path", path.into())]))
    }

    pub(crate) fn delete_activity(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        if self.session_meta.as_ref().map(|m| m.id == id).unwrap_or(false) && self.ride_active() {
            return Err("This ride is in progress.".into());
        }
        self.store.delete_activity(id)?;
        // Remove plan completion links.
        if let Value::Obj(o) = &mut self.completions {
            o.retain(|(_, v)| v.as_str() != Some(id));
        }
        self.store.write_value("plans/completions.json", &self.completions)?;
        Ok(Value::Null)
    }

    pub(crate) fn recover_activity(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        if self.session_meta.as_ref().map(|m| m.id == id).unwrap_or(false) {
            return Err("This ride is the current session.".into());
        }
        let (meta, summary, bad) = self.store.recover_activity(id)?;
        Ok(Value::obj([("meta", meta.to_json()), ("summary", summary.to_json()), ("corrupt_lines", bad.into())]))
    }

    pub(crate) fn discard_recovery(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let m = self.store.activity_meta(id)?.ok_or("Unknown activity.")?;
        if m.status != "recording" || self.session_meta.as_ref().map(|s| s.id == id).unwrap_or(false) {
            return Err("Only interrupted rides can be discarded here.".into());
        }
        self.store.delete_activity(id)?;
        Ok(Value::Null)
    }

    pub(crate) fn rename_activity(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let title: String = pstr(p, "title", 120)?.chars().filter(|c| !c.is_control()).collect();
        let mut m = self.store.activity_meta(id)?.ok_or("Unknown activity.")?;
        if m.demo && !title.starts_with("[Demo]") {
            m.title = format!("[Demo] {title}");
        } else {
            m.title = title;
        }
        self.store.write_doc(&format!("activities/{id}/meta.json"), &m)?;
        Ok(Value::Null)
    }
}
