//! GATT assigned numbers used by Ridgeline.
//!
//! Source: Bluetooth SIG Assigned Numbers document (16-bit UUIDs for services
//! and characteristics). Each value below must be re-checked against the
//! current Assigned Numbers PDF before release (see docs/protocol.md).

// Services
pub const SVC_FTMS: u16 = 0x1826;
pub const SVC_CYCLING_POWER: u16 = 0x1818;
pub const SVC_HEART_RATE: u16 = 0x180D;
pub const SVC_CSC: u16 = 0x1816;
pub const SVC_BATTERY: u16 = 0x180F;
pub const SVC_DEVICE_INFO: u16 = 0x180A;

// FTMS characteristics
pub const CHR_FTMS_FEATURE: u16 = 0x2ACC;
pub const CHR_INDOOR_BIKE_DATA: u16 = 0x2AD2;
pub const CHR_TRAINING_STATUS: u16 = 0x2AD3;
pub const CHR_SUPPORTED_RESISTANCE_RANGE: u16 = 0x2AD6;
pub const CHR_SUPPORTED_POWER_RANGE: u16 = 0x2AD8;
pub const CHR_FTMS_CONTROL_POINT: u16 = 0x2AD9;
pub const CHR_FTMS_STATUS: u16 = 0x2ADA;

// Other characteristics
pub const CHR_CYCLING_POWER_MEASUREMENT: u16 = 0x2A63;
pub const CHR_CYCLING_POWER_FEATURE: u16 = 0x2A65;
pub const CHR_HEART_RATE_MEASUREMENT: u16 = 0x2A37;
pub const CHR_CSC_MEASUREMENT: u16 = 0x2A5B;
pub const CHR_BATTERY_LEVEL: u16 = 0x2A19;
pub const CHR_MANUFACTURER_NAME: u16 = 0x2A29;
pub const CHR_MODEL_NUMBER: u16 = 0x2A24;
pub const CHR_FIRMWARE_REVISION: u16 = 0x2A26;

/// Expand a 16-bit SIG UUID onto the Bluetooth Base UUID
/// 0000xxxx-0000-1000-8000-00805F9B34FB.
pub fn uuid128(short: u16) -> String {
    format!("0000{short:04x}-0000-1000-8000-00805f9b34fb")
}

/// Extract the 16-bit short form from a 128-bit UUID string on the base UUID.
pub fn short_from_uuid128(u: &str) -> Option<u16> {
    let l = u.to_ascii_lowercase();
    if l.len() == 36 && l.starts_with("0000") && l.ends_with("-0000-1000-8000-00805f9b34fb") {
        u16::from_str_radix(&l[4..8], 16).ok()
    } else {
        None
    }
}

pub fn service_name(s: u16) -> &'static str {
    match s {
        SVC_FTMS => "Fitness Machine",
        SVC_CYCLING_POWER => "Cycling Power",
        SVC_HEART_RATE => "Heart Rate",
        SVC_CSC => "Cycling Speed and Cadence",
        SVC_BATTERY => "Battery",
        SVC_DEVICE_INFO => "Device Information",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uuid_roundtrip() {
        assert_eq!(uuid128(0x1826), "00001826-0000-1000-8000-00805f9b34fb");
        assert_eq!(short_from_uuid128("00002AD9-0000-1000-8000-00805F9B34FB"), Some(0x2AD9));
        assert_eq!(short_from_uuid128("6e40fec1-b5a3-f393-e0a9-e50e24dcca9e"), None);
    }
}
