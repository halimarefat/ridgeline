//! Cycling Power (0x2A63), Heart Rate (0x2A37), CSC (0x2A5B) and Battery
//! (0x2A19) parsing, plus counter-based cadence/speed calculation with
//! wraparound handling.

use crate::bytes::{ParseError, Reader, Writer};

// ------------------------------------------------------------ heart rate

#[derive(Debug, Clone, PartialEq)]
pub struct HeartRateMeasurement {
    pub bpm: u16,
    /// None = sensor contact feature not supported.
    pub contact_detected: Option<bool>,
    pub energy_expended_kj: Option<u16>,
    /// RR intervals in seconds (resolution 1/1024 s).
    pub rr_s: Vec<f64>,
}

impl HeartRateMeasurement {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut r = Reader::new(b);
        let flags = r.u8()?;
        let bpm = if flags & 0x01 != 0 { r.u16()? } else { r.u8()? as u16 };
        let contact_detected = if flags & 0x04 != 0 { Some(flags & 0x02 != 0) } else { None };
        let energy_expended_kj = if flags & 0x08 != 0 { Some(r.u16()?) } else { None };
        let mut rr_s = Vec::new();
        if flags & 0x10 != 0 {
            while r.remaining() >= 2 {
                rr_s.push(r.u16()? as f64 / 1024.0);
            }
        }
        Ok(HeartRateMeasurement { bpm, contact_detected, energy_expended_kj, rr_s })
    }
    pub fn encode_simple(bpm: u8) -> Vec<u8> {
        vec![0x00, bpm]
    }
}

// ------------------------------------------------------------ cycling power

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CyclingPowerMeasurement {
    pub flags: u16,
    pub power_w: i16,
    pub pedal_balance_pct: Option<f64>,
    pub accumulated_torque_nm: Option<f64>,
    /// Cumulative wheel revolutions (uint32) and last event time in 1/2048 s.
    pub wheel: Option<(u32, u16)>,
    /// Cumulative crank revolutions (uint16) and last event time in 1/1024 s.
    pub crank: Option<(u16, u16)>,
    pub accumulated_energy_kj: Option<u16>,
}

impl CyclingPowerMeasurement {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let flags = r.u16()?;
        let power_w = r.i16()?;
        let mut m = CyclingPowerMeasurement { flags, power_w, ..Default::default() };
        if flags & (1 << 0) != 0 {
            let v = r.u8()?;
            // Pedal power balance, resolution 1/2 %.
            m.pedal_balance_pct = Some(v as f64 * 0.5);
        }
        // bit1: pedal power balance reference (no data)
        if flags & (1 << 2) != 0 {
            m.accumulated_torque_nm = Some(r.u16()? as f64 / 32.0);
        }
        // bit3: accumulated torque source (no data)
        if flags & (1 << 4) != 0 {
            m.wheel = Some((r.u32()?, r.u16()?));
        }
        if flags & (1 << 5) != 0 {
            m.crank = Some((r.u16()?, r.u16()?));
        }
        if flags & (1 << 6) != 0 {
            r.skip(4)?; // extreme force magnitudes: sint16 max, sint16 min
        }
        if flags & (1 << 7) != 0 {
            r.skip(4)?; // extreme torque magnitudes
        }
        if flags & (1 << 8) != 0 {
            r.skip(3)?; // extreme angles: 2 x uint12
        }
        if flags & (1 << 9) != 0 {
            r.skip(2)?; // top dead spot angle
        }
        if flags & (1 << 10) != 0 {
            r.skip(2)?; // bottom dead spot angle
        }
        if flags & (1 << 11) != 0 {
            m.accumulated_energy_kj = Some(r.u16()?);
        }
        Ok(m)
    }
    pub fn encode(power_w: i16, crank: Option<(u16, u16)>) -> Vec<u8> {
        let mut w = Writer::new();
        let flags: u16 = if crank.is_some() { 1 << 5 } else { 0 };
        w.u16(flags).i16(power_w);
        if let Some((rev, t)) = crank {
            w.u16(rev).u16(t);
        }
        w.done()
    }
}

