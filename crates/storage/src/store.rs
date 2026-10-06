//! Local-first document store.
//!
//! Layout (under the platform app-data directory):
//!
//! ```text
//! store.json                       store schema version
//! profile.json  settings.json  ftp.json  devices.json  onboarding_draft.json
//! workouts/<id>.json               custom workouts (+ .versions/<id>.<v>.json)
//! plans/current.json               {plan_id, version}
//! plans/<plan_id>/v<version>.json  every accepted/proposed plan version
//! plans/proposals/<id>.json        coach proposals (validated)
//! plans/log.jsonl                  accept / reject / undo history
//! routes/<id>/route.json           raw geometry, provenance, corrections
//! routes/<id>/profile.json         processed profile (+ processing version)
//! activities/<id>/meta.json        snapshots, status
//! activities/<id>/{samples,events,laps}.jsonl   journal
//! activities/<id>/summary.json  feedback.json
//! coach/chat.jsonl                 private coach conversation
//! ```
//!
//! The store is not encrypted; it relies on the operating-system account's
//! file permissions (see docs/privacy.md).

use crate::fsx::{self, atomic_write, read_string, safe_id};
use crate::journal::{read_jsonl, ActivityJournal};
use rl_domain::gpx::RawPoint;
use rl_domain::plan::TrainingPlan;
use rl_domain::rider::{FtpEntry, RiderProfile};
use rl_domain::route::{Correction, ElevationSource, RouteProfile, Sample as RSample, Flag, ProfileConfig};
use rl_domain::workout::Workout;
use rl_json::{json_struct, FromJson, JsonError, ToJson, Value};
use rl_session::record::{summarize, Lap, Sample, SessionEvent, Summary};
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityMeta {
    pub schema: u32,
    pub id: String,
    /// "recording", "complete", "recovered"
    pub status: String,
    pub demo: bool,
    pub mode: String,
    pub title: String,
    pub start_utc: i64,
    pub end_utc: Option<i64>,
    pub tz_name: String,
    pub tz_offset_min: i32,
    pub workout_id: Option<String>,
    pub workout_name: Option<String>,
    pub workout: Option<Value>,
    pub route_id: Option<String>,
    pub route_name: Option<String>,
    pub ftp_w: Option<f64>,
    pub plan_session_id: Option<String>,
    pub virtual_route: bool,
    pub app_version: String,
    pub created_utc: i64,
}
json_struct!(ActivityMeta {
    schema: "schema" = 1,
    id: "id",
    status: "status",
    demo: "demo" = false,
    mode: "mode",
    title: "title" = String::new(),
    start_utc: "start_utc",
    end_utc: "end_utc",
    tz_name: "tz_name" = String::new(),
    tz_offset_min: "tz_offset_min" = 0,
    workout_id: "workout_id",
    workout_name: "workout_name",
    workout: "workout",
    route_id: "route_id",
    route_name: "route_name",
    ftp_w: "ftp_w",
    plan_session_id: "plan_session_id",
    virtual_route: "virtual_route" = false,
    app_version: "app_version" = String::new(),
    created_utc: "created_utc" = 0,
});

#[derive(Debug, Clone, PartialEq)]
pub struct RouteRecord {
    pub schema: u32,
    pub id: String,
    pub name: String,
    /// "gpx", "built", "synthetic", "reversed"
    pub kind: String,
    /// Untrusted text from the file (displayed as data only).
    pub description: String,
    pub created_utc: i64,
    pub segments: Vec<Vec<RawPoint>>,
    pub elevation: ElevationSource,
    /// Terrain-model samples (raw, as fetched) when elevation came from a DEM.
    pub dem_segments: Option<Vec<Vec<RawPoint>>>,
    pub corrections: Vec<Correction>,
    pub flat_fallback: bool,
    pub waypoints: Vec<(f64, f64)>,
    pub routing_provider: String,
    pub attribution: String,
    pub reversed_from: Option<String>,
    pub bundled: bool,
}

