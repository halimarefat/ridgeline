//! Bluetooth fitness-device layer: GATT protocol parsing/encoding (FTMS,
//! Cycling Power, Heart Rate, CSC, Battery, Device Information), the
//! transport-neutral adapter contract, a deterministic simulator, the device
//! connection state machine, telemetry normalization and the FTMS trainer
//! controller. The real BLE adapter (btleplug) lives in the desktop crate and
//! implements the same [`adapter::DeviceAdapter`] contract.

pub mod adapter;
pub mod bytes;
pub mod controller;
pub mod ftms;
pub mod gatt;
pub mod manager;
pub mod sensors;
pub mod simulator;
pub mod telemetry;