// ------------------------------------------------------------ CSC

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CscMeasurement {
    /// Cumulative wheel revolutions (uint32), last event time 1/1024 s.
    pub wheel: Option<(u32, u16)>,
    /// Cumulative crank revolutions (uint16), last event time 1/1024 s.
    pub crank: Option<(u16, u16)>,
}

impl CscMeasurement {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let flags = r.u8()?;
        let mut m = CscMeasurement::default();
        if flags & 0x01 != 0 {
            m.wheel = Some((r.u32()?, r.u16()?));
        }
        if flags & 0x02 != 0 {
            m.crank = Some((r.u16()?, r.u16()?));
        }
        Ok(m)
    }
    pub fn encode_crank(revs: u16, time_1024: u16) -> Vec<u8> {
        Writer::new().u8(0x02).u16(revs).u16(time_1024).done()
    }
}

// ------------------------------------------------------------ battery

pub fn parse_battery_level(b: &[u8]) -> Result<u8, ParseError> {
    let mut r = Reader::new(b);
    let v = r.u8()?;
    if v > 100 {
        return Err(ParseError::Invalid(format!("battery level {v} > 100")));
    }
    Ok(v)
}

pub fn parse_utf8(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_end_matches('\0').trim().chars().take(64).collect()
}

// ------------------------------------------------------------ rate from counters

/// Computes revolutions-per-minute from cumulative counter + event-time pairs.
///
/// * Counters and event times wrap (uint16 crank counts, uint16 event time,
///   uint32 wheel counts). Deltas are computed modulo the field width.
/// * A repeated event time means "no new revolution"; after `zero_after_ms`
///   without a new event the rate is reported as a measured 0 (coasting).
/// * Implausible values are dropped rather than reported.
#[derive(Debug, Clone)]
pub struct RevolutionRate {
    count_modulus: u64,
    time_ticks_per_s: f64,
    max_rpm: f64,
    zero_after_ms: u64,
    last: Option<(u64, u16, u64)>, // (count, event_time, received_at_ms)
    last_event_rx_ms: Option<u64>,
    pub rpm: Option<f64>,
}

impl RevolutionRate {
    pub fn crank() -> Self {
        Self::new(1 << 16, 1024.0, 250.0, 2500)
    }
    pub fn csc_wheel() -> Self {
        Self::new(1 << 32, 1024.0, 3000.0, 2500)
    }
    pub fn cps_wheel() -> Self {
        Self::new(1 << 32, 2048.0, 3000.0, 2500)
    }
    pub fn new(count_modulus: u64, time_ticks_per_s: f64, max_rpm: f64, zero_after_ms: u64) -> Self {
        RevolutionRate { count_modulus, time_ticks_per_s, max_rpm, zero_after_ms, last: None, last_event_rx_ms: None, rpm: None }
    }

