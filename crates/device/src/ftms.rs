//! Fitness Machine Service (FTMS 1.0.1) parsing and command encoding.
//!
//! Field layouts follow the Bluetooth Fitness Machine Service specification
//! v1.0.1 and were cross-checked against an independent open-source
//! implementation (GoldenCheetah). Golden byte tests at the bottom pin every
//! encoding. Hardware validation is still required (docs/hardware-matrix.md).

use crate::bytes::{ParseError, Reader, Writer};

// ------------------------------------------------------------ feature bits

/// Fitness Machine Features field (first uint32 of 0x2ACC).
pub mod machine_feature {
    pub const AVERAGE_SPEED: u32 = 1 << 0;
    pub const CADENCE: u32 = 1 << 1;
    pub const TOTAL_DISTANCE: u32 = 1 << 2;
    pub const INCLINATION: u32 = 1 << 3;
    pub const ELEVATION_GAIN: u32 = 1 << 4;
    pub const PACE: u32 = 1 << 5;
    pub const STEP_COUNT: u32 = 1 << 6;
    pub const RESISTANCE_LEVEL: u32 = 1 << 7;
    pub const STRIDE_COUNT: u32 = 1 << 8;
    pub const EXPENDED_ENERGY: u32 = 1 << 9;
    pub const HEART_RATE: u32 = 1 << 10;
    pub const METABOLIC_EQUIVALENT: u32 = 1 << 11;
    pub const ELAPSED_TIME: u32 = 1 << 12;
    pub const REMAINING_TIME: u32 = 1 << 13;
    pub const POWER_MEASUREMENT: u32 = 1 << 14;
    pub const FORCE_ON_BELT: u32 = 1 << 15;
    pub const USER_DATA_RETENTION: u32 = 1 << 16;
}

/// Target Setting Features field (second uint32 of 0x2ACC).
pub mod target_feature {
    pub const SPEED: u32 = 1 << 0;
    pub const INCLINATION: u32 = 1 << 1;
    pub const RESISTANCE: u32 = 1 << 2;
    pub const POWER: u32 = 1 << 3;
    pub const HEART_RATE: u32 = 1 << 4;
    pub const EXPENDED_ENERGY: u32 = 1 << 5;
    pub const STEP_NUMBER: u32 = 1 << 6;
    pub const STRIDE_NUMBER: u32 = 1 << 7;
    pub const DISTANCE: u32 = 1 << 8;
    pub const TRAINING_TIME: u32 = 1 << 9;
    pub const TIME_TWO_HR_ZONES: u32 = 1 << 10;
    pub const TIME_THREE_HR_ZONES: u32 = 1 << 11;
    pub const TIME_FIVE_HR_ZONES: u32 = 1 << 12;
    pub const INDOOR_BIKE_SIMULATION: u32 = 1 << 13;
    pub const WHEEL_CIRCUMFERENCE: u32 = 1 << 14;
    pub const SPIN_DOWN: u32 = 1 << 15;
    pub const CADENCE: u32 = 1 << 16;
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FtmsFeatures {
    pub machine: u32,
    pub target: u32,
}

impl FtmsFeatures {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        Ok(FtmsFeatures { machine: r.u32()?, target: r.u32()? })
    }
    pub fn encode(&self) -> Vec<u8> {
        Writer::new().u32(self.machine).u32(self.target).done()
    }
    pub fn supports_power_target(&self) -> bool {
        self.target & target_feature::POWER != 0
    }
    pub fn supports_simulation(&self) -> bool {
        self.target & target_feature::INDOOR_BIKE_SIMULATION != 0
    }
    pub fn supports_resistance_target(&self) -> bool {
        self.target & target_feature::RESISTANCE != 0
    }
    pub fn supports_spin_down(&self) -> bool {
        self.target & target_feature::SPIN_DOWN != 0
    }
    pub fn reports_power(&self) -> bool {
        self.machine & machine_feature::POWER_MEASUREMENT != 0
    }
    pub fn reports_cadence(&self) -> bool {
        self.machine & machine_feature::CADENCE != 0
    }
}

// ------------------------------------------------------------ ranges

