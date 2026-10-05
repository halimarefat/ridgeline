//! Route elevation pipeline (spec §7): validation, segment preservation,
//! horizontal distance, elevation fill, ~10 m resampling, spike removal,
//! smoothing in distance space, windowed grade, flags and statistics.
//!
//! The processed profile is the single source for resistance, displayed
//! grade, elevation chart and climbing totals. Raw input is retained by the
//! store for audit and reprocessing; `PROFILE_VERSION` changes whenever the
//! algorithm changes.

use crate::geo::{haversine_m, lerp, LatLon};
use crate::gpx::RawPoint;
use rl_json::{json_struct, ToJson, Value};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileConfig {
    pub step_m: f64,
    /// Hampel filter half-window in samples.
    pub spike_half_window: usize,
    pub spike_k: f64,
    pub spike_min_m: f64,
    pub smooth_window_m: f64,
    pub grade_window_m: f64,
    pub steep_flag_pct: f64,
    /// Grade change between adjacent samples that is flagged as suspicious.
    pub abrupt_change_pct: f64,
    /// A jump between consecutive points larger than this splits the segment.
    pub split_gap_m: f64,
    /// Missing-elevation runs up to this length are linearly interpolated.
    pub max_fill_m: f64,
}
json_struct!(ProfileConfig {
    step_m: "step_m",
    spike_half_window: "spike_half_window",
    spike_k: "spike_k",
    spike_min_m: "spike_min_m",
    smooth_window_m: "smooth_window_m",
    grade_window_m: "grade_window_m",
    steep_flag_pct: "steep_flag_pct",
    abrupt_change_pct: "abrupt_change_pct",
    split_gap_m: "split_gap_m",
    max_fill_m: "max_fill_m",
});

