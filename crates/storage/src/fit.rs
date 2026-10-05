//! FIT activity encoder (Garmin FIT protocol 2.0, little-endian).
//!
//! Message and field numbers were checked against the Garmin FIT SDK profile
//! (garmin/fit-javascript-sdk `profile.js`). Exports are verified with the
//! independent Garmin decoder in CI (scripts/verify-fit.mjs).
//!
//! Contents: file_id, timer start/stop events (pauses), 1 Hz records,
//! laps, one session (sport cycling; sub-sport indoor_cycling or
//! virtual_activity) and the activity message.

use rl_session::record::{Lap, Sample, SessionEvent, Summary};

/// Seconds between the Unix epoch and the FIT epoch (1989-12-31T00:00:00Z).
pub const FIT_EPOCH_OFFSET_S: i64 = 631_065_600;

const BT_ENUM: u8 = 0x00;
const BT_UINT8: u8 = 0x02;
const BT_SINT16: u8 = 0x83;
const BT_UINT16: u8 = 0x84;
const BT_SINT32: u8 = 0x85;
const BT_UINT32: u8 = 0x86;
const BT_UINT32Z: u8 = 0x8C;

const CRC_TABLE: [u16; 16] = [0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800, 0xB401, 0x5000, 0x9C01, 0x8801, 0x4400];

pub fn crc16(mut crc: u16, data: &[u8]) -> u16 {
    for &b in data {
        let mut tmp = CRC_TABLE[(crc & 0xF) as usize];
        crc = (crc >> 4) & 0x0FFF;
        crc = crc ^ tmp ^ CRC_TABLE[(b & 0xF) as usize];
        tmp = CRC_TABLE[(crc & 0xF) as usize];
        crc = (crc >> 4) & 0x0FFF;
        crc = crc ^ tmp ^ CRC_TABLE[((b >> 4) & 0xF) as usize];
    }
    crc
}

pub fn fit_time(utc_ms: i64) -> u32 {
    ((utc_ms / 1000) - FIT_EPOCH_OFFSET_S).clamp(0, u32::MAX as i64 - 1) as u32
}

#[derive(Clone, Copy)]
enum V {
    U8(Option<u8>),
    E(Option<u8>),
    U16(Option<u16>),
    S16(Option<i16>),
    U32(Option<u32>),
    U32Z(Option<u32>),
    S32(Option<i32>),
}

impl V {
    fn base(&self) -> (u8, u8) {
        match self {
            V::U8(_) => (BT_UINT8, 1),
            V::E(_) => (BT_ENUM, 1),
            V::U16(_) => (BT_UINT16, 2),
            V::S16(_) => (BT_SINT16, 2),
            V::U32(_) => (BT_UINT32, 4),
            V::U32Z(_) => (BT_UINT32Z, 4),
            V::S32(_) => (BT_SINT32, 4),
        }
    }
    fn write(&self, out: &mut Vec<u8>) {
        match *self {
            V::U8(v) | V::E(v) => out.push(v.unwrap_or(0xFF)),
            V::U16(v) => out.extend_from_slice(&v.unwrap_or(0xFFFF).to_le_bytes()),
            V::S16(v) => out.extend_from_slice(&v.unwrap_or(0x7FFF).to_le_bytes()),
            V::U32(v) => out.extend_from_slice(&v.unwrap_or(0xFFFF_FFFF).to_le_bytes()),
            V::U32Z(v) => out.extend_from_slice(&v.unwrap_or(0).to_le_bytes()),
            V::S32(v) => out.extend_from_slice(&v.unwrap_or(0x7FFF_FFFF).to_le_bytes()),
        }
    }
}

struct Writer {
    data: Vec<u8>,
    /// (global message, field-number signature) for each local type 0..15.
    defs: Vec<(u16, Vec<(u8, u8, u8)>)>,
}

impl Writer {
    fn msg(&mut self, global: u16, fields: &[(u8, V)]) {
        let sig: Vec<(u8, u8, u8)> = fields.iter().map(|(n, v)| (*n, v.base().1, v.base().0)).collect();
        let local = match self.defs.iter().position(|(g, s)| *g == global && *s == sig) {
            Some(i) => i as u8,
            None => {
                let i = if self.defs.len() < 16 {
                    self.defs.push((global, sig.clone()));
                    self.defs.len() - 1
                } else {
                    // Reuse slot 15 when all local types are taken.
                    self.defs[15] = (global, sig.clone());
                    15
                };
                self.data.push(0x40 | i as u8);
                self.data.push(0); // reserved
                self.data.push(0); // little endian
                self.data.extend_from_slice(&global.to_le_bytes());
                self.data.push(sig.len() as u8);
                for (n, size, bt) in &sig {
                    self.data.extend_from_slice(&[*n, *size, *bt]);
                }
                i as u8
            }
        };
        self.data.push(local & 0x0F);
        for (_, v) in fields {
            v.write(&mut self.data);
        }
    }
}