/// Supported Power Range (0x2AD8): sint16 min W, sint16 max W, uint16 increment W.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerRange {
    pub min_w: i16,
    pub max_w: i16,
    pub inc_w: u16,
}
impl PowerRange {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let pr = PowerRange { min_w: r.i16()?, max_w: r.i16()?, inc_w: r.u16()? };
        if pr.max_w < pr.min_w {
            return Err(ParseError::Invalid("power range max < min".into()));
        }
        Ok(pr)
    }
    pub fn encode(&self) -> Vec<u8> {
        Writer::new().i16(self.min_w).i16(self.max_w).u16(self.inc_w).done()
    }
}

/// Supported Resistance Level Range (0x2AD6): sint16 min, sint16 max,
/// uint16 increment; unitless with 0.1 resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResistanceRange {
    pub min: f64,
    pub max: f64,
    pub inc: f64,
}
impl ResistanceRange {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let min = r.i16()? as f64 * 0.1;
        let max = r.i16()? as f64 * 0.1;
        let inc = r.u16()? as f64 * 0.1;
        if max < min {
            return Err(ParseError::Invalid("resistance range max < min".into()));
        }
        Ok(ResistanceRange { min, max, inc })
    }
}

// ------------------------------------------------------------ indoor bike data

pub mod ibd_flags {
    pub const MORE_DATA: u16 = 1 << 0; // NOTE: 0 => instantaneous speed present
    pub const AVERAGE_SPEED: u16 = 1 << 1;
    pub const INST_CADENCE: u16 = 1 << 2;
    pub const AVERAGE_CADENCE: u16 = 1 << 3;
    pub const TOTAL_DISTANCE: u16 = 1 << 4;
    pub const RESISTANCE_LEVEL: u16 = 1 << 5;
    pub const INST_POWER: u16 = 1 << 6;
    pub const AVERAGE_POWER: u16 = 1 << 7;
    pub const EXPENDED_ENERGY: u16 = 1 << 8;
    pub const HEART_RATE: u16 = 1 << 9;
    pub const METABOLIC_EQUIVALENT: u16 = 1 << 10;
    pub const ELAPSED_TIME: u16 = 1 << 11;
    pub const REMAINING_TIME: u16 = 1 << 12;
}

/// Indoor Bike Data (0x2AD2) after unit conversion. `None` means the field was
/// absent from this packet (it is not zero).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndoorBikeData {
    pub flags: u16,
    pub speed_kmh: Option<f64>,
    pub avg_speed_kmh: Option<f64>,
    pub cadence_rpm: Option<f64>,
    pub avg_cadence_rpm: Option<f64>,
    pub total_distance_m: Option<u32>,
    pub resistance_level: Option<i16>,
    pub power_w: Option<i16>,
    pub avg_power_w: Option<i16>,
    pub total_energy_kcal: Option<u16>,
    pub energy_per_hour_kcal: Option<u16>,
    pub energy_per_min_kcal: Option<u8>,
    pub heart_rate_bpm: Option<u8>,
    pub met: Option<f64>,
    pub elapsed_s: Option<u16>,
    pub remaining_s: Option<u16>,
}

/// FTMS uses 0xFFFF / 0xFF "data not available" sentinels for some energy fields.
fn energy16(v: u16) -> Option<u16> {
    if v == 0xFFFF {
        None
    } else {
        Some(v)
    }
}