fn pts_to_json(segs: &[Vec<RawPoint>]) -> Value {
    Value::Arr(
        segs.iter()
            .map(|s| Value::Arr(s.iter().map(|p| Value::Arr(vec![((p.lat * 1e7).round() / 1e7).into(), ((p.lon * 1e7).round() / 1e7).into(), p.ele.map(|e| (e * 100.0).round() / 100.0).into()])).collect()))
            .collect(),
    )
}

fn pts_from_json(v: &Value) -> Result<Vec<Vec<RawPoint>>, JsonError> {
    let segs = v.as_arr().ok_or_else(|| JsonError("segments must be an array".into()))?;
    segs.iter()
        .map(|s| {
            s.as_arr()
                .ok_or_else(|| JsonError("segment must be an array".into()))?
                .iter()
                .map(|p| {
                    let a = p.as_arr().ok_or_else(|| JsonError("point must be an array".into()))?;
                    let lat = a.first().and_then(|x| x.as_f64()).ok_or_else(|| JsonError("bad lat".into()))?;
                    let lon = a.get(1).and_then(|x| x.as_f64()).ok_or_else(|| JsonError("bad lon".into()))?;
                    if !rl_domain::geo::valid_lat_lon(lat, lon) {
                        return Err(JsonError("coordinate out of range".into()));
                    }
                    Ok(RawPoint { lat, lon, ele: a.get(2).and_then(|x| x.as_f64()) })
                })
                .collect()
        })
        .collect()
}

impl ToJson for RouteRecord {
    fn to_json(&self) -> Value {
        Value::obj([
            ("schema", self.schema.into()),
            ("id", self.id.clone().into()),
            ("name", self.name.clone().into()),
            ("kind", self.kind.clone().into()),
            ("description", self.description.clone().into()),
            ("created_utc", self.created_utc.into()),
            ("segments", pts_to_json(&self.segments)),
            ("elevation", self.elevation.to_json()),
            ("dem_segments", self.dem_segments.as_ref().map(|d| pts_to_json(d)).unwrap_or(Value::Null)),
            ("corrections", self.corrections.to_json()),
            ("flat_fallback", self.flat_fallback.into()),
            ("waypoints", Value::Arr(self.waypoints.iter().map(|(a, b)| Value::Arr(vec![(*a).into(), (*b).into()])).collect())),
            ("routing_provider", self.routing_provider.clone().into()),
            ("attribution", self.attribution.clone().into()),
            ("reversed_from", self.reversed_from.clone().into()),
            ("bundled", self.bundled.into()),
        ])
    }
}

impl FromJson for RouteRecord {
    fn from_json(v: &Value) -> Result<Self, JsonError> {
        Ok(RouteRecord {
            schema: rl_json::field_or(v, "schema", || 1)?,
            id: rl_json::field(v, "id")?,
            name: rl_json::field(v, "name")?,
            kind: rl_json::field(v, "kind")?,
            description: rl_json::field_or(v, "description", String::new)?,
            created_utc: rl_json::field_or(v, "created_utc", || 0)?,
            segments: pts_from_json(v.req("segments")?)?,
            elevation: rl_json::field(v, "elevation")?,
            dem_segments: match v.get("dem_segments") {
                None | Some(Value::Null) => None,
                Some(d) => Some(pts_from_json(d)?),
            },
            corrections: rl_json::field_or(v, "corrections", Vec::new)?,
            flat_fallback: rl_json::field_or(v, "flat_fallback", || false)?,
            waypoints: rl_json::field_or(v, "waypoints", Vec::new)?,
            routing_provider: rl_json::field_or(v, "routing_provider", String::new)?,
            attribution: rl_json::field_or(v, "attribution", String::new)?,
            reversed_from: rl_json::field(v, "reversed_from")?,
            bundled: rl_json::field_or(v, "bundled", || false)?,
        })
    }
}

/// Compact, versioned serialization of a processed profile.
pub fn profile_to_json(p: &RouteProfile) -> Value {
    let mut v = p.summary_json();
    v.set(
        "samples",
        Value::Arr(
            p.samples
                .iter()
                .map(|s| {
                    Value::Arr(vec![
                        ((s.s * 100.0).round() / 100.0).into(),
                        ((s.lat * 1e7).round() / 1e7).into(),
                        ((s.lon * 1e7).round() / 1e7).into(),
                        s.ele.map(|e| (e * 1000.0).round() / 1000.0).into(),
                        s.grade.map(|g| (g * 10000.0).round() / 10000.0).into(),
                        s.seg.into(),
                    ])
                })
                .collect(),
        ),
    );
    v
}

