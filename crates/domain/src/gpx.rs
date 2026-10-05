//! GPX 1.0/1.1 import with a small, hardened XML reader.
//!
//! Security: document type declarations and entity definitions are rejected
//! outright (no external-entity expansion is possible), only the five XML
//! predefined entities and numeric character references are decoded, and
//! input size, nesting depth and point counts are bounded. Text from the file
//! (names, descriptions) is untrusted data and is never used as instructions.

use crate::geo::valid_lat_lon;

pub const MAX_GPX_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_POINTS: usize = 200_000;
pub const MAX_SEGMENTS: usize = 500;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct RawPoint {
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawRoute {
    pub name: String,
    pub description: String,
    /// "track" or "route"
    pub kind: String,
    pub segments: Vec<Vec<RawPoint>>,
    /// Points dropped because coordinates were invalid.
    pub dropped_points: usize,
    /// Elevations ignored because they were outside a plausible range.
    pub dropped_elevations: usize,
}

impl RawRoute {
    pub fn point_count(&self) -> usize {
        self.segments.iter().map(|s| s.len()).sum()
    }
}

#[derive(Debug)]
enum Tok<'a> {
    Start { name: &'a str, attrs: Vec<(&'a str, String)>, self_closing: bool },
    End(&'a str),
    Text(String),
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn decode_entities(s: &str) -> Result<String, String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';').ok_or("unterminated entity reference")?;
        let ent = &after[..end];
        let c = match ent {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            e if e.starts_with("#x") || e.starts_with("#X") => char::from_u32(u32::from_str_radix(&e[2..], 16).map_err(|_| "bad character reference")?).ok_or("bad character reference")?,
            e if e.starts_with('#') => char::from_u32(e[1..].parse::<u32>().map_err(|_| "bad character reference")?).ok_or("bad character reference")?,
            _ => return Err(format!("entity '&{ent};' is not allowed")),
        };
        out.push(c);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

struct Lexer<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Result<Option<Tok<'a>>, String> {
        loop {
            if self.i >= self.s.len() {
                return Ok(None);
            }
            let rest = &self.s[self.i..];
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                let raw = &rest[..end];
                self.i += end;
                if raw.trim().is_empty() {
                    continue;
                }
                return Ok(Some(Tok::Text(decode_entities(raw)?)));
            }
            if rest.starts_with("<?") {
                let e = rest.find("?>").ok_or("unterminated processing instruction")?;
                self.i += e + 2;
                continue;
            }
            if rest.starts_with("<!--") {
                let e = rest[4..].find("-->").ok_or("unterminated comment")?;
                self.i += 4 + e + 3;
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                let e = rest[9..].find("]]>").ok_or("unterminated CDATA")?;
                let t = rest[9..9 + e].to_string();
                self.i += 9 + e + 3;
                return Ok(Some(Tok::Text(t)));
            }
            if rest.starts_with("<!") {
                return Err("document type declarations and entities are not allowed".into());
            }
            let e = find_tag_end(rest).ok_or("unterminated tag")?;
            let inner = &rest[1..e];
            self.i += e + 1;
            if let Some(n) = inner.strip_prefix('/') {
                return Ok(Some(Tok::End(n.trim())));
            }
            let (inner, self_closing) = match inner.strip_suffix('/') {
                Some(x) => (x, true),
                None => (inner, false),
            };
            let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let name = &inner[..name_end];
            if name.is_empty() {
                return Err("empty tag name".into());
            }
            let attrs = parse_attrs(&inner[name_end..])?;
            return Ok(Some(Tok::Start { name, attrs, self_closing }));
        }
    }
}

/// Find the closing '>' of a tag, skipping quoted attribute values.
fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &c) in s.as_bytes().iter().enumerate().skip(1) {
        match (quote, c) {
            (None, b'"') | (None, b'\'') => quote = Some(c),
            (Some(q), c2) if c2 == q => quote = None,
            (None, b'>') => return Some(i),
            (None, b'<') => return None,
            _ => {}
        }
    }
    None
}

fn parse_attrs(s: &str) -> Result<Vec<(&str, String)>, String> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let eq = rest.find('=').ok_or("malformed attribute")?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next().ok_or("malformed attribute")?;
        if q != '"' && q != '\'' {
            return Err("attribute values must be quoted".into());
        }
        let end = after[1..].find(q).ok_or("unterminated attribute")?;
        out.push((name, decode_entities(&after[1..1 + end])?));
        rest = after[1 + end + 1..].trim_start();
        if out.len() > 64 {
            return Err("too many attributes".into());
        }
    }
    Ok(out)
}