impl IndoorBikeData {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        use ibd_flags::*;
        if b.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut r = Reader::new(b);
        let flags = r.u16()?;
        let mut d = IndoorBikeData { flags, ..Default::default() };
        if flags & MORE_DATA == 0 {
            d.speed_kmh = Some(r.u16()? as f64 * 0.01);
        }
        if flags & AVERAGE_SPEED != 0 {
            d.avg_speed_kmh = Some(r.u16()? as f64 * 0.01);
        }
        if flags & INST_CADENCE != 0 {
            d.cadence_rpm = Some(r.u16()? as f64 * 0.5);
        }
        if flags & AVERAGE_CADENCE != 0 {
            d.avg_cadence_rpm = Some(r.u16()? as f64 * 0.5);
        }
        if flags & TOTAL_DISTANCE != 0 {
            d.total_distance_m = Some(r.u24()?);
        }
        if flags & RESISTANCE_LEVEL != 0 {
            d.resistance_level = Some(r.i16()?);
        }
        if flags & INST_POWER != 0 {
            d.power_w = Some(r.i16()?);
        }
        if flags & AVERAGE_POWER != 0 {
            d.avg_power_w = Some(r.i16()?);
        }
        if flags & EXPENDED_ENERGY != 0 {
            d.total_energy_kcal = energy16(r.u16()?);
            d.energy_per_hour_kcal = energy16(r.u16()?);
            let m = r.u8()?;
            d.energy_per_min_kcal = if m == 0xFF { None } else { Some(m) };
        }
        if flags & HEART_RATE != 0 {
            d.heart_rate_bpm = Some(r.u8()?);
        }
        if flags & METABOLIC_EQUIVALENT != 0 {
            d.met = Some(r.u8()? as f64 * 0.1);
        }
        if flags & ELAPSED_TIME != 0 {
            d.elapsed_s = Some(r.u16()?);
        }
        if flags & REMAINING_TIME != 0 {
            d.remaining_s = Some(r.u16()?);
        }
        Ok(d)
    }

    /// Encoder used by the simulator so simulated devices exercise the same
    /// parser as real ones. Supports the subset of fields the simulator emits.
    pub fn encode(speed_kmh: Option<f64>, cadence_rpm: Option<f64>, power_w: Option<i16>, hr: Option<u8>) -> Vec<u8> {
        use ibd_flags::*;
        let mut flags = 0u16;
        if speed_kmh.is_none() {
            flags |= MORE_DATA;
        }
        if cadence_rpm.is_some() {
            flags |= INST_CADENCE;
        }
        if power_w.is_some() {
            flags |= INST_POWER;
        }
        if hr.is_some() {
            flags |= HEART_RATE;
        }
        let mut w = Writer::new();
        w.u16(flags);
        if let Some(s) = speed_kmh {
            w.u16((s / 0.01).round().clamp(0.0, 65535.0) as u16);
        }
        if let Some(c) = cadence_rpm {
            w.u16((c / 0.5).round().clamp(0.0, 65535.0) as u16);
        }
        if let Some(p) = power_w {
            w.i16(p);
        }
        if let Some(h) = hr {
            w.u8(h);
        }
        w.done()
    }
}

// ------------------------------------------------------------ control point

pub mod opcode {
    pub const REQUEST_CONTROL: u8 = 0x00;
    pub const RESET: u8 = 0x01;
    pub const SET_TARGET_SPEED: u8 = 0x02;
    pub const SET_TARGET_INCLINATION: u8 = 0x03;
    pub const SET_TARGET_RESISTANCE: u8 = 0x04;
    pub const SET_TARGET_POWER: u8 = 0x05;
    pub const SET_TARGET_HEART_RATE: u8 = 0x06;
    pub const START_OR_RESUME: u8 = 0x07;
    pub const STOP_OR_PAUSE: u8 = 0x08;
    pub const SET_INDOOR_BIKE_SIMULATION: u8 = 0x11;
    pub const SET_WHEEL_CIRCUMFERENCE: u8 = 0x12;
    pub const SPIN_DOWN_CONTROL: u8 = 0x13;
    pub const SET_TARGETED_CADENCE: u8 = 0x14;
    pub const RESPONSE_CODE: u8 = 0x80;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCode {
    Success,
    OpCodeNotSupported,
    InvalidParameter,
    OperationFailed,
    ControlNotPermitted,
    Unknown(u8),
}

impl ResultCode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => ResultCode::Success,
            0x02 => ResultCode::OpCodeNotSupported,
            0x03 => ResultCode::InvalidParameter,
            0x04 => ResultCode::OperationFailed,
            0x05 => ResultCode::ControlNotPermitted,
            x => ResultCode::Unknown(x),
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            ResultCode::Success => 0x01,
            ResultCode::OpCodeNotSupported => 0x02,
            ResultCode::InvalidParameter => 0x03,
            ResultCode::OperationFailed => 0x04,
            ResultCode::ControlNotPermitted => 0x05,
            ResultCode::Unknown(x) => x,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ResultCode::Success => "success",
            ResultCode::OpCodeNotSupported => "not supported",
            ResultCode::InvalidParameter => "invalid parameter",
            ResultCode::OperationFailed => "operation failed",
            ResultCode::ControlNotPermitted => "control not permitted",
            ResultCode::Unknown(_) => "unknown result",
        }
    }
}