pub fn profile_from_json(v: &Value) -> Result<RouteProfile, JsonError> {
    let samples = v
        .req_arr("samples")?
        .iter()
        .map(|a| {
            let a = a.as_arr().ok_or_else(|| JsonError("bad sample".into()))?;
            let f = |i: usize| a.get(i).and_then(|x| x.as_f64());
            Ok(RSample { s: f(0).ok_or_else(|| JsonError("bad s".into()))?, lat: f(1).unwrap_or(0.0), lon: f(2).unwrap_or(0.0), ele: f(3), grade: f(4), seg: f(5).unwrap_or(0.0) as u32 })
        })
        .collect::<Result<Vec<_>, JsonError>>()?;
    let hist = v
        .req_arr("histogram")?
        .iter()
        .filter_map(|h| h.as_arr().and_then(|a| Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_f64()?))))
        .collect();
    Ok(RouteProfile {
        version: rl_json::field(v, "version")?,
        config: rl_json::field_or(v, "config", ProfileConfig::default)?,
        total_m: rl_json::field(v, "total_m")?,
        samples,
        segment_starts_m: rl_json::field(v, "segment_starts_m")?,
        ascent_m: rl_json::field(v, "ascent_m")?,
        descent_m: rl_json::field(v, "descent_m")?,
        max_grade: rl_json::field(v, "max_grade")?,
        min_grade: rl_json::field(v, "min_grade")?,
        ele_min: rl_json::field(v, "ele_min")?,
        ele_max: rl_json::field(v, "ele_max")?,
        histogram: hist,
        source: rl_json::field(v, "source")?,
        coverage_pct: rl_json::field(v, "coverage_pct")?,
        spikes_removed: rl_json::field(v, "spikes_removed")?,
        duplicates_removed: rl_json::field(v, "duplicates_removed")?,
        flags: rl_json::field::<Vec<Flag>>(v, "flags")?,
        flat_fallback: rl_json::field(v, "flat_fallback")?,
        simulation_ready: rl_json::field(v, "simulation_ready")?,
        blocked_reason: rl_json::field(v, "blocked_reason")?,
    })
}

pub struct LoadedActivity {
    pub meta: ActivityMeta,
    pub samples: Vec<Sample>,
    pub laps: Vec<Lap>,
    pub events: Vec<SessionEvent>,
    pub summary: Option<Summary>,
    pub feedback: Option<Value>,
    pub corrupt_lines: usize,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Store, String> {
        fs::create_dir_all(root).map_err(|e| fsx::io_msg("create data folder", root, &e))?;
        let s = Store { root: root.to_path_buf() };
        match s.read_value("store.json")? {
            None => s.write_value("store.json", &Value::obj([("store_version", STORE_VERSION.into()), ("created_utc", rl_domain::time::now_utc_ms().into())]))?,
            Some(v) => {
                let ver = v.get("store_version").and_then(|x| x.as_i64()).unwrap_or(0);
                if ver > STORE_VERSION as i64 {
                    return Err(format!("This data was written by a newer Ridgeline (store version {ver}). Update the app to open it; your data has not been changed."));
                }
                if ver < STORE_VERSION as i64 {
                    s.migrate(ver as u32)?;
                }
            }
        }
        for d in ["workouts", "plans/proposals", "routes", "activities", "coach", "exports"] {
            fs::create_dir_all(root.join(d)).map_err(|e| fsx::io_msg("create folder", &root.join(d), &e))?;
        }
        Ok(s)
    }