fn u8f(v: Option<f64>) -> Option<u8> {
    v.filter(|x| x.is_finite() && *x >= 0.0 && *x < 255.0).map(|x| x.round() as u8)
}
fn u16f(v: Option<f64>, scale: f64) -> Option<u16> {
    v.map(|x| x * scale).filter(|x| x.is_finite() && *x >= 0.0 && *x < 65535.0).map(|x| x.round() as u16)
}
fn u32f(v: Option<f64>, scale: f64, offset: f64) -> Option<u32> {
    v.map(|x| (x + offset) * scale).filter(|x| x.is_finite() && *x >= 0.0 && *x < 4.29e9).map(|x| x.round() as u32)
}
fn semicircles(deg: Option<f64>) -> Option<i32> {
    deg.filter(|d| d.is_finite() && d.abs() <= 180.0).map(|d| (d * (2f64.powi(31) / 180.0)).round().clamp(i32::MIN as f64 + 1.0, i32::MAX as f64 - 1.0) as i32)
}

pub struct FitInput<'a> {
    pub start_utc_ms: i64,
    pub tz_offset_min: i32,
    pub virtual_route: bool,
    pub samples: &'a [Sample],
    pub laps: &'a [Lap],
    pub events: &'a [SessionEvent],
    pub summary: &'a Summary,
}

