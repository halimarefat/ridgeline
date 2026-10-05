//! Geodesy helpers on a spherical Earth (mean radius). Horizontal distances
//! are ground distances in meters; longitudes are handled across the
//! antimeridian.

pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

pub fn valid_lat_lon(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

/// Normalise a longitude difference into (-180, 180].
pub fn wrap_dlon(d: f64) -> f64 {
    let mut x = (d + 180.0).rem_euclid(360.0) - 180.0;
    if x == -180.0 {
        x = 180.0;
    }
    x
}

pub fn haversine_m(a: LatLon, b: LatLon) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = wrap_dlon(b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Linear interpolation in lat/lon space with antimeridian-aware longitude.
/// Accurate enough for points ≤ a few km apart (routes are densely sampled).
pub fn lerp(a: LatLon, b: LatLon, f: f64) -> LatLon {
    let dl = wrap_dlon(b.lon - a.lon);
    let mut lon = a.lon + dl * f;
    if lon > 180.0 {
        lon -= 360.0;
    } else if lon < -180.0 {
        lon += 360.0;
    }
    LatLon { lat: a.lat + (b.lat - a.lat) * f, lon }
}

/// Make longitudes continuous along a path (for map rendering across ±180°).
pub fn unwrap_lons(lons: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(lons.len());
    let mut prev: Option<f64> = None;
    for &l in lons {
        let v = match prev {
            None => l,
            Some(p) => p + wrap_dlon(l - p),
        };
        out.push(v);
        prev = Some(v);
    }
    out
}

/// Encode/decode Google-style polylines (precision 5 or 6, as used by
/// routing services).
pub fn decode_polyline(s: &str, precision: u32) -> Option<Vec<LatLon>> {
    let factor = 10f64.powi(precision as i32);
    let b = s.as_bytes();
    let mut i = 0;
    let (mut lat, mut lon) = (0i64, 0i64);
    let mut out = Vec::new();
    let next = |i: &mut usize| -> Option<i64> {
        let mut result: i64 = 0;
        let mut shift = 0;
        loop {
            let c = *b.get(*i)? as i64 - 63;
            *i += 1;
            if !(0..64).contains(&c) || shift > 60 {
                return None;
            }
            result |= (c & 0x1f) << shift;
            shift += 5;
            if c < 0x20 {
                break;
            }
        }
        Some(if result & 1 != 0 { !(result >> 1) } else { result >> 1 })
    };
    while i < b.len() {
        lat += next(&mut i)?;
        lon += next(&mut i)?;
        let p = LatLon { lat: lat as f64 / factor, lon: lon as f64 / factor };
        if !valid_lat_lon(p.lat, p.lon) {
            return None;
        }
        out.push(p);
    }
    Some(out)
}

pub fn encode_polyline(pts: &[LatLon], precision: u32) -> String {
    let factor = 10f64.powi(precision as i32);
    let mut out = String::new();
    let (mut plat, mut plon) = (0i64, 0i64);
    let enc = |v: i64, out: &mut String| {
        let mut v = if v < 0 { !(v << 1) } else { v << 1 };
        while v >= 0x20 {
            out.push((((v & 0x1f) | 0x20) as u8 + 63) as char);
            v >>= 5;
        }
        out.push((v as u8 + 63) as char);
    };
    for p in pts {
        let lat = (p.lat * factor).round() as i64;
        let lon = (p.lon * factor).round() as i64;
        enc(lat - plat, &mut out);
        enc(lon - plon, &mut out);
        plat = lat;
        plon = lon;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        // One degree of latitude ≈ 111.2 km on the mean sphere.
        let d = haversine_m(LatLon { lat: 0.0, lon: 0.0 }, LatLon { lat: 1.0, lon: 0.0 });
        assert!((d - 111_195.0).abs() < 50.0, "{d}");
        // Across the antimeridian is short, not ~40,000 km.
        let d = haversine_m(LatLon { lat: 0.0, lon: 179.999 }, LatLon { lat: 0.0, lon: -179.999 });
        assert!(d < 300.0, "{d}");
        let m = lerp(LatLon { lat: 0.0, lon: 179.9 }, LatLon { lat: 0.0, lon: -179.9 }, 0.5);
        assert!((m.lon.abs() - 180.0).abs() < 1e-9, "{m:?}");
        assert_eq!(unwrap_lons(&[179.0, -179.0, -178.0]), vec![179.0, 181.0, 182.0]);
    }

    #[test]
    fn polyline_roundtrip() {
        // Reference example from the polyline algorithm documentation (precision 5).
        let pts = decode_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@", 5).unwrap();
        assert_eq!(pts.len(), 3);
        assert!((pts[0].lat - 38.5).abs() < 1e-9 && (pts[0].lon + 120.2).abs() < 1e-9);
        assert!((pts[2].lat - 43.252).abs() < 1e-9 && (pts[2].lon + 126.453).abs() < 1e-9);
        assert_eq!(encode_polyline(&pts, 5), "_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        let p6 = encode_polyline(&pts, 6);
        let back = decode_polyline(&p6, 6).unwrap();
        assert!((back[1].lon - pts[1].lon).abs() < 1e-9);
        assert!(decode_polyline("\u{7f}", 5).is_none());
    }
}
