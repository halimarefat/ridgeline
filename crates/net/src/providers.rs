//! Routing and elevation provider interfaces with free, key-less default
//! implementations:
//!
//! * Routing: Valhalla (public FOSSGIS instance by default, or self-hosted).
//!   Usage terms of the public server: ~1 request/s per user, no heavy or
//!   commercial production use; credit OpenStreetMap, Valhalla and FOSSGIS.
//! * Elevation: Open-Meteo Elevation API (Copernicus DEM GLO-90, ~90 m),
//!   free for non-commercial use without a key, up to 100 points per request.
//!   Credit Open-Meteo and the Copernicus programme.
//!
//! Map tiles are rendered by the UI (MapLibre + OpenFreeMap) and are not part
//! of the control path.

use crate::http::{HttpClient, HttpError, HttpRequest};
use rl_domain::geo::{decode_polyline, LatLon};
use rl_domain::route::ElevationSource;
use rl_json::{parse, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const MAX_WAYPOINTS: usize = 25;
pub const MAX_DEM_POINTS: usize = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub struct RoutedPath {
    pub points: Vec<LatLon>,
    pub distance_m: f64,
    pub provider: String,
    pub attribution: String,
}

pub trait RoutingProvider: Send + Sync {
    fn id(&self) -> String;
    fn route(&self, http: &dyn HttpClient, waypoints: &[LatLon], cancel: &AtomicBool) -> Result<RoutedPath, String>;
}

pub trait ElevationProvider: Send + Sync {
    fn source(&self) -> ElevationSource;
    /// Elevation in meters for each point (`None` where the source has no value).
    fn sample(&self, http: &dyn HttpClient, pts: &[LatLon], cancel: &AtomicBool, progress: &dyn Fn(usize, usize)) -> Result<Vec<Option<f64>>, String>;
}

fn http_err(e: HttpError, what: &str) -> String {
    match e {
        HttpError::Connect(_) => format!("Could not reach the {what} service. Check your internet connection."),
        HttpError::Timeout => format!("The {what} service did not answer in time."),
        HttpError::Cancelled => "Cancelled.".into(),
        e => format!("{what} request failed: {e}"),
    }
}

pub struct Valhalla {
    pub base_url: String,
    pub client_id: String,
}

impl Valhalla {
    pub fn fossgis() -> Valhalla {
        Valhalla { base_url: "https://valhalla1.openstreetmap.de".into(), client_id: "ridgeline-desktop".into() }
    }

    pub fn request_body(waypoints: &[LatLon]) -> String {
        let locs: Vec<Value> = waypoints
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let t = if i == 0 || i == waypoints.len() - 1 { "break" } else { "through" };
                Value::obj([("lat", p.lat.into()), ("lon", p.lon.into()), ("type", t.into())])
            })
            .collect();
        Value::obj([
            ("locations", Value::Arr(locs)),
            ("costing", "bicycle".into()),
            ("costing_options", Value::obj([("bicycle", Value::obj([("bicycle_type", "Road".into()), ("use_roads", 0.6.into()), ("use_hills", 0.5.into())]))])),
            ("directions_type", "none".into()),
            ("units", "kilometers".into()),
        ])
        .to_string_compact()
    }

    pub fn parse_response(body: &str) -> Result<(Vec<LatLon>, f64), String> {
        let v = parse(body).map_err(|e| format!("Routing response was not valid JSON ({e})."))?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return Err(format!("The routing service could not find a route: {}", err.chars().take(200).collect::<String>()));
        }
        let trip = v.get("trip").ok_or("Routing response has no trip.")?;
        let legs = trip.get("legs").and_then(|l| l.as_arr()).ok_or("Routing response has no legs.")?;
        let mut pts: Vec<LatLon> = Vec::new();
        for leg in legs {
            let shape = leg.get("shape").and_then(|s| s.as_str()).ok_or("Routing leg has no shape.")?;
            let p = decode_polyline(shape, 6).ok_or("Routing shape could not be decoded.")?;
            for q in p {
                if pts.last().map(|l| (l.lat - q.lat).abs() > 1e-9 || (l.lon - q.lon).abs() > 1e-9).unwrap_or(true) {
                    pts.push(q);
                }
            }
        }
        let km = trip.get("summary").and_then(|s| s.get("length")).and_then(|x| x.as_f64()).unwrap_or(0.0);
        if pts.len() < 2 {
            return Err("The routing service returned an empty route.".into());
        }
        Ok((pts, km * 1000.0))
    }
}