pub fn encode_activity(inp: &FitInput) -> Vec<u8> {
    let mut w = Writer { data: Vec::new(), defs: Vec::new() };
    let start = fit_time(inp.start_utc_ms);
    let end_utc = inp.samples.last().map(|s| s.utc_ms).unwrap_or(inp.start_utc_ms).max(inp.start_utc_ms);
    let end = fit_time(end_utc);
    let sub_sport = if inp.virtual_route { 58 } else { 6 };
    // file_id
    w.msg(0, &[(0, V::E(Some(4))), (1, V::U16(Some(255))), (2, V::U16(Some(1))), (3, V::U32Z(Some(0x5249_4447))), (4, V::U32(Some(start)))]);
    // timer start
    w.msg(21, &[(253, V::U32(Some(start))), (0, V::E(Some(0))), (1, V::E(Some(0))), (4, V::U8(Some(0)))]);
    // Records interleaved with pause/resume events in time order.
    let mut ev_iter = inp.events.iter().filter(|e| e.kind == "pause" || e.kind == "resume").peekable();
    for s in inp.samples {
        while let Some(e) = ev_iter.peek() {
            if e.utc_ms <= s.utc_ms {
                let et = if e.kind == "pause" { 4 } else { 0 };
                w.msg(21, &[(253, V::U32(Some(fit_time(e.utc_ms)))), (0, V::E(Some(0))), (1, V::E(Some(et))), (4, V::U8(Some(0)))]);
                ev_iter.next();
            } else {
                break;
            }
        }
        let mut f: Vec<(u8, V)> = vec![(253, V::U32(Some(fit_time(s.utc_ms))))];
        if inp.virtual_route {
            f.push((0, V::S32(semicircles(s.lat))));
            f.push((1, V::S32(semicircles(s.lon))));
            f.push((78, V::U32(u32f(s.ele, 5.0, 500.0))));
            f.push((9, V::S16(s.grade.filter(|g| g.abs() < 300.0).map(|g| (g * 100.0).round() as i16))));
        }
        f.push((3, V::U8(u8f(s.hr))));
        f.push((4, V::U8(u8f(s.cadence))));
        f.push((5, V::U32(u32f(s.distance, 100.0, 0.0))));
        f.push((7, V::U16(u16f(s.power.map(|p| p.max(0.0)), 1.0))));
        f.push((73, V::U32(u32f(s.speed, 1000.0, 0.0))));
        w.msg(20, &f);
    }
    for e in ev_iter {
        let et = if e.kind == "pause" { 4 } else { 0 };
        w.msg(21, &[(253, V::U32(Some(fit_time(e.utc_ms)))), (0, V::E(Some(0))), (1, V::E(Some(et))), (4, V::U8(Some(0)))]);
    }
    // timer stop
    w.msg(21, &[(253, V::U32(Some(end))), (0, V::E(Some(0))), (1, V::E(Some(4))), (4, V::U8(Some(0)))]);
    // laps
    let laps = &inp.summary.laps;
    for (i, l) in inp.laps.iter().enumerate() {
        let ls = laps.iter().find(|x| x.index == l.index);
        let start_t = fit_time(l.start_utc_ms.max(inp.start_utc_ms));
        let end_t = fit_time(inp.start_utc_ms + l.end_t_ms as i64).max(start_t);
        let trig = match l.trigger.as_str() {
            "session_end" => 7,
            "step" => 1,
            _ => 0,
        };
        w.msg(
            19,
            &[
                (253, V::U32(Some(end_t))),
                (254, V::U16(Some(i as u16))),
                (0, V::E(Some(9))),
                (1, V::E(Some(1))),
                (2, V::U32(Some(start_t))),
                (7, V::U32(Some((l.end_t_ms.saturating_sub(l.start_t_ms)) as u32))),
                (8, V::U32(Some(l.end_active_s.saturating_sub(l.start_active_s) * 1000))),
                (9, V::U32(u32f(ls.and_then(|x| x.distance_m), 100.0, 0.0))),
                (15, V::U8(u8f(ls.and_then(|x| x.hr.avg)))),
                (16, V::U8(u8f(ls.and_then(|x| x.hr.max)))),
                (17, V::U8(u8f(ls.and_then(|x| x.cadence.avg)))),
                (18, V::U8(u8f(ls.and_then(|x| x.cadence.max)))),
                (19, V::U16(u16f(ls.and_then(|x| x.power.avg), 1.0))),
                (20, V::U16(u16f(ls.and_then(|x| x.power.max), 1.0))),
                (24, V::E(Some(trig))),
                (25, V::E(Some(2))),
                (39, V::E(Some(sub_sport))),
            ],
        );
    }
    let sm = inp.summary;
    w.msg(
        18,
        &[
            (253, V::U32(Some(end))),
            (254, V::U16(Some(0))),
            (0, V::E(Some(8))),
            (1, V::E(Some(1))),
            (2, V::U32(Some(start))),
            (5, V::E(Some(2))),
            (6, V::E(Some(sub_sport))),
            (7, V::U32(u32f(Some(sm.elapsed_s), 1000.0, 0.0))),
            (8, V::U32(u32f(Some(sm.timer_s), 1000.0, 0.0))),
            (9, V::U32(u32f(sm.distance_m, 100.0, 0.0))),
            (11, V::U16(u16f(sm.energy_kcal_estimate, 1.0))),
            (14, V::U16(u16f(sm.speed.avg, 1000.0))),
            (15, V::U16(u16f(sm.speed.max, 1000.0))),
            (16, V::U8(u8f(sm.hr.avg))),
            (17, V::U8(u8f(sm.hr.max))),
            (18, V::U8(u8f(sm.cadence.avg))),
            (19, V::U8(u8f(sm.cadence.max))),
            (20, V::U16(u16f(sm.power.avg, 1.0))),
            (21, V::U16(u16f(sm.power.max, 1.0))),
            (22, V::U16(u16f(sm.ascent_m, 1.0))),
            (25, V::U16(Some(0))),
            (26, V::U16(Some(inp.laps.len() as u16))),
            (28, V::E(Some(0))),
        ],
    );
    let local_ts = (end as i64 + inp.tz_offset_min as i64 * 60).clamp(0, u32::MAX as i64 - 1) as u32;
    w.msg(
        34,
        &[
            (253, V::U32(Some(end))),
            (0, V::U32(u32f(Some(sm.timer_s), 1000.0, 0.0))),
            (1, V::U16(Some(1))),
            (2, V::E(Some(0))),
            (3, V::E(Some(26))),
            (4, V::E(Some(1))),
            (5, V::U32(Some(local_ts))),
        ],
    );
    // Header + CRCs.
    let mut out = Vec::with_capacity(w.data.len() + 16);
    out.push(14);
    out.push(0x20); // protocol 2.0
    out.extend_from_slice(&2132u16.to_le_bytes()); // profile 21.32
    out.extend_from_slice(&(w.data.len() as u32).to_le_bytes());
    out.extend_from_slice(b".FIT");
    let hcrc = crc16(0, &out[..12]);
    out.extend_from_slice(&hcrc.to_le_bytes());
    out.extend_from_slice(&w.data);
    let crc = crc16(0, &out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_session::record::summarize;

    #[test]
    fn crc_reference() {
        // CRC of the bytes of an empty FIT header ".FIT" region is stable;
        // verify the algorithm on a known vector: CRC-16/ARC("123456789") = 0xBB3D.
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
    }

    #[test]
    fn structure_and_crc() {
        let samples: Vec<Sample> = (1..=120)
            .map(|i| Sample { t_ms: i * 1000, utc_ms: 1_790_000_000_000 + i as i64 * 1000, active_s: i as u32, power: Some(200.0), hr: if i > 60 { Some(140.0) } else { None }, cadence: Some(90.0), distance: Some(i as f64 * 8.0), speed: Some(8.0), ..Default::default() })
            .collect();
        let laps = vec![Lap { index: 0, start_t_ms: 0, end_t_ms: 120_000, start_active_s: 0, end_active_s: 120, start_utc_ms: 1_790_000_000_000, label: "Ride".into(), trigger: "session_end".into(), step: None, target_w: None }];
        let sum = summarize(&samples, &laps, 120.0, Some(250.0));
        let b = encode_activity(&FitInput { start_utc_ms: 1_790_000_000_000, tz_offset_min: -150, virtual_route: false, samples: &samples, laps: &laps, events: &[], summary: &sum });
        assert_eq!(&b[8..12], b".FIT");
        let n = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
        assert_eq!(b.len(), 14 + n + 2);
        assert_eq!(crc16(0, &b), 0, "file CRC must validate to zero");
        assert_eq!(crc16(0, &b[..14]), 0, "header CRC must validate to zero");
    }
}