/// Validated simulation parameters in SI / percent units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimParams {
    /// Headwind positive, m/s.
    pub wind_speed_mps: f64,
    /// Road grade in percent (5.0 == 5 %), not a fraction and not degrees.
    pub grade_percent: f64,
    /// Coefficient of rolling resistance (dimensionless, e.g. 0.004).
    pub crr: f64,
    /// Wind resistance coefficient in kg/m (FTMS "Cw"), e.g. 0.51.
    pub cw_kg_per_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopKind {
    Stop,
    Pause,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    RequestControl,
    Reset,
    StartOrResume,
    StopOrPause(StopKind),
    SetTargetPower { watts: i16 },
    /// Resistance level, unitless. Encoded as sint16 with 0.1 resolution, as in
    /// widely deployed implementations. Must be confirmed per device.
    SetTargetResistance { level: f64 },
    SetSimulation(SimParams),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EncodeError {
    OutOfRange(&'static str, f64),
    NonFinite(&'static str),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::OutOfRange(n, v) => write!(f, "{n} out of encodable range: {v}"),
            EncodeError::NonFinite(n) => write!(f, "{n} is not finite"),
        }
    }
}

fn scaled_i16(name: &'static str, v: f64, resolution: f64) -> Result<i16, EncodeError> {
    if !v.is_finite() {
        return Err(EncodeError::NonFinite(name));
    }
    let raw = (v / resolution).round();
    if raw < i16::MIN as f64 || raw > i16::MAX as f64 {
        return Err(EncodeError::OutOfRange(name, v));
    }
    Ok(raw as i16)
}

fn scaled_u8(name: &'static str, v: f64, resolution: f64) -> Result<u8, EncodeError> {
    if !v.is_finite() {
        return Err(EncodeError::NonFinite(name));
    }
    let raw = (v / resolution).round();
    if !(0.0..=255.0).contains(&raw) {
        return Err(EncodeError::OutOfRange(name, v));
    }
    Ok(raw as u8)
}

impl ControlCommand {
    pub fn opcode(&self) -> u8 {
        use opcode::*;
        match self {
            ControlCommand::RequestControl => REQUEST_CONTROL,
            ControlCommand::Reset => RESET,
            ControlCommand::StartOrResume => START_OR_RESUME,
            ControlCommand::StopOrPause(_) => STOP_OR_PAUSE,
            ControlCommand::SetTargetPower { .. } => SET_TARGET_POWER,
            ControlCommand::SetTargetResistance { .. } => SET_TARGET_RESISTANCE,
            ControlCommand::SetSimulation(_) => SET_INDOOR_BIKE_SIMULATION,
        }
    }

    /// True for commands whose newer instance makes a pending older one obsolete.
    pub fn is_target(&self) -> bool {
        matches!(self, ControlCommand::SetTargetPower { .. } | ControlCommand::SetTargetResistance { .. } | ControlCommand::SetSimulation(_))
    }

    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut w = Writer::new();
        w.u8(self.opcode());
        match self {
            ControlCommand::RequestControl | ControlCommand::Reset | ControlCommand::StartOrResume => {}
            ControlCommand::StopOrPause(k) => {
                w.u8(match k {
                    StopKind::Stop => 0x01,
                    StopKind::Pause => 0x02,
                });
            }
            ControlCommand::SetTargetPower { watts } => {
                w.i16(*watts);
            }
            ControlCommand::SetTargetResistance { level } => {
                w.i16(scaled_i16("resistance level", *level, 0.1)?);
            }
            ControlCommand::SetSimulation(p) => {
                // wind speed: sint16, 0.001 m/s
                w.i16(scaled_i16("wind speed", p.wind_speed_mps, 0.001)?);
                // grade: sint16, 0.01 %
                w.i16(scaled_i16("grade", p.grade_percent, 0.01)?);
                // Crr: uint8, 0.0001
                w.u8(scaled_u8("crr", p.crr, 0.0001)?);
                // Cw: uint8, 0.01 kg/m
                w.u8(scaled_u8("cw", p.cw_kg_per_m, 0.01)?);
            }
        }
        Ok(w.done())
    }

    /// Decode a control point write (used by the simulator acting as a trainer).
    pub fn decode(b: &[u8]) -> Result<ControlCommand, ParseError> {
        use opcode::*;
        let mut r = Reader::new(b);
        let op = r.u8()?;
        Ok(match op {
            REQUEST_CONTROL => ControlCommand::RequestControl,
            RESET => ControlCommand::Reset,
            START_OR_RESUME => ControlCommand::StartOrResume,
            STOP_OR_PAUSE => match r.u8()? {
                0x01 => ControlCommand::StopOrPause(StopKind::Stop),
                0x02 => ControlCommand::StopOrPause(StopKind::Pause),
                x => return Err(ParseError::Invalid(format!("stop/pause parameter {x:#04x}"))),
            },
            SET_TARGET_POWER => ControlCommand::SetTargetPower { watts: r.i16()? },
            SET_TARGET_RESISTANCE => ControlCommand::SetTargetResistance { level: r.i16()? as f64 * 0.1 },
            SET_INDOOR_BIKE_SIMULATION => ControlCommand::SetSimulation(SimParams {
                wind_speed_mps: r.i16()? as f64 * 0.001,
                grade_percent: r.i16()? as f64 * 0.01,
                crr: r.u8()? as f64 * 0.0001,
                cw_kg_per_m: r.u8()? as f64 * 0.01,
            }),
            x => return Err(ParseError::Invalid(format!("unsupported opcode {x:#04x}"))),
        })
    }
}