pub fn parse_gpx(input: &str) -> Result<RawRoute, String> {
    if input.len() > MAX_GPX_BYTES {
        return Err(format!("GPX file is larger than {} MB.", MAX_GPX_BYTES / (1024 * 1024)));
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let upper = input.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err("This file contains a document type declaration, which is not allowed.".into());
    }
    let mut lx = Lexer { s: input, i: 0 };
    let mut stack: Vec<String> = Vec::new();
    let mut saw_gpx = false;
    let mut name = String::new();
    let mut desc = String::new();
    let mut track_segs: Vec<Vec<RawPoint>> = Vec::new();
    let mut route_segs: Vec<Vec<RawPoint>> = Vec::new();
    let mut cur_pt: Option<RawPoint> = None;
    let mut cur_pt_valid = true;
    let mut dropped_points = 0usize;
    let mut dropped_ele = 0usize;
    let mut total = 0usize;

    while let Some(t) = lx.next().map_err(|e| format!("Malformed XML: {e}"))? {
        match t {
            Tok::Start { name: n, attrs, self_closing } => {
                let ln = local(n).to_string();
                if stack.is_empty() {
                    if ln != "gpx" {
                        return Err("Not a GPX file (root element is not <gpx>).".into());
                    }
                    saw_gpx = true;
                }
                match ln.as_str() {
                    "trkseg" => {
                        if stack.last().map(|s| s.as_str()) == Some("trk") {
                            track_segs.push(Vec::new());
                        }
                    }
                    "rte" => route_segs.push(Vec::new()),
                    "trkpt" | "rtept" => {
                        let get = |k: &str| attrs.iter().find(|(a, _)| *a == k).and_then(|(_, v)| v.trim().parse::<f64>().ok());
                        match (get("lat"), get("lon")) {
                            (Some(lat), Some(lon)) if valid_lat_lon(lat, lon) => {
                                cur_pt = Some(RawPoint { lat, lon, ele: None });
                                cur_pt_valid = true;
                            }
                            _ => {
                                cur_pt = None;
                                cur_pt_valid = false;
                                dropped_points += 1;
                            }
                        }
                        if ln == "trkpt" && track_segs.is_empty() {
                            track_segs.push(Vec::new()); // trkpt outside trkseg (lenient)
                        }
                    }
                    _ => {}
                }
                if self_closing {
                    if ln == "trkpt" || ln == "rtept" {
                        finish_point(&ln, &mut cur_pt, &mut track_segs, &mut route_segs, &mut total)?;
                    }
                } else {
                    stack.push(ln);
                    if stack.len() > MAX_DEPTH {
                        return Err("GPX nesting is too deep.".into());
                    }
                }
            }
            Tok::End(n) => {
                let ln = local(n);
                match stack.pop() {
                    Some(open) if open == ln => {}
                    _ => return Err(format!("Malformed XML: unexpected </{n}>.")),
                }
                if (ln == "trkpt" || ln == "rtept") && cur_pt_valid {
                    finish_point(ln, &mut cur_pt, &mut track_segs, &mut route_segs, &mut total)?;
                }
            }
            Tok::Text(t) => {
                let top = stack.last().map(|s| s.as_str()).unwrap_or("");
                let parent = if stack.len() >= 2 { stack[stack.len() - 2].as_str() } else { "" };
                match (parent, top) {
                    ("trkpt", "ele") | ("rtept", "ele") => {
                        if let Some(p) = cur_pt.as_mut() {
                            match t.trim().parse::<f64>() {
                                Ok(e) if e.is_finite() && (-500.0..=9000.0).contains(&e) => p.ele = Some(e),
                                _ => dropped_ele += 1,
                            }
                        }
                    }
                    ("metadata", "name") | ("trk", "name") | ("rte", "name") if name.is_empty() => {
                        name = t.trim().chars().filter(|c| !c.is_control()).take(120).collect();
                    }
                    ("metadata", "desc") | ("trk", "desc") | ("rte", "desc") if desc.is_empty() => {
                        desc = t.trim().chars().filter(|c| !c.is_control() || *c == '\n').take(500).collect();
                    }
                    _ => {}
                }
            }
        }
    }
    if !saw_gpx {
        return Err("Not a GPX file.".into());
    }
    if !stack.is_empty() {
        return Err("Malformed XML: unclosed elements.".into());
    }
    let (kind, segs) = if track_segs.iter().any(|s| !s.is_empty()) { ("track", track_segs) } else { ("route", route_segs) };
    let segments: Vec<Vec<RawPoint>> = segs.into_iter().filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err("The GPX file has no track or route points.".into());
    }
    if segments.len() > MAX_SEGMENTS {
        return Err("The GPX file has too many track segments.".into());
    }
    Ok(RawRoute { name, description: desc, kind: kind.into(), segments, dropped_points, dropped_elevations: dropped_ele })
}