    /// Store-level migrations. Document-level migrations are handled by
    /// field defaults on load (see `json_struct!` defaults) and tested.
    fn migrate(&self, from: u32) -> Result<(), String> {
        if from == 0 {
            // v0 had no store.json; nothing to transform.
            self.write_value("store.json", &Value::obj([("store_version", STORE_VERSION.into()), ("migrated_from", 0.into())]))?;
        }
        Ok(())
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read_value(&self, rel: &str) -> Result<Option<Value>, String> {
        match read_string(&self.p(rel))? {
            None => Ok(None),
            Some(s) => rl_json::parse(&s).map(Some).map_err(|e| format!("{rel} is damaged ({e}).")),
        }
    }
    pub fn write_value(&self, rel: &str, v: &Value) -> Result<(), String> {
        atomic_write(&self.p(rel), v.to_string_pretty().as_bytes())
    }
    pub fn read_doc<T: FromJson>(&self, rel: &str) -> Result<Option<T>, String> {
        match self.read_value(rel)? {
            None => Ok(None),
            Some(v) => T::from_json(&v).map(Some).map_err(|e| format!("{rel}: {}", e.0)),
        }
    }
    pub fn write_doc<T: ToJson>(&self, rel: &str, t: &T) -> Result<(), String> {
        self.write_value(rel, &t.to_json())
    }
    pub fn remove(&self, rel: &str) -> Result<(), String> {
        match fs::remove_file(self.p(rel)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(fsx::io_msg("delete", &self.p(rel), &e)),
        }
    }

    // ---------------------------------------------------------------- profile

    pub fn profile(&self) -> Result<Option<RiderProfile>, String> {
        self.read_doc("profile.json")
    }
    pub fn save_profile(&self, p: &RiderProfile) -> Result<(), String> {
        p.validate()?;
        self.write_doc("profile.json", p)
    }
    pub fn ftp_history(&self) -> Result<Vec<FtpEntry>, String> {
        Ok(self.read_doc::<Vec<FtpEntry>>("ftp.json")?.unwrap_or_default())
    }
    pub fn save_ftp_history(&self, h: &[FtpEntry]) -> Result<(), String> {
        self.write_value("ftp.json", &h.to_vec().to_json())
    }

    // ---------------------------------------------------------------- workouts

    pub fn custom_workouts(&self) -> Result<Vec<Workout>, String> {
        let mut out = Vec::new();
        let dir = self.p("workouts");
        let Ok(rd) = fs::read_dir(&dir) else { return Ok(out) };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().map(|x| x == "json").unwrap_or(false) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    match self.read_doc::<Workout>(&format!("workouts/{name}")) {
                        Ok(Some(w)) => out.push(w),
                        Ok(None) => {}
                        Err(_) => {} // damaged files are skipped, not fatal
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    pub fn save_workout(&self, w: &Workout) -> Result<(), String> {
        let id = safe_id(&w.id)?;
        if let Some(old) = self.read_value(&format!("workouts/{id}.json"))? {
            let v = old.get("version").and_then(|x| x.as_i64()).unwrap_or(1);
            self.write_value(&format!("workouts/.versions/{id}.v{v}.json"), &old)?;
        }
        self.write_doc(&format!("workouts/{id}.json"), w)
    }
    pub fn delete_workout(&self, id: &str) -> Result<(), String> {
        let id = safe_id(id)?;
        if let Some(old) = self.read_value(&format!("workouts/{id}.json"))? {
            self.write_value(&format!("workouts/.versions/{id}.deleted.json"), &old)?;
        }
        self.remove(&format!("workouts/{id}.json"))
    }

    // ---------------------------------------------------------------- plans

    pub fn current_plan(&self) -> Result<Option<TrainingPlan>, String> {
        let Some(cur) = self.read_value("plans/current.json")? else { return Ok(None) };
        let (Some(id), Some(ver)) = (cur.get("plan_id").and_then(|v| v.as_str()), cur.get("version").and_then(|v| v.as_i64())) else { return Ok(None) };
        self.plan_version(id, ver as u32)
    }
    pub fn plan_version(&self, id: &str, version: u32) -> Result<Option<TrainingPlan>, String> {
        let id = safe_id(id)?;
        self.read_doc(&format!("plans/{id}/v{version}.json"))
    }
    pub fn save_plan_version(&self, p: &TrainingPlan) -> Result<(), String> {
        let id = safe_id(&p.id)?;
        self.write_doc(&format!("plans/{id}/v{}.json", p.version), p)
    }
    pub fn set_current_plan(&self, id: Option<(&str, u32)>) -> Result<(), String> {
        match id {
            Some((id, v)) => self.write_value("plans/current.json", &Value::obj([("plan_id", id.into()), ("version", v.into())])),
            None => self.remove("plans/current.json"),
        }
    }
    pub fn current_plan_pointer(&self) -> Result<Option<(String, u32)>, String> {
        Ok(self.read_value("plans/current.json")?.and_then(|c| Some((c.get("plan_id")?.as_str()?.to_string(), c.get("version")?.as_i64()? as u32))))
    }
    pub fn save_proposal(&self, id: &str, v: &Value) -> Result<(), String> {
        let id = safe_id(id)?;
        self.write_value(&format!("plans/proposals/{id}.json"), v)
    }
    pub fn proposal(&self, id: &str) -> Result<Option<Value>, String> {
        let id = safe_id(id)?;
        self.read_value(&format!("plans/proposals/{id}.json"))
    }
    pub fn proposals(&self) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.p("plans/proposals")) {
            for e in rd.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    if let Ok(Some(v)) = self.read_value(&format!("plans/proposals/{n}")) {
                        out.push(v);
                    }
                }
            }
        }
        out.sort_by_key(|v| -(v.get("created_utc").and_then(|x| x.as_i64()).unwrap_or(0)));
        Ok(out)
    }
    pub fn append_plan_log(&self, v: &Value) -> Result<(), String> {
        fsx::append_line(&self.p("plans/log.jsonl"), &v.to_string_compact())
    }
    pub fn plan_log(&self) -> Vec<Value> {
        read_jsonl(&self.p("plans/log.jsonl")).0
    }

    // ---------------------------------------------------------------- routes

    pub fn routes(&self) -> Result<Vec<(RouteRecord, Option<Value>)>, String> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.p("routes")) {
            for e in rd.flatten() {
                let Some(id) = e.file_name().to_str().map(|s| s.to_string()) else { continue };
                if safe_id(&id).is_err() {
                    continue;
                }
                if let Ok(Some(r)) = self.read_doc::<RouteRecord>(&format!("routes/{id}/route.json")) {
                    // Summary without the samples array (cheap listing).
                    let summary = self.read_value(&format!("routes/{id}/summary.json")).ok().flatten();
                    out.push((r, summary));
                }
            }
        }
        out.sort_by_key(|(r, _)| -r.created_utc);
        Ok(out)
    }
    pub fn route(&self, id: &str) -> Result<Option<RouteRecord>, String> {
        let id = safe_id(id)?;
        self.read_doc(&format!("routes/{id}/route.json"))
    }
    pub fn save_route(&self, r: &RouteRecord) -> Result<(), String> {
        let id = safe_id(&r.id)?;
        self.write_doc(&format!("routes/{id}/route.json"), r)
    }
    pub fn save_route_profile(&self, id: &str, p: &RouteProfile) -> Result<(), String> {
        let id = safe_id(id)?;
        self.write_value(&format!("routes/{id}/profile.json"), &profile_to_json(p))?;
        self.write_value(&format!("routes/{id}/summary.json"), &p.summary_json())
    }
    pub fn route_profile(&self, id: &str) -> Result<Option<RouteProfile>, String> {
        let id = safe_id(id)?;
        match self.read_value(&format!("routes/{id}/profile.json"))? {
            None => Ok(None),
            Some(v) => profile_from_json(&v).map(Some).map_err(|e| e.0),
        }
    }
    pub fn delete_route(&self, id: &str) -> Result<(), String> {
        let id = safe_id(id)?;
        let p = self.p(&format!("routes/{id}"));
        if p.exists() {
            fs::remove_dir_all(&p).map_err(|e| fsx::io_msg("delete", &p, &e))?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- activities

    fn adir(&self, id: &str) -> Result<PathBuf, String> {
        Ok(self.p(&format!("activities/{}", safe_id(id)?)))
    }

    pub fn begin_activity(&self, meta: &ActivityMeta) -> Result<ActivityJournal, String> {
        let dir = self.adir(&meta.id)?;
        let j = ActivityJournal::create(&dir)?;
        atomic_write(&dir.join("meta.json"), meta.to_json().to_string_pretty().as_bytes())?;
        Ok(j)
    }

    pub fn activity_meta(&self, id: &str) -> Result<Option<ActivityMeta>, String> {
        let id = safe_id(id)?;
        self.read_doc(&format!("activities/{id}/meta.json"))
    }

    pub fn finalize_activity(&self, meta: &ActivityMeta, summary: &Summary) -> Result<(), String> {
        let id = safe_id(&meta.id)?;
        self.write_doc(&format!("activities/{id}/summary.json"), summary)?;
        self.write_doc(&format!("activities/{id}/meta.json"), meta)
    }

    pub fn list_activities(&self) -> Result<Vec<(ActivityMeta, Option<Summary>)>, String> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.p("activities")) {
            for e in rd.flatten() {
                let Some(id) = e.file_name().to_str().map(|s| s.to_string()) else { continue };
                if safe_id(&id).is_err() {
                    continue;
                }
                if let Ok(Some(m)) = self.read_doc::<ActivityMeta>(&format!("activities/{id}/meta.json")) {
                    let s = self.read_doc::<Summary>(&format!("activities/{id}/summary.json")).ok().flatten();
                    out.push((m, s));
                }
            }
        }
        out.sort_by_key(|(m, _)| -m.start_utc);
        Ok(out)
    }

    pub fn load_activity(&self, id: &str) -> Result<Option<LoadedActivity>, String> {
        let dir = self.adir(id)?;
        let Some(meta) = self.activity_meta(id)? else { return Ok(None) };
        let (rows, bad1) = read_jsonl(&dir.join("samples.jsonl"));
        let samples: Vec<Sample> = rows.iter().filter_map(|r| Sample::from_json(r).ok()).collect();
        let (rows, bad2) = read_jsonl(&dir.join("laps.jsonl"));
        let laps: Vec<Lap> = rows.iter().filter_map(|r| Lap::from_json(r).ok()).collect();
        let (rows, bad3) = read_jsonl(&dir.join("events.jsonl"));
        let events: Vec<SessionEvent> = rows.iter().filter_map(|r| SessionEvent::from_json(r).ok()).collect();
        let summary = self.read_doc::<Summary>(&format!("activities/{}/summary.json", safe_id(id)?)).ok().flatten();
        let feedback = self.read_value(&format!("activities/{}/feedback.json", safe_id(id)?)).ok().flatten();
        Ok(Some(LoadedActivity { meta, samples, laps, events, summary, feedback, corrupt_lines: bad1 + bad2 + bad3 }))
    }

    pub fn save_feedback(&self, id: &str, v: &Value) -> Result<(), String> {
        let id = safe_id(id)?;
        if self.activity_meta(id)?.is_none() {
            return Err("Unknown activity.".into());
        }
        self.write_value(&format!("activities/{id}/feedback.json"), v)
    }

    pub fn delete_activity(&self, id: &str) -> Result<(), String> {
        let dir = self.adir(id)?;
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| fsx::io_msg("delete", &dir, &e))?;
        }
        Ok(())
    }

    /// Activities left in "recording" state by a crash (excluding `active`).
    pub fn incomplete_activities(&self, active: Option<&str>) -> Result<Vec<ActivityMeta>, String> {
        Ok(self.list_activities()?.into_iter().map(|(m, _)| m).filter(|m| m.status == "recording" && Some(m.id.as_str()) != active).collect())
    }

    /// Finalize an interrupted activity from its journal. Never touches the trainer.
    pub fn recover_activity(&self, id: &str) -> Result<(ActivityMeta, Summary, usize), String> {
        let a = self.load_activity(id)?.ok_or("Unknown activity.")?;
        let mut meta = a.meta;
        let mut laps = a.laps;
        let last = a.samples.last().cloned();
        let end_utc = last.as_ref().map(|s| s.utc_ms).unwrap_or(meta.start_utc);
        if laps.is_empty() || laps.last().map(|l| l.end_active_s) < last.as_ref().map(|s| s.active_s) {
            let start_active = laps.last().map(|l| l.end_active_s).unwrap_or(0);
            let start_t = laps.last().map(|l| l.end_t_ms).unwrap_or(0);
            let lap = Lap {
                index: laps.len() as u32,
                start_t_ms: start_t,
                end_t_ms: last.as_ref().map(|s| s.t_ms).unwrap_or(0),
                start_active_s: start_active,
                end_active_s: last.as_ref().map(|s| s.active_s).unwrap_or(0),
                start_utc_ms: meta.start_utc + start_t as i64,
                label: "Recovered".into(),
                trigger: "session_end".into(),
                step: None,
                target_w: None,
            };
            crate::fsx::append_line(&self.adir(id)?.join("laps.jsonl"), &lap.to_json().to_string_compact())?;
            laps.push(lap);
        }
        let elapsed = last.as_ref().map(|s| s.t_ms as f64 / 1000.0).unwrap_or(0.0);
        let summary = summarize(&a.samples, &laps, elapsed, meta.ftp_w);
        meta.status = "recovered".into();
        meta.end_utc = Some(end_utc);
        self.finalize_activity(&meta, &summary)?;
        Ok((meta, summary, a.corrupt_lines))
    }

    // ---------------------------------------------------------------- coach chat

    pub fn append_chat(&self, v: &Value) -> Result<(), String> {
        fsx::append_line(&self.p("coach/chat.jsonl"), &v.to_string_compact())
    }
    pub fn chat_history(&self, limit: usize) -> Vec<Value> {
        let (rows, _) = read_jsonl(&self.p("coach/chat.jsonl"));
        let n = rows.len();
        rows.into_iter().skip(n.saturating_sub(limit)).collect()
    }
    pub fn clear_chat(&self) -> Result<(), String> {
        self.remove("coach/chat.jsonl")
    }

    // ---------------------------------------------------------------- data management

    pub fn usage(&self) -> Value {
        let area = |d: &str| fsx::dir_size(&self.p(d));
        Value::obj([
            ("activities_bytes", area("activities").into()),
            ("routes_bytes", area("routes").into()),
            ("plans_bytes", area("plans").into()),
            ("workouts_bytes", area("workouts").into()),
            ("coach_bytes", area("coach").into()),
            ("exports_bytes", area("exports").into()),
            ("total_bytes", fsx::dir_size(&self.root).into()),
        ])
    }

    /// Delete everything Ridgeline stored locally.
    pub fn delete_all(&self) -> Result<(), String> {
        let rd = fs::read_dir(&self.root).map_err(|e| fsx::io_msg("read", &self.root, &e))?;
        for e in rd.flatten() {
            let p = e.path();
            let r = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
            r.map_err(|e| fsx::io_msg("delete", &p, &e))?;
        }
        Store::open(&self.root).map(|_| ())
    }

    pub fn export_all(&self, dest: &Path) -> Result<u64, String> {
        let mut n = 0;
        for d in ["activities", "routes", "plans", "workouts", "coach"] {
            let src = self.p(d);
            if src.exists() {
                n += fsx::copy_dir(&src, &dest.join(d))?;
            }
        }
        for f in ["profile.json", "settings.json", "ftp.json", "devices.json", "store.json"] {
            let src = self.p(f);
            if src.exists() {
                n += fs::copy(&src, dest.join(f)).map_err(|e| fsx::io_msg("copy", &src, &e))?;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_json::parse;
    use rl_session::record::RecordSink;

    fn tmp() -> Store {
        let d = std::env::temp_dir().join(format!("rl-store-{}", rl_domain::ids::new_uuid()));
        Store::open(&d).unwrap()
    }

    #[test]
    fn profile_and_migration_defaults() {
        let s = tmp();
        assert!(s.profile().unwrap().is_none());
        s.save_profile(&RiderProfile::demo()).unwrap();
        assert_eq!(s.profile().unwrap().unwrap(), RiderProfile::demo());
        // A minimal older document still loads with documented defaults.
        let old = r#"{"goal":"endurance","event_date":null,"experience":"some","recent_weekly_min":120,"consistent_weeks":4,
            "availability_min":[0,60,0,60,0,90,0],"long_ride_day":5,"rider_mass_kg":70,"bike_mass_kg":8,"max_hr":null,"threshold_hr":null}"#;
        crate::fsx::atomic_write(&s.root.join("profile.json"), old.as_bytes()).unwrap();
        let p = s.profile().unwrap().unwrap();
        assert_eq!(p.trainer_difficulty_pct, 100.0);
        assert!(!p.ai_consent.enabled, "consent defaults to off");
        // A newer store refuses to open rather than corrupting data.
        crate::fsx::atomic_write(&s.root.join("store.json"), br#"{"store_version":99}"#).unwrap();
        assert!(Store::open(&s.root).unwrap_err().contains("newer"));
        fs::remove_dir_all(&s.root).unwrap();
    }

    #[test]
    fn activity_crash_recovery() {
        let s = tmp();
        let meta = ActivityMeta {
            schema: 1,
            id: rl_domain::ids::new_uuid(),
            status: "recording".into(),
            demo: true,
            mode: "erg".into(),
            title: "Test".into(),
            start_utc: 1_790_000_000_000,
            end_utc: None,
            tz_name: "UTC".into(),
            tz_offset_min: 0,
            workout_id: None,
            workout_name: None,
            workout: None,
            route_id: None,
            route_name: None,
            ftp_w: Some(250.0),
            plan_session_id: None,
            virtual_route: false,
            app_version: "test".into(),
            created_utc: 0,
        };
        let mut j = s.begin_activity(&meta).unwrap();
        for i in 1..=30u32 {
            j.sample(&Sample { t_ms: i as u64 * 1000, utc_ms: meta.start_utc + i as i64 * 1000, active_s: i, power: Some(200.0), ..Default::default() }).unwrap();
            j.flush(i % 5 == 0).unwrap();
        }
        // Process "dies" here without finalizing: drop journal unflushed tail.
        drop(j);
        let inc = s.incomplete_activities(None).unwrap();
        assert_eq!(inc.len(), 1);
        let (m, sum, _bad) = s.recover_activity(&meta.id).unwrap();
        assert_eq!(m.status, "recovered");
        assert!(sum.timer_s >= 30.0 - 5.0, "at most the unflushed tail is lost: {}", sum.timer_s);
        assert!(s.incomplete_activities(None).unwrap().is_empty());
        let loaded = s.load_activity(&meta.id).unwrap().unwrap();
        assert_eq!(loaded.laps.len(), 1);
        fs::remove_dir_all(&s.root).unwrap();
    }

    #[test]
    fn ids_cannot_escape_the_store() {
        let s = tmp();
        assert!(s.route("../../etc").is_err());
        assert!(s.delete_activity("..").is_err());
        assert!(s.proposal("a/b").is_err());
        fs::remove_dir_all(&s.root).unwrap();
    }

    #[test]
    fn route_and_profile_roundtrip() {
        use rl_domain::geo::LatLon;
        use rl_domain::route::{process, synthetic, RouteInput};
        let s = tmp();
        let segs = vec![synthetic(LatLon { lat: 47.5, lon: -52.7 }, &[(500.0, 3.0), (500.0, -2.0)], false)];
        let src = ElevationSource { kind: "synthetic".into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: 0, attribution: String::new() };
        let p = process(&RouteInput { segments: &segs, source: src.clone(), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        let r = RouteRecord {
            schema: 1,
            id: rl_domain::ids::new_uuid(),
            name: "R".into(),
            kind: "synthetic".into(),
            description: String::new(),
            created_utc: 1,
            segments: segs.clone(),
            elevation: src,
            dem_segments: None,
            corrections: vec![],
            flat_fallback: false,
            waypoints: vec![],
            routing_provider: String::new(),
            attribution: String::new(),
            reversed_from: None,
            bundled: true,
        };
        s.save_route(&r).unwrap();
        s.save_route_profile(&r.id, &p).unwrap();
        let back = s.route(&r.id).unwrap().unwrap();
        assert_eq!(back.segments.len(), 1);
        assert_eq!(back.segments[0].len(), segs[0].len());
        let bp = s.route_profile(&r.id).unwrap().unwrap();
        assert_eq!(bp.samples.len(), p.samples.len());
        assert!((bp.at(250.0).grade.unwrap() - 3.0).abs() < 0.1);
        assert_eq!(s.routes().unwrap().len(), 1);
        let _ = parse("{}");
        fs::remove_dir_all(&s.root).unwrap();
    }
}
