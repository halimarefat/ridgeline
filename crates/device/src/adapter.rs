//! Transport-neutral device adapter contract.
//!
//! Real BLE (desktop crate, btleplug) and the deterministic simulator both
//! implement [`DeviceAdapter`]. The contract is poll-based so the session
//! runtime controls time: adapters queue events internally (e.g. from async
//! BLE callbacks) and hand them over in `poll`.
//!
//! Requests (`connect`, `read`, `subscribe`, `write`) return immediately with
//! a request id; their outcome arrives later as an event. A successful
//! `WriteComplete` only means the transport delivered the bytes — machine
//! acceptance of a control command arrives separately as an FTMS control
//! point indication.

use std::fmt;

/// Adapter-local device identifier. Not assumed to be stable across
/// platforms (macOS exposes per-host UUIDs rather than MAC addresses).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub String);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Advertisement {
    pub id: DeviceId,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    /// 16-bit service UUIDs found in the advertisement (may be incomplete).
    pub services: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdapterErrorKind {
    AdapterUnavailable,
    PermissionDenied,
    NotFound,
    /// Typically: another app (e.g. another trainer app) holds the connection.
    Busy,
    Timeout,
    Disconnected,
    Unsupported,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterError {
    pub kind: AdapterErrorKind,
    pub message: String,
}

impl AdapterError {
    pub fn new(kind: AdapterErrorKind, message: impl Into<String>) -> Self {
        AdapterError { kind, message: message.into() }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

pub type RequestId = u64;

#[derive(Debug, Clone, PartialEq)]
pub enum AdapterEvent {
    AdapterState { available: bool, message: Option<String> },
    ScanStarted,
    ScanStopped,
    Discovered(Advertisement),
    /// GATT connection established and services discovered.
    Connected { id: DeviceId, services: Vec<u16>, characteristics: Vec<u16> },
    ConnectFailed { id: DeviceId, error: AdapterError },
    Disconnected { id: DeviceId, reason: Option<String> },
    ReadResult { id: DeviceId, req: RequestId, characteristic: u16, result: Result<Vec<u8>, AdapterError> },
    SubscribeResult { id: DeviceId, req: RequestId, characteristic: u16, result: Result<(), AdapterError> },
    /// Transport-level write completion only (not machine acceptance).
    WriteComplete { id: DeviceId, req: RequestId, characteristic: u16, result: Result<(), AdapterError> },
    /// Notification or indication payload.
    Notification { id: DeviceId, characteristic: u16, data: Vec<u8> },
}

pub trait DeviceAdapter: Send {
    /// Human-readable adapter kind: "ble" or "simulator".
    fn kind(&self) -> &'static str;
    fn start_scan(&mut self) -> Result<(), AdapterError>;
    fn stop_scan(&mut self);
    fn connect(&mut self, id: &DeviceId) -> Result<(), AdapterError>;
    fn disconnect(&mut self, id: &DeviceId);
    fn read(&mut self, id: &DeviceId, characteristic: u16) -> Result<RequestId, AdapterError>;
    fn subscribe(&mut self, id: &DeviceId, characteristic: u16) -> Result<RequestId, AdapterError>;
    fn write(&mut self, id: &DeviceId, characteristic: u16, data: &[u8], with_response: bool) -> Result<RequestId, AdapterError>;
    /// Drain queued events; `now_ms` is the runtime's monotonic clock.
    fn poll(&mut self, now_ms: u64) -> Vec<AdapterEvent>;
    /// Downcast hook so the app can reach simulator controls in demo mode.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