impl RoutingProvider for Valhalla {
    fn id(&self) -> String {
        format!("valhalla ({})", self.base_url)
    }
    fn route(&self, http: &dyn HttpClient, waypoints: &[LatLon], cancel: &AtomicBool) -> Result<RoutedPath, String> {
        if waypoints.len() < 2 || waypoints.len() > MAX_WAYPOINTS {
            return Err(format!("Choose a start, an end and at most {} points in between.", MAX_WAYPOINTS - 2));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let url = format!("{}/route", self.base_url.trim_end_matches('/'));
        let req = HttpRequest::post_json(&url, Self::request_body(waypoints), Duration::from_secs(30)).header("X-Client-Id", &self.client_id);
        let resp = http.send(&req).map_err(|e| http_err(e, "routing"))?;
        if resp.status == 429 {
            return Err("The public routing server is busy (rate limit). Wait a few seconds and try again.".into());
        }
        if resp.status >= 500 {
            return Err(format!("The routing service had an error (HTTP {}).", resp.status));
        }
        let (points, distance_m) = Self::parse_response(&resp.text())?;
        Ok(RoutedPath { points, distance_m, provider: self.id(), attribution: "Routing © Valhalla, FOSSGIS e.V.; map data © OpenStreetMap contributors (ODbL)".into() })
    }
}

pub struct OpenMeteo {
    pub base_url: String,
}

impl OpenMeteo {
    pub fn public() -> OpenMeteo {
        OpenMeteo { base_url: "https://api.open-meteo.com".into() }
    }
    pub fn parse_response(body: &str, n: usize) -> Result<Vec<Option<f64>>, String> {
        let v = parse(body).map_err(|e| format!("Elevation response was not valid JSON ({e})."))?;
        if v.get("error").and_then(|x| x.as_bool()) == Some(true) {
            return Err(format!("Elevation service error: {}", v.get("reason").and_then(|r| r.as_str()).unwrap_or("unknown")));
        }
        let arr = v.get("elevation").and_then(|e| e.as_arr()).ok_or("Elevation response has no data.")?;
        if arr.len() != n {
            return Err("Elevation response has the wrong number of values.".into());
        }
        Ok(arr.iter().map(|x| x.as_f64().filter(|e| e.is_finite() && (-500.0..=9000.0).contains(e))).collect())
    }
}

impl ElevationProvider for OpenMeteo {
    fn source(&self) -> ElevationSource {
        ElevationSource {
            kind: "dem".into(),
            dataset: "Copernicus DEM GLO-90 via Open-Meteo".into(),
            resolution_m: 90.0,
            fetched_utc: rl_domain::time::now_utc_ms(),
            attribution: "Elevation data: Open-Meteo.com (CC BY 4.0); Copernicus DEM © DLR e.V. 2010–2014 and © Airbus Defence and Space GmbH 2014–2018, provided under COPERNICUS by the European Union and ESA".into(),
        }
    }
    fn sample(&self, http: &dyn HttpClient, pts: &[LatLon], cancel: &AtomicBool, progress: &dyn Fn(usize, usize)) -> Result<Vec<Option<f64>>, String> {
        if pts.len() > MAX_DEM_POINTS {
            return Err("Route is too long for terrain sampling in one go.".into());
        }
        let mut out = Vec::with_capacity(pts.len());
        let chunks: Vec<&[LatLon]> = pts.chunks(100).collect();
        for (i, c) in chunks.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled.".into());
            }
            let lats: Vec<String> = c.iter().map(|p| format!("{:.5}", p.lat)).collect();
            let lons: Vec<String> = c.iter().map(|p| format!("{:.5}", p.lon)).collect();
            let url = format!("{}/v1/elevation?latitude={}&longitude={}", self.base_url.trim_end_matches('/'), lats.join(","), lons.join(","));
            let mut attempt = 0;
            let resp = loop {
                match http.send(&HttpRequest::get(&url, Duration::from_secs(20))) {
                    Ok(r) if r.status == 429 && attempt < 2 => {
                        attempt += 1;
                        std::thread::sleep(Duration::from_millis(1500 * attempt));
                    }
                    Ok(r) => break r,
                    Err(e) if attempt < 1 && matches!(e, HttpError::Timeout) => attempt += 1,
                    Err(e) => return Err(http_err(e, "elevation")),
                }
            };
            if resp.status != 200 {
                return Err(format!("Elevation service returned HTTP {}.", resp.status));
            }
            out.extend(Self::parse_response(&resp.text(), c.len())?);
            progress(i + 1, chunks.len());
            if i + 1 < chunks.len() {
                std::thread::sleep(Duration::from_millis(150)); // be polite to a free service
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::MockHttp;
    use rl_domain::geo::encode_polyline;

    #[test]
    fn valhalla_request_and_response() {
        let wps = vec![LatLon { lat: 47.56, lon: -52.71 }, LatLon { lat: 47.57, lon: -52.72 }, LatLon { lat: 47.58, lon: -52.70 }];
        let body = Valhalla::request_body(&wps);
        assert!(body.contains("\"costing\":\"bicycle\"") && body.contains("\"type\":\"through\""));
        let shape = encode_polyline(&wps, 6);
        let resp = format!("{{\"trip\":{{\"legs\":[{{\"shape\":\"{}\"}}],\"summary\":{{\"length\":3.2}},\"status\":0}}}}", shape.replace('\\', "\\\\"));
        let mock = MockHttp::new(vec![MockHttp::ok(&resp)]);
        let r = Valhalla::fossgis().route(&mock, &wps, &AtomicBool::new(false)).unwrap();
        assert_eq!(r.points.len(), 3);
        assert!((r.distance_m - 3200.0).abs() < 1e-6);
        let req = &mock.requests.lock().unwrap()[0];
        assert!(req.url.ends_with("/route"));
        assert!(req.headers.iter().any(|(k, _)| k == "X-Client-Id"));
        assert!(Valhalla::parse_response(r#"{"error_code":442,"error":"No path could be found for input"}"#).unwrap_err().contains("No path"));
    }

    #[test]
    fn open_meteo_batches_of_100() {
        let pts: Vec<LatLon> = (0..250).map(|i| LatLon { lat: 47.0 + i as f64 * 1e-4, lon: -52.0 }).collect();
        let mk = |n: usize| MockHttp::ok(&format!("{{\"elevation\":[{}]}}", vec!["12.5"; n].join(",")));
        let mock = MockHttp::new(vec![mk(100), mk(100), mk(50)]);
        let e = OpenMeteo::public().sample(&mock, &pts, &AtomicBool::new(false), &|_, _| {}).unwrap();
        assert_eq!(e.len(), 250);
        assert_eq!(e[249], Some(12.5));
        let reqs = mock.requests.lock().unwrap();
        assert_eq!(reqs.len(), 3);
        assert!(reqs[0].url.contains("/v1/elevation?latitude="));
        assert!(OpenMeteo::parse_response("{\"elevation\":[1,2]}", 3).is_err());
        assert_eq!(OpenMeteo::parse_response("{\"elevation\":[1,null]}", 2).unwrap(), vec![Some(1.0), None]);
    }
}
