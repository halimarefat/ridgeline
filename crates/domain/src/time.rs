//! Calendar dates and UTC timestamps without third-party crates.
//!
//! Time policy (spec §8): intervals and control use the runtime's monotonic
//! clock; storage uses UTC milliseconds plus the rider's timezone offset
//! captured from the UI at the moment of the event. The Rust core never
//! guesses the local timezone.

use rl_json::{err, FromJson, JResult, ToJson, Value};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current wall-clock time in UTC milliseconds since the Unix epoch.
pub fn now_utc_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's algorithm).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// A calendar date in the rider's local calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    pub fn new(y: i32, m: u32, d: u32) -> Option<Date> {
        if !(1900..=2200).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(Date { y, m, d })
    }
    pub fn parse(s: &str) -> Option<Date> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !s.is_ascii() {
            return None;
        }
        let y: i32 = s[0..4].parse().ok()?;
        let m: u32 = s[5..7].parse().ok()?;
        let d: u32 = s[8..10].parse().ok()?;
        Date::new(y, m, d)
    }
    pub fn days(&self) -> i64 {
        days_from_civil(self.y, self.m, self.d)
    }
    pub fn from_days(z: i64) -> Date {
        let (y, m, d) = civil_from_days(z);
        Date { y, m, d }
    }
    pub fn add_days(&self, n: i64) -> Date {
        Date::from_days(self.days() + n)
    }
    /// Days from `self` to `other` (positive if other is later).
    pub fn days_until(&self, other: &Date) -> i64 {
        other.days() - self.days()
    }
    /// ISO weekday: 0 = Monday … 6 = Sunday.
    pub fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday (3).
        ((self.days() % 7 + 7 + 3) % 7) as u32
    }
    /// Monday of the ISO week containing this date.
    pub fn week_start(&self) -> Date {
        self.add_days(-(self.weekday() as i64))
    }
    /// Local date for a UTC instant given the rider's UTC offset in minutes
    /// (east positive, e.g. -150 for Newfoundland daylight time).
    pub fn from_utc_ms(ms: i64, offset_min: i32) -> Date {
        let local = ms + offset_min as i64 * 60_000;
        Date::from_days(local.div_euclid(86_400_000))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

impl ToJson for Date {
    fn to_json(&self) -> Value {
        Value::Str(self.to_string())
    }
}
impl FromJson for Date {
    fn from_json(v: &Value) -> JResult<Self> {
        match v.as_str().and_then(Date::parse) {
            Some(d) => Ok(d),
            None => err("expected a valid date YYYY-MM-DD"),
        }
    }
}

pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// RFC 3339 UTC timestamp with millisecond precision.
pub fn iso_utc(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let h = rem / 3_600_000;
    let mi = (rem / 60_000) % 60;
    let s = (rem / 1000) % 60;
    let milli = rem % 1000;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` into UTC ms. Used for GPX times.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 || !s.is_ascii() {
        return None;
    }
    let date = Date::parse(&s[0..10])?;
    let sep = s.as_bytes()[10];
    if sep != b'T' && sep != b' ' {
        return None;
    }
    let h: i64 = s[11..13].parse().ok()?;
    let mi: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(r) = rest.strip_prefix('.') {
        let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let frac: String = digits.chars().chain("000".chars()).take(3).collect();
        ms = frac.parse().ok()?;
        rest = &r[digits.len()..];
    }
    let offset_min: i64 = match rest {
        "" | "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let oh: i64 = o[1..3].parse().ok()?;
            let om: i64 = o[4..6].parse().ok()?;
            sign * (oh * 60 + om)
        }
        _ => return None,
    };
    Some(date.days() * 86_400_000 + h * 3_600_000 + mi * 60_000 + sec * 1000 + ms - offset_min * 60_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for z in [-800_000i64, -1, 0, 1, 19_000, 20_365, 60_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(Date::parse("2026-10-04").unwrap().weekday(), 6); // Sunday
        assert_eq!(Date::parse("2026-10-05").unwrap().weekday(), 0); // Monday
        assert_eq!(Date::parse("2024-02-29").unwrap().add_days(1).to_string(), "2024-03-01");
        assert!(Date::parse("2025-02-29").is_none());
        assert!(Date::parse("2025-13-01").is_none());
        assert_eq!(Date::parse("2026-10-08").unwrap().week_start().to_string(), "2026-10-05");
    }

    #[test]
    fn iso_and_local_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        let t = parse_iso("2026-10-05T02:06:22.411Z").unwrap();
        assert_eq!(iso_utc(t), "2026-10-05T02:06:22.411Z");
        assert_eq!(parse_iso("2026-10-04T23:36:22-02:30").unwrap(), parse_iso("2026-10-05T02:06:22Z").unwrap());
        // 02:06 UTC on Oct 5 is still Oct 4 in St. John's (UTC-2:30).
        assert_eq!(Date::from_utc_ms(t, -150).to_string(), "2026-10-04");
        assert_eq!(Date::from_utc_ms(t, 0).to_string(), "2026-10-05");
        assert!(parse_iso("garbage").is_none());
    }
}
