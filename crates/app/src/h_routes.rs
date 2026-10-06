//! Routes: GPX import, route builder (routing provider), elevation
//! sampling, corrections, reversal, export.

use crate::app::{popt_str, pstr, R};
use crate::App;
use rl_domain::geo::LatLon;
use rl_domain::gpx::{parse_gpx, to_gpx, RawPoint};
use rl_domain::route::{densify, process, reverse_corrections, reverse_segments, Correction, ElevationSource, ProfileConfig, RouteInput};
use rl_domain::time::now_utc_ms;
use rl_json::{ToJson, Value};
use rl_net::providers::{ElevationProvider, OpenMeteo, RoutingProvider, Valhalla};
use rl_storage::store::RouteRecord;
use std::sync::Arc;

/// Rebuild the processed profile from a record (single source of truth).
pub fn reprocess(rec: &RouteRecord) -> Result<rl_domain::route::RouteProfile, String> {
    let segs: &[Vec<RawPoint>] = rec.dem_segments.as_deref().unwrap_or(&rec.segments);
    process(&RouteInput { segments: segs, source: rec.elevation.clone(), corrections: &rec.corrections, flat_fallback: rec.flat_fallback }, &ProfileConfig::default())
}

fn sanitize_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_control()).take(100).collect::<String>().trim().to_string();
    if n.is_empty() {
        "Untitled route".into()
    } else {
        n
    }
}

impl App {
    fn save_and_process(&mut self, rec: &RouteRecord) -> Result<rl_domain::route::RouteProfile, String> {
        let prof = reprocess(rec)?;
        self.store.save_route(rec)?;
        self.store.save_route_profile(&rec.id, &prof)?;
        self.route_cache.insert(rec.id.clone(), Arc::new(prof.clone()));
        Ok(prof)
    }

    pub(crate) fn list_routes(&mut self, _p: &Value) -> R {
        let rows: Vec<Value> = self
            .store
            .routes()?
            .into_iter()
            .map(|(r, s)| {
                Value::obj([
                    ("id", r.id.into()),
                    ("name", r.name.into()),
                    ("kind", r.kind.into()),
                    ("bundled", r.bundled.into()),
                    ("created_utc", r.created_utc.into()),
                    ("elevation_kind", r.elevation.kind.into()),
                    ("summary", s.unwrap_or(Value::Null)),
                ])
            })
            .collect();
        Ok(Value::Arr(rows))
    }

    pub(crate) fn get_route(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let rec = self.store.route(id)?.ok_or("Unknown route.")?;
        let prof = self.route_profile(id)?;
        let display = prof.display_json(p.opt_i64("max_points")?.unwrap_or(2500).clamp(100, 20_000) as usize);
        Ok(Value::obj([
            ("id", rec.id.clone().into()),
            ("name", rec.name.clone().into()),
            ("kind", rec.kind.clone().into()),
            ("description", rec.description.clone().into()),
            ("bundled", rec.bundled.into()),
            ("waypoints", Value::Arr(rec.waypoints.iter().map(|(a, b)| Value::Arr(vec![(*a).into(), (*b).into()])).collect())),
            ("routing_provider", rec.routing_provider.clone().into()),
            ("attribution", rec.attribution.clone().into()),
            ("elevation", rec.elevation.to_json()),
            ("corrections", rec.corrections.to_json()),
            ("summary", prof.summary_json()),
            ("display", display),
            ("raw_points", rec.segments.iter().map(|s| s.len()).sum::<usize>().into()),
        ]))
    }

    pub(crate) fn import_gpx(&mut self, p: &Value) -> R {
        let text = p.req_str("text")?;
        let raw = parse_gpx(text)?;
        let name = sanitize_name(popt_str(p, "name", 200)?.filter(|s| !s.trim().is_empty()).unwrap_or(&raw.name));
        let total = raw.point_count();
        let with_ele = raw.segments.iter().flatten().filter(|p| p.ele.is_some()).count();
        let source = if with_ele * 10 >= total * 9 {
            ElevationSource { kind: "imported".into(), dataset: "GPX file elevations".into(), resolution_m: 0.0, fetched_utc: now_utc_ms(), attribution: String::new() }
        } else {
            ElevationSource { kind: "none".into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: now_utc_ms(), attribution: String::new() }
        };
        let rec = RouteRecord {
            schema: 1,
            id: rl_domain::ids::new_uuid(),
            name,
            kind: "gpx".into(),
            description: raw.description.clone(),
            created_utc: now_utc_ms(),
            segments: raw.segments,
            elevation: source,
            dem_segments: None,
            corrections: vec![],
            flat_fallback: false,
            waypoints: vec![],
            routing_provider: String::new(),
            attribution: String::new(),
            reversed_from: None,
            bundled: false,
        };
        let prof = self.save_and_process(&rec)?;
        Ok(Value::obj([
            ("id", rec.id.into()),
            ("dropped_points", raw.dropped_points.into()),
            ("dropped_elevations", raw.dropped_elevations.into()),
            ("needs_elevation", (!prof.simulation_ready).into()),
        ]))
    }