impl Default for ProfileConfig {
    fn default() -> Self {
        ProfileConfig {
            step_m: 10.0,
            spike_half_window: 5,
            spike_k: 3.0,
            spike_min_m: 3.0,
            smooth_window_m: 30.0,
            grade_window_m: 40.0,
            steep_flag_pct: 25.0,
            abrupt_change_pct: 8.0,
            split_gap_m: 2000.0,
            max_fill_m: 200.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElevationSource {
    /// "imported" (from the file), "dem" (terrain model), "synthetic", "none".
    pub kind: String,
    pub dataset: String,
    /// Nominal horizontal resolution of the source, meters (0 = unknown).
    pub resolution_m: f64,
    pub fetched_utc: i64,
    pub attribution: String,
}
json_struct!(ElevationSource {
    kind: "kind",
    dataset: "dataset" = String::new(),
    resolution_m: "resolution_m" = 0.0,
    fetched_utc: "fetched_utc" = 0,
    attribution: "attribution" = String::new(),
});

impl ElevationSource {
    pub fn of(kind: &str) -> ElevationSource {
        ElevationSource { kind: kind.into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: 0, attribution: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub from_m: f64,
    pub to_m: f64,
    /// "linear": replace elevation between the endpoints with a straight line
    /// (typical for bridges and tunnels where terrain models show a dip/hump).
    pub kind: String,
    pub note: String,
}
json_struct!(Correction { from_m: "from_m", to_m: "to_m", kind: "kind", note: "note" = String::new() });

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub s: f64,
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
    pub grade: Option<f64>,
    pub seg: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    pub from_m: f64,
    pub to_m: f64,
    pub kind: String,
    pub message: String,
}
json_struct!(Flag { from_m: "from_m", to_m: "to_m", kind: "kind", message: "message" });

#[derive(Debug, Clone, PartialEq)]
pub struct RouteProfile {
    pub version: u32,
    pub config: ProfileConfig,
    pub total_m: f64,
    pub samples: Vec<Sample>,
    pub segment_starts_m: Vec<f64>,
    pub ascent_m: f64,
    pub descent_m: f64,
    pub max_grade: f64,
    pub min_grade: f64,
    pub ele_min: Option<f64>,
    pub ele_max: Option<f64>,
    pub histogram: Vec<(String, f64)>,
    pub source: ElevationSource,
    pub coverage_pct: f64,
    pub spikes_removed: u32,
    pub duplicates_removed: u32,
    pub flags: Vec<Flag>,
    pub flat_fallback: bool,
    pub simulation_ready: bool,
    pub blocked_reason: Option<String>,
}

pub struct RouteInput<'a> {
    pub segments: &'a [Vec<RawPoint>],
    pub source: ElevationSource,
    pub corrections: &'a [Correction],
    /// Rider explicitly accepted a flat fallback for missing elevation.
    pub flat_fallback: bool,
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

struct Seg {
    pts: Vec<LatLon>,
    ele: Vec<Option<f64>>,
    cum: Vec<f64>,
}

/// Remove near-duplicates, split at large jumps, compute cumulative distance.
fn prepare_segments(input: &[Vec<RawPoint>], cfg: &ProfileConfig, dups: &mut u32, flags: &mut Vec<Flag>) -> Vec<Seg> {
    let mut out = Vec::new();
    for raw in input {
        let mut cur = Seg { pts: vec![], ele: vec![], cum: vec![] };
        for p in raw {
            let ll = LatLon { lat: p.lat, lon: p.lon };
            if let Some(last) = cur.pts.last() {
                let d = haversine_m(*last, ll);
                if d < 0.5 {
                    *dups += 1;
                    if cur.ele.last().map(|e| e.is_none()).unwrap_or(false) {
                        *cur.ele.last_mut().unwrap() = p.ele;
                    }
                    continue;
                }
                if d > cfg.split_gap_m {
                    flags.push(Flag { from_m: 0.0, to_m: 0.0, kind: "gap".into(), message: format!("Geometry jumps {:.1} km between two points; treated as separate segments.", d / 1000.0) });
                    if cur.pts.len() >= 2 {
                        out.push(cur);
                    }
                    cur = Seg { pts: vec![], ele: vec![], cum: vec![] };
                } else {
                    let c = cur.cum.last().copied().unwrap_or(0.0) + d;
                    cur.pts.push(ll);
                    cur.ele.push(p.ele);
                    cur.cum.push(c);
                    continue;
                }
            }
            cur.pts.push(ll);
            cur.ele.push(p.ele);
            cur.cum.push(0.0);
        }
        if cur.pts.len() >= 2 {
            out.push(cur);
        }
    }
    out
}

/// Fill short missing-elevation runs by linear interpolation in distance.
fn fill_elevation(seg: &mut Seg, max_fill_m: f64) {
    let n = seg.ele.len();
    let known: Vec<usize> = (0..n).filter(|&i| seg.ele[i].is_some()).collect();
    if known.is_empty() {
        return;
    }
    let (first, last) = (known[0], *known.last().unwrap());
    for i in 0..first {
        if seg.cum[first] - seg.cum[i] <= max_fill_m / 4.0 {
            seg.ele[i] = seg.ele[first];
        }
    }
    for i in last + 1..n {
        if seg.cum[i] - seg.cum[last] <= max_fill_m / 4.0 {
            seg.ele[i] = seg.ele[last];
        }
    }
    for w in known.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b == a + 1 || seg.cum[b] - seg.cum[a] > max_fill_m {
            continue;
        }
        let (ea, eb) = (seg.ele[a].unwrap(), seg.ele[b].unwrap());
        for i in a + 1..b {
            let f = (seg.cum[i] - seg.cum[a]) / (seg.cum[b] - seg.cum[a]);
            seg.ele[i] = Some(ea + (eb - ea) * f);
        }
    }
}

fn interp_seg(seg: &Seg, s: f64) -> (LatLon, Option<f64>) {
    let i = match seg.cum.binary_search_by(|c| c.partial_cmp(&s).unwrap_or(std::cmp::Ordering::Less)) {
        Ok(i) => return (seg.pts[i], seg.ele[i]),
        Err(i) => i,
    };
    if i == 0 {
        return (seg.pts[0], seg.ele[0]);
    }
    if i >= seg.pts.len() {
        let l = seg.pts.len() - 1;
        return (seg.pts[l], seg.ele[l]);
    }
    let (a, b) = (i - 1, i);
    let f = (s - seg.cum[a]) / (seg.cum[b] - seg.cum[a]).max(1e-9);
    let e = match (seg.ele[a], seg.ele[b]) {
        (Some(x), Some(y)) => Some(x + (y - x) * f),
        _ => None,
    };
    (lerp(seg.pts[a], seg.pts[b], f), e)
}

pub fn process(input: &RouteInput, cfg: &ProfileConfig) -> Result<RouteProfile, String> {
    let mut flags = Vec::new();
    let mut dups = 0u32;
    let mut segs = prepare_segments(input.segments, cfg, &mut dups, &mut flags);
    if segs.is_empty() {
        return Err("The route needs at least two distinct points.".into());
    }
    let raw_points: usize = segs.iter().map(|s| s.ele.len()).sum();
    let raw_with_ele: usize = segs.iter().map(|s| s.ele.iter().filter(|e| e.is_some()).count()).sum();
    let total_len: f64 = segs.iter().map(|s| *s.cum.last().unwrap()).sum();
    if total_len > 2_000_000.0 {
        return Err("Routes longer than 2,000 km are not supported.".into());
    }
    for s in segs.iter_mut() {
        fill_elevation(s, cfg.max_fill_m);
    }
    // Resample.
    let mut samples: Vec<Sample> = Vec::new();
    let mut seg_starts = Vec::new();
    let mut offset = 0.0;
    for (k, seg) in segs.iter().enumerate() {
        let len = *seg.cum.last().unwrap();
        seg_starts.push(offset);
        let n = (len / cfg.step_m).floor() as usize;
        let mut locals: Vec<f64> = (0..=n).map(|i| i as f64 * cfg.step_m).collect();
        if len - locals.last().copied().unwrap_or(0.0) > 0.5 {
            locals.push(len);
        }
        for sl in locals {
            let (ll, e) = interp_seg(seg, sl);
            samples.push(Sample { s: offset + sl, lat: ll.lat, lon: ll.lon, ele: e, grade: None, seg: k as u32 });
        }
        offset += len;
    }
    let total_m = offset;
    // Corrections (curated, e.g. bridges/tunnels).
    for c in input.corrections {
        if !(c.from_m.is_finite() && c.to_m.is_finite()) || c.to_m <= c.from_m {
            continue;
        }
        let idx: Vec<usize> = (0..samples.len()).filter(|&i| samples[i].s >= c.from_m && samples[i].s <= c.to_m).collect();
        if idx.len() < 2 {
            continue;
        }
        let (a, b) = (idx[0], *idx.last().unwrap());
        if let (Some(ea), Some(eb)) = (samples[a].ele, samples[b].ele) {
            for &i in &idx {
                let f = (samples[i].s - samples[a].s) / (samples[b].s - samples[a].s).max(1e-9);
                samples[i].ele = Some(ea + (eb - ea) * f);
            }
        }
    }
    // Spike removal (Hampel) per segment.
    let mut spikes = 0u32;
    let mut spike_positions = Vec::new();
    let ele_raw: Vec<Option<f64>> = samples.iter().map(|s| s.ele).collect();
    for i in 0..samples.len() {
        let Some(x) = ele_raw[i] else { continue };
        let lo = i.saturating_sub(cfg.spike_half_window);
        let hi = (i + cfg.spike_half_window).min(samples.len() - 1);
        let mut win: Vec<f64> = (lo..=hi).filter(|&j| samples[j].seg == samples[i].seg).filter_map(|j| ele_raw[j]).collect();
        if win.len() < 5 {
            continue;
        }
        let m = median(&mut win);
        let mut dev: Vec<f64> = win.iter().map(|v| (v - m).abs()).collect();
        let mad = median(&mut dev);
        let thr = (cfg.spike_k * 1.4826 * mad).max(cfg.spike_min_m);
        if (x - m).abs() > thr {
            samples[i].ele = Some(m);
            spikes += 1;
            spike_positions.push(samples[i].s);
        }
    }
    if spikes > 0 {
        flags.push(Flag { from_m: spike_positions[0], to_m: *spike_positions.last().unwrap(), kind: "spikes".into(), message: format!("{spikes} isolated elevation spike(s) were removed.") });
    }
    // Smoothing: centered moving average within the segment.
    let half = ((cfg.smooth_window_m / cfg.step_m) / 2.0).floor() as usize;
    if half > 0 {
        let src: Vec<Option<f64>> = samples.iter().map(|s| s.ele).collect();
        for i in 0..samples.len() {
            if src[i].is_none() {
                continue;
            }
            let lo = i.saturating_sub(half);
            let hi = (i + half).min(samples.len() - 1);
            let vals: Vec<f64> = (lo..=hi).filter(|&j| samples[j].seg == samples[i].seg).filter_map(|j| src[j]).collect();
            if !vals.is_empty() {
                samples[i].ele = Some(vals.iter().sum::<f64>() / vals.len() as f64);
            }
        }
    }
    // Grade over a centered distance window (one-sided at segment ends).
    let d = cfg.grade_window_m;
    let mut seg_ranges: Vec<(usize, usize)> = Vec::new();
    for i in 0..samples.len() {
        if i == 0 || samples[i].seg != samples[i - 1].seg {
            seg_ranges.push((i, i));
        }
        seg_ranges.last_mut().unwrap().1 = i;
    }
    let ele_at = |a: usize, b: usize, s: f64| -> Option<f64> {
        let sl = &samples[a..=b];
        let j = sl.partition_point(|x| x.s < s);
        if j == 0 {
            return sl[0].ele;
        }
        if j >= sl.len() {
            return sl[sl.len() - 1].ele;
        }
        let (p, q) = (&sl[j - 1], &sl[j]);
        let f = (s - p.s) / (q.s - p.s).max(1e-9);
        Some(p.ele? + (q.ele? - p.ele?) * f)
    };
    let mut grades = vec![None; samples.len()];
    for &(a, b) in &seg_ranges {
        let (s0, s1) = (samples[a].s, samples[b].s);
        for (i, g) in grades.iter_mut().enumerate().take(b + 1).skip(a) {
            let s = samples[i].s;
            let (mut lo, mut hi) = (s - d / 2.0, s + d / 2.0);
            if lo < s0 {
                lo = s0;
                hi = (s0 + d).min(s1);
            }
            if hi > s1 {
                hi = s1;
                lo = (s1 - d).max(s0);
            }
            if hi - lo < 1.0 {
                *g = Some(0.0);
                continue;
            }
            if let (Some(h1), Some(h2)) = (ele_at(a, b, lo), ele_at(a, b, hi)) {
                if samples[i].ele.is_some() {
                    *g = Some(100.0 * (h2 - h1) / (hi - lo));
                }
            }
        }
    }
    for (i, g) in grades.into_iter().enumerate() {
        samples[i].grade = g;
    }
    // Flags: missing elevation runs, steep sections, abrupt changes.
    let mut run_start: Option<f64> = None;
    let mut missing_m = 0.0;
    for i in 0..samples.len() {
        let missing = samples[i].grade.is_none();
        if missing && run_start.is_none() {
            run_start = Some(samples[i].s);
        }
        if run_start.is_some() && (!missing || i == samples.len() - 1) {
            let from = run_start.take().unwrap();
            let to = samples[i].s;
            missing_m += (to - from).max(cfg.step_m);
            flags.push(Flag { from_m: from, to_m: to, kind: "missing".into(), message: "No elevation data. Road simulation is blocked here unless you choose the flat fallback.".into() });
        }
    }
    let mut steep_start: Option<f64> = None;
    for i in 0..samples.len() {
        let steep = samples[i].grade.map(|g| g.abs() > cfg.steep_flag_pct).unwrap_or(false);
        if steep && steep_start.is_none() {
            steep_start = Some(samples[i].s);
        }
        if steep_start.is_some() && (!steep || i == samples.len() - 1) {
            let from = steep_start.take().unwrap();
            flags.push(Flag {
                from_m: from,
                to_m: samples[i].s,
                kind: "steep".into(),
                message: format!("Gradient beyond ±{}%: possibly real, possibly a data error. Trainer commands are limited regardless.", cfg.steep_flag_pct),
            });
        }
    }
    let mut abrupt = 0;
    for i in 1..samples.len() {
        if samples[i].seg != samples[i - 1].seg {
            continue;
        }
        if let (Some(a), Some(b)) = (samples[i - 1].grade, samples[i].grade) {
            if (b - a).abs() > cfg.abrupt_change_pct && abrupt < 50 {
                abrupt += 1;
                flags.push(Flag { from_m: samples[i - 1].s, to_m: samples[i].s, kind: "abrupt".into(), message: "Abrupt gradient change: check for a bridge, tunnel or data artefact.".into() });
            }
        }
    }
    if input.source.kind == "dem" {
        flags.push(Flag {
            from_m: 0.0,
            to_m: total_m,
            kind: "estimated".into(),
            message: format!(
                "Elevation is estimated from a terrain model ({}, ~{:.0} m resolution). Terrain is not always the road surface: bridges, tunnels and cuttings may show false gradients.",
                input.source.dataset, input.source.resolution_m
            ),
        });
    }
    if input.source.kind == "synthetic" {
        flags.push(Flag { from_m: 0.0, to_m: total_m, kind: "synthetic".into(), message: "Synthetic demo profile: generated elevation, not real terrain.".into() });
    }
    // Statistics from the processed profile (0.5 m hysteresis).
    let (mut asc, mut desc) = (0.0, 0.0);
    let mut ref_e: Option<f64> = None;
    let mut ref_seg = u32::MAX;
    for s in &samples {
        let Some(e) = s.ele else { continue };
        if s.seg != ref_seg {
            ref_e = Some(e);
            ref_seg = s.seg;
            continue;
        }
        let r = ref_e.unwrap();
        if e - r >= 0.5 {
            asc += e - r;
            ref_e = Some(e);
        } else if r - e >= 0.5 {
            desc += r - e;
            ref_e = Some(e);
        }
    }
    let gs: Vec<f64> = samples.iter().filter_map(|s| s.grade).collect();
    let es: Vec<f64> = samples.iter().filter_map(|s| s.ele).collect();
    let buckets: [(&str, f64, f64); 8] = [
        ("< -8%", f64::NEG_INFINITY, -8.0),
        ("-8 to -4%", -8.0, -4.0),
        ("-4 to -1%", -4.0, -1.0),
        ("-1 to 1%", -1.0, 1.0),
        ("1 to 4%", 1.0, 4.0),
        ("4 to 8%", 4.0, 8.0),
        ("8 to 12%", 8.0, 12.0),
        ("> 12%", 12.0, f64::INFINITY),
    ];
    let mut hist: Vec<(String, f64)> = buckets.iter().map(|(n, _, _)| (n.to_string(), 0.0)).collect();
    for w in samples.windows(2) {
        if w[0].seg != w[1].seg {
            continue;
        }
        if let Some(g) = w[0].grade {
            let len = w[1].s - w[0].s;
            if let Some(k) = buckets.iter().position(|(_, lo, hi)| g >= *lo && g < *hi) {
                hist[k].1 += len;
            }
        }
    }
    let coverage = if raw_points == 0 { 0.0 } else { 100.0 * raw_with_ele as f64 / raw_points as f64 };
    let complete = samples.iter().all(|s| s.grade.is_some());
    let (ready, blocked) = if complete || input.flat_fallback {
        (true, None)
    } else {
        (false, Some(format!("{:.1} km of the route has no elevation. Fetch elevation data or explicitly choose the flat fallback before riding with road simulation.", missing_m / 1000.0)))
    };
    if !complete && input.flat_fallback {
        flags.push(Flag { from_m: 0.0, to_m: total_m, kind: "flat_fallback".into(), message: "Flat fallback chosen: sections without elevation are ridden at 0% and labelled as such.".into() });
    }
    Ok(RouteProfile {
        version: PROFILE_VERSION,
        config: cfg.clone(),
        total_m,
        samples,
        segment_starts_m: seg_starts,
        ascent_m: asc,
        descent_m: desc,
        max_grade: gs.iter().copied().fold(f64::NEG_INFINITY, f64::max).max(0.0),
        min_grade: gs.iter().copied().fold(f64::INFINITY, f64::min).min(0.0),
        ele_min: es.iter().copied().reduce(f64::min),
        ele_max: es.iter().copied().reduce(f64::max),
        histogram: hist,
        source: input.source.clone(),
        coverage_pct: coverage,
        spikes_removed: spikes,
        duplicates_removed: dups,
        flags,
        flat_fallback: input.flat_fallback && !complete,
        simulation_ready: ready,
        blocked_reason: blocked,
    })
}

/// State of the route at a distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
    /// Processed road grade, percent. `None` = unknown (no elevation).
    pub grade: Option<f64>,
    pub seg: u32,
}

impl RouteProfile {
    pub fn at(&self, s: f64) -> RoutePoint {
        let n = self.samples.len();
        let s = if s.is_finite() { s.clamp(0.0, self.total_m) } else { 0.0 };
        let j = self.samples.partition_point(|x| x.s <= s);
        let pick = |a: &Sample| RoutePoint { lat: a.lat, lon: a.lon, ele: a.ele, grade: a.grade, seg: a.seg };
        if j == 0 {
            return pick(&self.samples[0]);
        }
        if j >= n {
            return pick(&self.samples[n - 1]);
        }
        let (a, b) = (&self.samples[j - 1], &self.samples[j]);
        if a.seg != b.seg {
            return pick(b);
        }
        let f = (s - a.s) / (b.s - a.s).max(1e-9);
        let ll = lerp(LatLon { lat: a.lat, lon: a.lon }, LatLon { lat: b.lat, lon: b.lon }, f);
        let ele = match (a.ele, b.ele) {
            (Some(x), Some(y)) => Some(x + (y - x) * f),
            _ => None,
        };
        let grade = match (a.grade, b.grade) {
            (Some(x), Some(y)) => Some(x + (y - x) * f),
            _ => None,
        };
        RoutePoint { lat: ll.lat, lon: ll.lon, ele, grade, seg: a.seg }
    }

    /// Grade used for simulation at `s`: processed grade, or 0 under an
    /// explicitly accepted flat fallback, or `None` (blocked).
    pub fn sim_grade(&self, s: f64) -> (Option<f64>, bool) {
        match self.at(s).grade {
            Some(g) => (Some(g), false),
            None if self.flat_fallback => (Some(0.0), true),
            None => (None, false),
        }
    }

    pub fn summary_json(&self) -> Value {
        Value::obj([
            ("version", self.version.into()),
            ("total_m", self.total_m.into()),
            ("ascent_m", self.ascent_m.into()),
            ("descent_m", self.descent_m.into()),
            ("max_grade", self.max_grade.into()),
            ("min_grade", self.min_grade.into()),
            ("ele_min", self.ele_min.into()),
            ("ele_max", self.ele_max.into()),
            ("histogram", Value::Arr(self.histogram.iter().map(|(k, v)| Value::Arr(vec![k.clone().into(), (*v).into()])).collect())),
            ("source", self.source.to_json()),
            ("coverage_pct", self.coverage_pct.into()),
            ("spikes_removed", self.spikes_removed.into()),
            ("duplicates_removed", self.duplicates_removed.into()),
            ("segments", self.segment_starts_m.len().into()),
            ("segment_starts_m", self.segment_starts_m.clone().into()),
            ("flags", self.flags.to_json()),
            ("flat_fallback", self.flat_fallback.into()),
            ("simulation_ready", self.simulation_ready.into()),
            ("blocked_reason", self.blocked_reason.clone().into()),
            ("config", self.config.to_json()),
        ])
    }

    /// Downsampled arrays for charts and maps: [s, ele, grade, seg] and [lon, lat, seg].
    pub fn display_json(&self, max_points: usize) -> Value {
        let n = self.samples.len();
        let stride = n.div_ceil(max_points.max(2)).max(1);
        let mut chart = Vec::new();
        let mut line = Vec::new();
        for (i, s) in self.samples.iter().enumerate() {
            let keep = i % stride == 0 || i == n - 1 || (i > 0 && self.samples[i - 1].seg != s.seg) || (i + 1 < n && self.samples[i + 1].seg != s.seg);
            if keep {
                chart.push(Value::Arr(vec![(s.s.round()).into(), s.ele.map(|e| (e * 10.0).round() / 10.0).into(), s.grade.map(|g| (g * 100.0).round() / 100.0).into(), s.seg.into()]));
                line.push(Value::Arr(vec![((s.lon * 1e6).round() / 1e6).into(), ((s.lat * 1e6).round() / 1e6).into(), s.seg.into()]));
            }
        }
        Value::obj([("chart", Value::Arr(chart)), ("line", Value::Arr(line))])
    }
}

/// Reverse geometry for riding a route backwards.
pub fn reverse_segments(segs: &[Vec<RawPoint>]) -> Vec<Vec<RawPoint>> {
    segs.iter().rev().map(|s| s.iter().rev().cloned().collect()).collect()
}
pub fn reverse_corrections(cs: &[Correction], total_m: f64) -> Vec<Correction> {
    cs.iter().map(|c| Correction { from_m: total_m - c.to_m, to_m: total_m - c.from_m, kind: c.kind.clone(), note: c.note.clone() }).collect()
}

/// Points along the geometry spaced ~`spacing_m` apart, for sampling a
/// terrain model.
pub fn densify(segs: &[Vec<RawPoint>], spacing_m: f64) -> Vec<Vec<RawPoint>> {
    let mut out = Vec::new();
    for seg in segs {
        let mut o: Vec<RawPoint> = Vec::new();
        for w in seg.windows(2) {
            let (a, b) = (LatLon { lat: w[0].lat, lon: w[0].lon }, LatLon { lat: w[1].lat, lon: w[1].lon });
            let d = haversine_m(a, b);
            let n = (d / spacing_m).ceil().clamp(1.0, 10_000.0) as usize;
            for k in 0..n {
                let p = lerp(a, b, k as f64 / n as f64);
                o.push(RawPoint { lat: p.lat, lon: p.lon, ele: None });
            }
        }
        if let Some(l) = seg.last() {
            o.push(RawPoint { lat: l.lat, lon: l.lon, ele: None });
        }
        out.push(o);
    }
    out
}

// ------------------------------------------------------------ synthetic routes

/// Build a synthetic route from (length_m, grade_pct) legs heading east from
/// a start point, optionally bending into a loop shape. Elevation is exact.
pub fn synthetic(start: LatLon, legs: &[(f64, f64)], loop_shape: bool) -> Vec<RawPoint> {
    let total: f64 = legs.iter().map(|l| l.0).sum();
    let mut pts = Vec::new();
    let mut s = 0.0;
    let mut ele = 50.0;
    let step = 10.0;
    let pos = |s: f64| -> (f64, f64) {
        if loop_shape {
            let theta = 2.0 * std::f64::consts::PI * s / total;
            let r = total / (2.0 * std::f64::consts::PI);
            (r * theta.sin(), r * (1.0 - theta.cos()))
        } else {
            (s, 0.0)
        }
    };
    for &(len, g) in legs {
        let n = (len / step).round() as usize;
        for _ in 0..n {
            let (dx, dy) = pos(s);
            pts.push(RawPoint { lat: start.lat + dy / 111_195.0, lon: start.lon + dx / (111_195.0 * start.lat.to_radians().cos()), ele: Some(ele) });
            // Horizontal step `step` meters (along the path); elevation change from grade.
            ele += g / 100.0 * step;
            s += step;
        }
    }
    let (dx, dy) = if loop_shape { (0.0, 0.0) } else { (s, 0.0) };
    let end = RawPoint { lat: start.lat + dy / 111_195.0, lon: start.lon + dx / (111_195.0 * start.lat.to_radians().cos()), ele: Some(ele) };
    pts.push(end);
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_steps_produce_signed_grades() {
        // A05 profile: flat, +5 %, flat, -3 %, flat.
        let pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(1000.0, 0.0), (1000.0, 5.0), (500.0, 0.0), (1000.0, -3.0), (500.0, 0.0)], false);
        let segs = vec![pts];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("synthetic"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!((p.total_m - 4000.0).abs() < 5.0, "{}", p.total_m);
        let g = |s: f64| p.at(s).grade.unwrap();
        assert!(g(500.0).abs() < 0.05, "{}", g(500.0));
        assert!((g(1500.0) - 5.0).abs() < 0.1, "{}", g(1500.0));
        assert!(g(2250.0).abs() < 0.05);
        assert!((g(3000.0) + 3.0).abs() < 0.1, "{}", g(3000.0));
        assert!((p.ascent_m - 50.0).abs() < 2.0, "{}", p.ascent_m);
        assert!((p.descent_m - 30.0).abs() < 2.0, "{}", p.descent_m);
        assert!(p.simulation_ready);
    }

    #[test]
    fn spikes_duplicates_missing_and_reverse() {
        let mut pts = synthetic(LatLon { lat: 47.0, lon: -52.0 }, &[(2000.0, 2.0)], false);
        pts[100].ele = Some(pts[100].ele.unwrap() + 80.0); // isolated spike
        let dup = pts[50].clone();
        pts.insert(51, dup); // duplicate point
        let segs = vec![pts.clone()];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("imported"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!(p.spikes_removed >= 1);
        assert_eq!(p.duplicates_removed, 1);
        assert!(p.samples.iter().all(|s| s.grade.unwrap().abs() < 4.0), "spike must not create a steep grade");
        // Missing elevation over a long stretch blocks simulation.
        let mut gap = pts.clone();
        for p in gap.iter_mut().skip(40).take(80) {
            p.ele = None;
        }
        let segs = vec![gap];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("imported"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!(!p.simulation_ready);
        assert!(p.blocked_reason.is_some());
        assert_eq!(p.sim_grade(800.0), (None, false));
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("imported"), corrections: &[], flat_fallback: true }, &ProfileConfig::default()).unwrap();
        assert!(p.simulation_ready);
        assert_eq!(p.sim_grade(800.0), (Some(0.0), true));
        // Reversed route: grade sign flips.
        let fwd = vec![synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(1000.0, 4.0)], false)];
        let rev = reverse_segments(&fwd);
        let p = process(&RouteInput { segments: &rev, source: ElevationSource::of("synthetic"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!((p.at(500.0).grade.unwrap() + 4.0).abs() < 0.1);
    }

    #[test]
    fn segments_are_not_bridged() {
        let a = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(500.0, 0.0)], false);
        let b = synthetic(LatLon { lat: 0.1, lon: 0.1 }, &[(500.0, 0.0)], false); // ~15 km away
        let segs = vec![a, b];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("synthetic"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert_eq!(p.segment_starts_m.len(), 2);
        assert!((p.total_m - 1000.0).abs() < 2.0, "no distance across the gap: {}", p.total_m);
        assert!(p.samples.iter().all(|s| s.grade.unwrap().abs() < 1.0));
    }

    #[test]
    fn bridge_correction_flattens_dip() {
        let mut pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(1000.0, 0.0)], false);
        for (i, p) in pts.iter_mut().enumerate() {
            if (40..60).contains(&i) {
                p.ele = Some(p.ele.unwrap() - 15.0); // DEM shows the river valley under a bridge
            }
        }
        let segs = vec![pts];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("dem"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!(p.flags.iter().any(|f| f.kind == "abrupt" || f.kind == "steep"));
        let c = vec![Correction { from_m: 350.0, to_m: 650.0, kind: "linear".into(), note: "bridge".into() }];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("dem"), corrections: &c, flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!(p.samples.iter().all(|s| s.grade.unwrap().abs() < 0.5));
        assert!(p.flags.iter().any(|f| f.kind == "estimated"));
    }

    #[test]
    fn dateline_crossing() {
        let segs = vec![vec![RawPoint { lat: 0.0, lon: 179.9995, ele: Some(0.0) }, RawPoint { lat: 0.0, lon: -179.9995, ele: Some(0.0) }, RawPoint { lat: 0.0, lon: -179.9985, ele: Some(1.0) }]];
        let p = process(&RouteInput { segments: &segs, source: ElevationSource::of("imported"), corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap();
        assert!(p.total_m < 300.0, "{}", p.total_m);
    }
}