fn finish_point(ln: &str, cur: &mut Option<RawPoint>, trk: &mut [Vec<RawPoint>], rte: &mut [Vec<RawPoint>], total: &mut usize) -> Result<(), String> {
    if let Some(p) = cur.take() {
        *total += 1;
        if *total > MAX_POINTS {
            return Err(format!("The GPX file has more than {MAX_POINTS} points."));
        }
        let target = if ln == "trkpt" { trk.last_mut() } else { rte.last_mut() };
        if let Some(seg) = target {
            seg.push(p);
        }
    }
    Ok(())
}

/// Serialize geometry back to GPX 1.1 (route geometry export only).
pub fn to_gpx(name: &str, segments: &[Vec<RawPoint>]) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }
    let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"Ridgeline\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n");
    o.push_str(&format!("  <trk><name>{}</name>\n", esc(name)));
    for seg in segments {
        o.push_str("    <trkseg>\n");
        for p in seg {
            match p.ele {
                Some(e) => o.push_str(&format!("      <trkpt lat=\"{:.7}\" lon=\"{:.7}\"><ele>{:.1}</ele></trkpt>\n", p.lat, p.lon, e)),
                None => o.push_str(&format!("      <trkpt lat=\"{:.7}\" lon=\"{:.7}\"/>\n", p.lat, p.lon)),
            }
        }
        o.push_str("    </trkseg>\n");
    }
    o.push_str("  </trk>\n</gpx>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- comment -->
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1" xmlns:gpxtpx="http://www.garmin.com/xmlschemas/TrackPointExtension/v1">
  <metadata><name>Morning &amp; Hills</name></metadata>
  <trk><name>ignored second name</name>
    <trkseg>
      <trkpt lat="47.5" lon="-52.7"><ele>10.0</ele><time>2026-01-01T00:00:00Z</time>
        <extensions><gpxtpx:TrackPointExtension><gpxtpx:hr>140</gpxtpx:hr></gpxtpx:TrackPointExtension></extensions></trkpt>
      <trkpt lat='47.501' lon='-52.7'><ele>12.5</ele></trkpt>
      <trkpt lat="999" lon="-52.7"><ele>12.5</ele></trkpt>
    </trkseg>
    <trkseg><trkpt lat="47.6" lon="-52.6"/></trkseg>
  </trk>
</gpx>"#;

    #[test]
    fn parses_tracks_segments_and_entities() {
        let r = parse_gpx(SAMPLE).unwrap();
        assert_eq!(r.name, "Morning & Hills");
        assert_eq!(r.kind, "track");
        assert_eq!(r.segments.len(), 2);
        assert_eq!(r.segments[0].len(), 2);
        assert_eq!(r.segments[0][1].ele, Some(12.5));
        assert_eq!(r.segments[1][0].ele, None);
        assert_eq!(r.dropped_points, 1);
    }

    #[test]
    fn rejects_xxe_and_garbage() {
        let xxe = r#"<?xml version="1.0"?><!DOCTYPE gpx [<!ENTITY x SYSTEM "file:///etc/passwd">]><gpx><trk><name>&x;</name></trk></gpx>"#;
        assert!(parse_gpx(xxe).unwrap_err().contains("not allowed"));
        assert!(parse_gpx("<gpx><trk><name>&foo;</name></trk></gpx>").is_err());
        assert!(parse_gpx("<kml></kml>").is_err());
        assert!(parse_gpx("<gpx><trk><trkseg><trkpt lat=\"1\" lon=\"2\"></trkseg></trk></gpx>").is_err());
        assert!(parse_gpx("<gpx></gpx>").unwrap_err().contains("no track"));
        assert!(parse_gpx("not xml at all").is_err());
    }

    #[test]
    fn routes_and_roundtrip() {
        let src = r#"<gpx><rte><name>R</name><rtept lat="1" lon="2"><ele>5</ele></rtept><rtept lat="1.001" lon="2"/></rte></gpx>"#;
        let r = parse_gpx(src).unwrap();
        assert_eq!(r.kind, "route");
        assert_eq!(r.point_count(), 2);
        let back = parse_gpx(&to_gpx("A <b>", &r.segments)).unwrap();
        assert_eq!(back.name, "A <b>");
        assert_eq!(back.segments[0][0].ele, Some(5.0));
    }
}