/// Control point indication: 0x80, request op code, result code, [params].
#[derive(Debug, Clone, PartialEq)]
pub struct ControlResponse {
    pub request_opcode: u8,
    pub result: ResultCode,
}

impl ControlResponse {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let code = r.u8()?;
        if code != opcode::RESPONSE_CODE {
            return Err(ParseError::Invalid(format!("not a response indication (op {code:#04x})")));
        }
        Ok(ControlResponse { request_opcode: r.u8()?, result: ResultCode::from_u8(r.u8()?) })
    }
    pub fn encode(&self) -> Vec<u8> {
        vec![opcode::RESPONSE_CODE, self.request_opcode, self.result.to_u8()]
    }
}

// ------------------------------------------------------------ machine status

#[derive(Debug, Clone, PartialEq)]
pub enum MachineStatus {
    Reset,
    StoppedOrPausedByUser(StopKind),
    StoppedBySafetyKey,
    StartedOrResumedByUser,
    TargetResistanceChanged(f64),
    TargetPowerChanged(i16),
    SimulationParametersChanged(SimParams),
    ControlPermissionLost,
    Other(u8),
}

impl MachineStatus {
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(b);
        let op = r.u8()?;
        Ok(match op {
            0x01 => MachineStatus::Reset,
            0x02 => MachineStatus::StoppedOrPausedByUser(if r.u8()? == 0x02 { StopKind::Pause } else { StopKind::Stop }),
            0x03 => MachineStatus::StoppedBySafetyKey,
            0x04 => MachineStatus::StartedOrResumedByUser,
            0x07 => MachineStatus::TargetResistanceChanged(r.u8()? as f64 * 0.1),
            0x08 => MachineStatus::TargetPowerChanged(r.i16()?),
            0x12 => MachineStatus::SimulationParametersChanged(SimParams {
                wind_speed_mps: r.i16()? as f64 * 0.001,
                grade_percent: r.i16()? as f64 * 0.01,
                crr: r.u8()? as f64 * 0.0001,
                cw_kg_per_m: r.u8()? as f64 * 0.01,
            }),
            0xFF => MachineStatus::ControlPermissionLost,
            x => MachineStatus::Other(x),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{from_hex, hex};

    #[test]
    fn golden_set_target_power() {
        let b = ControlCommand::SetTargetPower { watts: 250 }.encode().unwrap();
        assert_eq!(hex(&b), "05 FA 00");
        let b = ControlCommand::SetTargetPower { watts: 1000 }.encode().unwrap();
        assert_eq!(hex(&b), "05 E8 03");
    }

    #[test]
    fn golden_simulation_five_percent() {
        // 5 % grade must encode as 500 (0x01F4) in 0.01 % units — not 5 or 50000.
        let p = SimParams { wind_speed_mps: 0.0, grade_percent: 5.0, crr: 0.004, cw_kg_per_m: 0.51 };
        let b = ControlCommand::SetSimulation(p).encode().unwrap();
        assert_eq!(hex(&b), "11 00 00 F4 01 28 33");
    }

    #[test]
    fn golden_simulation_negative_grade() {
        let p = SimParams { wind_speed_mps: -1.5, grade_percent: -3.0, crr: 0.005, cw_kg_per_m: 0.0 };
        let b = ControlCommand::SetSimulation(p).encode().unwrap();
        // wind -1500 = 0xFA24, grade -300 = 0xFED4, crr 50 = 0x32, cw 0
        assert_eq!(hex(&b), "11 24 FA D4 FE 32 00");
        match ControlCommand::decode(&b).unwrap() {
            ControlCommand::SetSimulation(d) => {
                assert!((d.grade_percent + 3.0).abs() < 1e-9);
                assert!((d.wind_speed_mps + 1.5).abs() < 1e-9);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn simulation_rejects_out_of_range() {
        let p = SimParams { wind_speed_mps: 0.0, grade_percent: 400.0, crr: 0.004, cw_kg_per_m: 0.51 };
        assert!(ControlCommand::SetSimulation(p).encode().is_err());
        let p = SimParams { wind_speed_mps: 0.0, grade_percent: f64::NAN, crr: 0.004, cw_kg_per_m: 0.51 };
        assert!(ControlCommand::SetSimulation(p).encode().is_err());
        let p = SimParams { wind_speed_mps: 0.0, grade_percent: 1.0, crr: 0.04, cw_kg_per_m: 0.51 };
        assert!(ControlCommand::SetSimulation(p).encode().is_err(), "crr 0.04 => 400 > 255");
    }

    #[test]
    fn golden_simple_commands() {
        assert_eq!(hex(&ControlCommand::RequestControl.encode().unwrap()), "00");
        assert_eq!(hex(&ControlCommand::StartOrResume.encode().unwrap()), "07");
        assert_eq!(hex(&ControlCommand::StopOrPause(StopKind::Stop).encode().unwrap()), "08 01");
        assert_eq!(hex(&ControlCommand::StopOrPause(StopKind::Pause).encode().unwrap()), "08 02");
        assert_eq!(hex(&ControlCommand::SetTargetResistance { level: 12.5 }.encode().unwrap()), "04 7D 00");
    }

    #[test]
    fn response_parsing() {
        let r = ControlResponse::parse(&[0x80, 0x05, 0x01]).unwrap();
        assert_eq!(r.request_opcode, 0x05);
        assert_eq!(r.result, ResultCode::Success);
        let r = ControlResponse::parse(&[0x80, 0x00, 0x05]).unwrap();
        assert_eq!(r.result, ResultCode::ControlNotPermitted);
        assert!(ControlResponse::parse(&[0x80, 0x05]).is_err());
        assert!(ControlResponse::parse(&[0x05, 0x05, 0x01]).is_err());
    }

    #[test]
    fn indoor_bike_data_speed_cadence_power() {
        // flags 0x0044: speed present (bit0=0), cadence (bit2), power (bit6)
        // speed 3250 => 32.50 km/h, cadence 180 => 90 rpm, power 0x00C8 = 200 W
        let b = from_hex("44 00 B2 0C B4 00 C8 00").unwrap();
        let d = IndoorBikeData::parse(&b).unwrap();
        assert_eq!(d.speed_kmh, Some(32.5));
        assert_eq!(d.cadence_rpm, Some(90.0));
        assert_eq!(d.power_w, Some(200));
        assert_eq!(d.heart_rate_bpm, None);
    }

    #[test]
    fn indoor_bike_data_more_data_means_no_speed() {
        // flags 0x0041: more data set => no speed; power only.
        let d = IndoorBikeData::parse(&from_hex("41 00 2C 01").unwrap()).unwrap();
        assert_eq!(d.speed_kmh, None);
        assert_eq!(d.power_w, Some(300));
    }

    #[test]
    fn indoor_bike_data_all_fields() {
        // flags = all bits 1..12 set, bit0 = 0 (speed present) => 0x1FFE
        let mut w = Writer::new();
        w.u16(0x1FFE)
            .u16(2500) // speed 25.00
            .u16(2400) // avg speed 24.00
            .u16(170) // cadence 85
            .u16(160) // avg cadence 80
            .u24(70000) // distance
            .i16(-5) // resistance
            .i16(-10) // power (negative allowed by sint16)
            .i16(180) // avg power
            .u16(0xFFFF) // total energy N/A
            .u16(600)
            .u8(0xFF)
            .u8(142) // HR
            .u8(65) // MET 6.5
            .u16(3600)
            .u16(120);
        let d = IndoorBikeData::parse(&w.done()).unwrap();
        assert_eq!(d.speed_kmh, Some(25.0));
        assert_eq!(d.avg_speed_kmh, Some(24.0));
        assert_eq!(d.cadence_rpm, Some(85.0));
        assert_eq!(d.avg_cadence_rpm, Some(80.0));
        assert_eq!(d.total_distance_m, Some(70000));
        assert_eq!(d.resistance_level, Some(-5));
        assert_eq!(d.power_w, Some(-10));
        assert_eq!(d.avg_power_w, Some(180));
        assert_eq!(d.total_energy_kcal, None);
        assert_eq!(d.energy_per_hour_kcal, Some(600));
        assert_eq!(d.energy_per_min_kcal, None);
        assert_eq!(d.heart_rate_bpm, Some(142));
        assert!((d.met.unwrap() - 6.5).abs() < 1e-9);
        assert_eq!(d.elapsed_s, Some(3600));
        assert_eq!(d.remaining_s, Some(120));
    }

    #[test]
    fn indoor_bike_data_truncated() {
        assert!(IndoorBikeData::parse(&[]).is_err());
        assert!(IndoorBikeData::parse(&[0x44]).is_err());
        assert!(IndoorBikeData::parse(&from_hex("44 00 B2 0C B4 00 C8").unwrap()).is_err());
    }

    #[test]
    fn indoor_bike_data_encode_roundtrip() {
        let b = IndoorBikeData::encode(Some(30.0), Some(92.5), Some(215), Some(150));
        let d = IndoorBikeData::parse(&b).unwrap();
        assert_eq!(d.speed_kmh, Some(30.0));
        assert_eq!(d.cadence_rpm, Some(92.5));
        assert_eq!(d.power_w, Some(215));
        assert_eq!(d.heart_rate_bpm, Some(150));
    }

    #[test]
    fn features_and_ranges() {
        let f = FtmsFeatures::parse(&from_hex("02 40 00 00 0C 20 00 00").unwrap()).unwrap();
        assert!(f.reports_cadence());
        assert!(f.reports_power());
        assert!(f.supports_power_target());
        assert!(f.supports_resistance_target());
        assert!(f.supports_simulation());
        assert!(!f.supports_spin_down());
        let pr = PowerRange::parse(&from_hex("00 00 D0 07 01 00").unwrap()).unwrap();
        assert_eq!(pr, PowerRange { min_w: 0, max_w: 2000, inc_w: 1 });
        let rr = ResistanceRange::parse(&from_hex("00 00 E8 03 0A 00").unwrap()).unwrap();
        assert!((rr.max - 100.0).abs() < 1e-9 && (rr.inc - 1.0).abs() < 1e-9);
        assert!(PowerRange::parse(&from_hex("D0 07 00 00 01 00").unwrap()).is_err());
    }

    #[test]
    fn machine_status() {
        assert_eq!(MachineStatus::parse(&[0xFF]).unwrap(), MachineStatus::ControlPermissionLost);
        assert_eq!(MachineStatus::parse(&[0x08, 0xC8, 0x00]).unwrap(), MachineStatus::TargetPowerChanged(200));
        match MachineStatus::parse(&from_hex("12 00 00 F4 01 28 33").unwrap()).unwrap() {
            MachineStatus::SimulationParametersChanged(p) => assert!((p.grade_percent - 5.0).abs() < 1e-9),
            _ => panic!(),
        }
    }
}