    /// Feed a measurement received at monotonic time `now_ms`. Returns the
    /// current rate, which may be `None` until two events have been seen.
    pub fn update(&mut self, count: u64, event_time: u16, now_ms: u64) -> Option<f64> {
        match self.last {
            None => {
                self.last = Some((count, event_time, now_ms));
                self.last_event_rx_ms = Some(now_ms);
            }
            Some((pc, pt, _)) => {
                let dc = (count + self.count_modulus - pc) % self.count_modulus;
                let dt_ticks = (event_time as u32 + 65536 - pt as u32) % 65536;
                if dt_ticks == 0 {
                    // No new event. Coasting check.
                    if dc == 0 {
                        if let Some(t) = self.last_event_rx_ms {
                            if now_ms.saturating_sub(t) >= self.zero_after_ms {
                                self.rpm = Some(0.0);
                            }
                        }
                    }
                    // dc != 0 with dt == 0 is malformed; ignore.
                } else {
                    let dt_s = dt_ticks as f64 / self.time_ticks_per_s;
                    let rpm = dc as f64 / dt_s * 60.0;
                    if rpm.is_finite() && rpm <= self.max_rpm && dc < 1000 {
                        self.rpm = Some(rpm);
                    }
                    self.last = Some((count, event_time, now_ms));
                    self.last_event_rx_ms = Some(now_ms);
                }
            }
        }
        self.rpm
    }
    pub fn reset(&mut self) {
        self.last = None;
        self.last_event_rx_ms = None;
        self.rpm = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::from_hex;

    #[test]
    fn hr_8bit_and_16bit() {
        assert_eq!(HeartRateMeasurement::parse(&[0x00, 72]).unwrap().bpm, 72);
        let m = HeartRateMeasurement::parse(&[0x01, 0x2C, 0x01]).unwrap();
        assert_eq!(m.bpm, 300);
        // contact supported (bit2) and detected (bit1), energy (bit3), RR (bit4)
        let m = HeartRateMeasurement::parse(&from_hex("1E 8C 10 00 00 04 00 02").unwrap()).unwrap();
        assert_eq!(m.bpm, 140);
        assert_eq!(m.contact_detected, Some(true));
        assert_eq!(m.energy_expended_kj, Some(16));
        assert_eq!(m.rr_s, vec![1.0, 0.5]);
        assert!(HeartRateMeasurement::parse(&[0x01, 0x2C]).is_err());
        assert!(HeartRateMeasurement::parse(&[]).is_err());
    }

    #[test]
    fn cps_with_crank_and_wheel() {
        // flags: wheel (bit4) + crank (bit5) = 0x0030
        let b = from_hex("30 00 FA 00 10 27 00 00 00 08 64 00 00 04").unwrap();
        let m = CyclingPowerMeasurement::parse(&b).unwrap();
        assert_eq!(m.power_w, 250);
        assert_eq!(m.wheel, Some((10000, 2048)));
        assert_eq!(m.crank, Some((100, 1024)));
    }

    #[test]
    fn cps_skips_optional_fields_in_order() {
        // balance(bit0) + torque(bit2) + crank(bit5) + extreme angles(bit8) + energy(bit11)
        let flags: u16 = 1 | (1 << 2) | (1 << 5) | (1 << 8) | (1 << 11);
        let mut w = Writer::new();
        w.u16(flags).i16(-5).u8(100).u16(64).u16(7).u16(512).u8(1).u8(2).u8(3).u16(42);
        let m = CyclingPowerMeasurement::parse(&w.done()).unwrap();
        assert_eq!(m.power_w, -5);
        assert_eq!(m.pedal_balance_pct, Some(50.0));
        assert_eq!(m.accumulated_torque_nm, Some(2.0));
        assert_eq!(m.crank, Some((7, 512)));
        assert_eq!(m.accumulated_energy_kj, Some(42));
    }

    #[test]
    fn cps_truncated() {
        assert!(CyclingPowerMeasurement::parse(&from_hex("20 00 FA 00 64 00").unwrap()).is_err());
    }

    #[test]
    fn csc_parse() {
        let m = CscMeasurement::parse(&from_hex("03 01 00 00 00 00 04 05 00 00 08").unwrap()).unwrap();
        assert_eq!(m.wheel, Some((1, 1024)));
        assert_eq!(m.crank, Some((5, 2048)));
    }

    #[test]
    fn cadence_from_counters_with_rollover() {
        let mut r = RevolutionRate::crank();
        assert_eq!(r.update(65534, 65000, 0), None);
        // 2 revs in 1 s, both counters wrap.
        let t2 = ((65000u32 + 1024) % 65536) as u16;
        let rpm = r.update(0, t2, 1000).unwrap();
        assert!((rpm - 120.0).abs() < 1e-6, "{rpm}");
        // Same event repeated: rate unchanged until zero timeout.
        assert!((r.update(0, t2, 2000).unwrap() - 120.0).abs() < 1e-6);
        assert_eq!(r.update(0, t2, 3600), Some(0.0));
    }

    #[test]
    fn cadence_rejects_implausible() {
        let mut r = RevolutionRate::crank();
        r.update(0, 0, 0);
        // 50 revs in 1/1024 s => absurd; keep previous (None)
        assert_eq!(r.update(50, 1, 10), None);
    }

    #[test]
    fn battery() {
        assert_eq!(parse_battery_level(&[87]).unwrap(), 87);
        assert!(parse_battery_level(&[101]).is_err());
    }
}