    pub(crate) fn build_route(&mut self, p: &Value) -> R {
        if !self.settings.providers.routing_enabled {
            return Err("The routing service is turned off in Settings.".into());
        }
        let wps: Vec<(f64, f64)> = rl_json::field(p, "waypoints")?;
        if wps.len() < 2 || wps.len() > rl_net::providers::MAX_WAYPOINTS {
            return Err("Place a start, an end and up to 23 points in between.".into());
        }
        let pts: Vec<LatLon> = wps.iter().map(|(lat, lon)| LatLon { lat: *lat, lon: *lon }).collect();
        if pts.iter().any(|p| !rl_domain::geo::valid_lat_lon(p.lat, p.lon)) {
            return Err("Invalid coordinates.".into());
        }
        let name = sanitize_name(popt_str(p, "name", 200)?.unwrap_or("Built route"));
        let fetch_ele = p.bool_or("fetch_elevation", true) && self.settings.providers.elevation_enabled;
        let routing = Valhalla { base_url: self.settings.providers.routing_url.clone(), client_id: "ridgeline-desktop".into() };
        let elev = OpenMeteo { base_url: self.settings.providers.elevation_url.clone() };
        let http = self.http.clone();
        let job = self.spawn_job("build_route", move |ctx| {
            ctx.progress(0, 2, "Finding a route on cycling roads…");
            let path = routing.route(http.as_ref(), &pts, &ctx.cancel)?;
            let segs = vec![path.points.iter().map(|p| RawPoint { lat: p.lat, lon: p.lon, ele: None }).collect::<Vec<_>>()];
            let mut rec = RouteRecord {
                schema: 1,
                id: rl_domain::ids::new_uuid(),
                name,
                kind: "built".into(),
                description: String::new(),
                created_utc: now_utc_ms(),
                segments: segs,
                elevation: ElevationSource { kind: "none".into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: 0, attribution: String::new() },
                dem_segments: None,
                corrections: vec![],
                flat_fallback: false,
                waypoints: wps,
                routing_provider: path.provider.clone(),
                attribution: path.attribution.clone(),
                reversed_from: None,
                bundled: false,
            };
            let mut warn = None;
            if fetch_ele {
                match sample_dem(&rec, &elev, http.as_ref(), ctx) {
                    Ok((src, dem)) => {
                        rec.elevation = src;
                        rec.dem_segments = Some(dem);
                    }
                    Err(e) => warn = Some(format!("Route saved without elevation: {e}")),
                }
            }
            let id = rec.id.clone();
            ctx.with_app(|a| a.save_and_process(&rec)).unwrap_or_else(|| Err("App closed.".into()))?;
            Ok(Value::obj([("id", id.into()), ("distance_m", path.distance_m.into()), ("warning", warn.into())]))
        })?;
        Ok(Value::obj([("job_id", job.into())]))
    }

    pub(crate) fn fetch_elevation(&mut self, p: &Value) -> R {
        if !self.settings.providers.elevation_enabled {
            return Err("The elevation service is turned off in Settings.".into());
        }
        let id = pstr(p, "id", 64)?.to_string();
        let rec = self.store.route(&id)?.ok_or("Unknown route.")?;
        if rec.bundled {
            return Err("Bundled synthetic routes already have elevation.".into());
        }
        let elev = OpenMeteo { base_url: self.settings.providers.elevation_url.clone() };
        let http = self.http.clone();
        let job = self.spawn_job("fetch_elevation", move |ctx| {
            let (src, dem) = sample_dem(&rec, &elev, http.as_ref(), ctx)?;
            let mut rec = rec;
            rec.elevation = src;
            rec.dem_segments = Some(dem);
            ctx.with_app(|a| a.save_and_process(&rec)).unwrap_or_else(|| Err("App closed.".into()))?;
            Ok(Value::obj([("id", rec.id.into())]))
        })?;
        Ok(Value::obj([("job_id", job.into())]))
    }

    fn mutate_route(&mut self, id: &str, f: impl FnOnce(&mut RouteRecord) -> Result<(), String>) -> R {
        if self.ride_active() && self.session.as_ref().and_then(|s| s.route.as_ref()).map(|r| r.route_id == id).unwrap_or(false) {
            return Err("This route is being ridden right now.".into());
        }
        let mut rec = self.store.route(id)?.ok_or("Unknown route.")?;
        f(&mut rec)?;
        let prof = self.save_and_process(&rec)?;
        Ok(prof.summary_json())
    }

    pub(crate) fn set_flat_fallback(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?.to_string();
        let on = p.req_bool("on")?;
        self.mutate_route(&id, |r| {
            r.flat_fallback = on;
            Ok(())
        })
    }

    pub(crate) fn add_correction(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?.to_string();
        let from = p.req_f64("from_m")?;
        let to = p.req_f64("to_m")?;
        let note = p.str_or("note", "").chars().take(200).collect::<String>();
        if !(from.is_finite() && to.is_finite()) || to - from < 20.0 || to - from > 5000.0 {
            return Err("A correction must span 20 m to 5 km.".into());
        }
        self.mutate_route(&id, |r| {
            if r.corrections.len() >= 50 {
                return Err("Too many corrections.".into());
            }
            r.corrections.push(Correction { from_m: from.max(0.0), to_m: to, kind: "linear".into(), note });
            Ok(())
        })
    }

    pub(crate) fn clear_corrections(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?.to_string();
        self.mutate_route(&id, |r| {
            r.corrections.clear();
            Ok(())
        })
    }

    pub(crate) fn rename_route(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?.to_string();
        let name = sanitize_name(pstr(p, "name", 200)?);
        let mut rec = self.store.route(&id)?.ok_or("Unknown route.")?;
        rec.name = name;
        self.store.save_route(&rec)?;
        Ok(Value::Null)
    }

    pub(crate) fn reverse_route(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let rec = self.store.route(id)?.ok_or("Unknown route.")?;
        let prof = self.route_profile(id)?;
        let total = prof.total_m;
        let new = RouteRecord {
            id: rl_domain::ids::new_uuid(),
            name: format!("{} (reversed)", rec.name).chars().take(100).collect(),
            kind: "reversed".into(),
            created_utc: now_utc_ms(),
            segments: reverse_segments(&rec.segments),
            dem_segments: rec.dem_segments.as_ref().map(|d| reverse_segments(d)),
            corrections: reverse_corrections(&rec.corrections, total),
            waypoints: rec.waypoints.iter().rev().cloned().collect(),
            reversed_from: Some(rec.id.clone()),
            bundled: false,
            ..rec
        };
        self.save_and_process(&new)?;
        Ok(Value::obj([("id", new.id.into())]))
    }

    pub(crate) fn delete_route(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let rec = self.store.route(id)?.ok_or("Unknown route.")?;
        if rec.bundled {
            return Err("Bundled demo routes can't be deleted.".into());
        }
        if self.session.as_ref().and_then(|s| s.route.as_ref()).map(|r| r.route_id == id).unwrap_or(false) {
            return Err("This route is in use by the current ride.".into());
        }
        self.store.delete_route(id)?;
        self.route_cache.remove(id);
        Ok(Value::Null)
    }

    pub(crate) fn export_route_gpx(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let rec = self.store.route(id)?.ok_or("Unknown route.")?;
        // Export the processed profile geometry with smoothed elevation.
        let prof = self.route_profile(id)?;
        let mut segs: Vec<Vec<RawPoint>> = Vec::new();
        for s in &prof.samples {
            if segs.len() <= s.seg as usize {
                segs.push(Vec::new());
            }
            segs[s.seg as usize].push(RawPoint { lat: s.lat, lon: s.lon, ele: s.ele });
        }
        let path = self.write_export(&format!("{}.gpx", crate::h_history::file_stem(&rec.name)), to_gpx(&rec.name, &segs).as_bytes())?;
        Ok(Value::obj([("path", path.into())]))
    }
}

/// Sample a terrain model along the route (~45 m spacing, half the 90 m
/// DEM resolution — more samples would not add accuracy).
fn sample_dem(rec: &RouteRecord, elev: &OpenMeteo, http: &dyn rl_net::http::HttpClient, ctx: &crate::jobs::JobCtx) -> Result<(ElevationSource, Vec<Vec<RawPoint>>), String> {
    let dense = densify(&rec.segments, 45.0);
    let flat: Vec<LatLon> = dense.iter().flatten().map(|p| LatLon { lat: p.lat, lon: p.lon }).collect();
    if flat.len() > rl_net::providers::MAX_DEM_POINTS {
        return Err("Route is too long for terrain sampling (about 450 km maximum).".into());
    }
    let progress = |a: usize, b: usize| ctx.progress(a, b, "Sampling terrain elevation…");
    let e = elev.sample(http, &flat, &ctx.cancel, &progress)?;
    let mut k = 0;
    let mut out = Vec::new();
    for seg in dense {
        let mut o = Vec::new();
        for p in seg {
            o.push(RawPoint { lat: p.lat, lon: p.lon, ele: e[k] });
            k += 1;
        }
        out.push(o);
    }
    Ok((elev.source(), out))
}
