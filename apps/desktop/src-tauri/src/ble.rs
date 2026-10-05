//! Native Bluetooth LE adapter (btleplug) implementing the same
//! `DeviceAdapter` contract as the simulator.
//!
//! btleplug is async; this adapter runs it on a private Tokio runtime thread.
//! Commands go in through a channel and results/notifications are queued as
//! `AdapterEvent`s that the device manager drains in `poll`. The session
//! clock never waits on Bluetooth I/O.
//!
//! Hardware status: compiled and packaged by CI for Windows and macOS. Real
//! trainer behaviour must be verified per device (docs/hardware-matrix.md).

use btleplug::api::{bleuuid::uuid_from_u16, Central, CentralEvent, CentralState, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use rl_device::adapter::{AdapterError, AdapterErrorKind, AdapterEvent, Advertisement, DeviceAdapter, DeviceId, RequestId};
use rl_device::gatt::short_from_uuid128;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const OP_TIMEOUT: Duration = Duration::from_secs(10);

enum Cmd {
    StartScan,
    StopScan,
    Connect(DeviceId),
    Disconnect(DeviceId),
    Read(DeviceId, u16, RequestId),
    Subscribe(DeviceId, u16, RequestId),
    Write(DeviceId, u16, Vec<u8>, bool, RequestId),
}

type Queue = Arc<Mutex<Vec<AdapterEvent>>>;

pub struct BleAdapter {
    tx: UnboundedSender<Cmd>,
    events: Queue,
    next_req: RequestId,
}

fn push(q: &Queue, e: AdapterEvent) {
    if let Ok(mut v) = q.lock() {
        if v.len() < 10_000 {
            v.push(e);
        }
    }
}

fn map_err(e: btleplug::Error) -> AdapterError {
    use btleplug::Error as E;
    let msg = e.to_string();
    let kind = match e {
        E::PermissionDenied => AdapterErrorKind::PermissionDenied,
        E::DeviceNotFound => AdapterErrorKind::NotFound,
        E::NotConnected => AdapterErrorKind::Disconnected,
        E::NotSupported(_) => AdapterErrorKind::Unsupported,
        E::TimedOut(_) => AdapterErrorKind::Timeout,
        _ => AdapterErrorKind::Other,
    };
    AdapterError::new(kind, msg)
}

fn short(u: &uuid::Uuid) -> Option<u16> {
    short_from_uuid128(&u.to_string())
}

/// Stable-enough textual id for a peripheral on this platform. Not assumed
/// to be stable across platforms or OS reinstalls.
fn id_string(id: &PeripheralId) -> String {
    let raw = format!("{id:?}");
    let inner = raw.trim_start_matches("PeripheralId(").trim_end_matches(')');
    inner.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').take(64).collect()
}

impl BleAdapter {
    pub fn start() -> BleAdapter {
        let (tx, rx) = unbounded_channel();
        let events: Queue = Arc::new(Mutex::new(Vec::new()));
        let q = events.clone();
        let spawned = std::thread::Builder::new().name("ridgeline-ble".into()).spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    push(&q, AdapterEvent::AdapterState { available: false, message: Some(format!("Bluetooth runtime failed: {e}")) });
                    return;
                }
            };
            rt.block_on(run(rx, q));
        });
        if let Err(e) = spawned {
            push(&events, AdapterEvent::AdapterState { available: false, message: Some(format!("Bluetooth thread failed: {e}")) });
        }
        BleAdapter { tx, events, next_req: 1 }
    }

    fn send(&self, c: Cmd) -> Result<(), AdapterError> {
        self.tx.send(c).map_err(|_| AdapterError::new(AdapterErrorKind::AdapterUnavailable, "Bluetooth service stopped"))
    }
    fn req(&mut self) -> RequestId {
        self.next_req += 1;
        self.next_req
    }
}

impl DeviceAdapter for BleAdapter {
    fn kind(&self) -> &'static str {
        "ble"
    }
    fn start_scan(&mut self) -> Result<(), AdapterError> {
        self.send(Cmd::StartScan)
    }
    fn stop_scan(&mut self) {
        let _ = self.send(Cmd::StopScan);
    }
    fn connect(&mut self, id: &DeviceId) -> Result<(), AdapterError> {
        self.send(Cmd::Connect(id.clone()))
    }
    fn disconnect(&mut self, id: &DeviceId) {
        let _ = self.send(Cmd::Disconnect(id.clone()));
    }
    fn read(&mut self, id: &DeviceId, ch: u16) -> Result<RequestId, AdapterError> {
        let r = self.req();
        self.send(Cmd::Read(id.clone(), ch, r))?;
        Ok(r)
    }
    fn subscribe(&mut self, id: &DeviceId, ch: u16) -> Result<RequestId, AdapterError> {
        let r = self.req();
        self.send(Cmd::Subscribe(id.clone(), ch, r))?;
        Ok(r)
    }
    fn write(&mut self, id: &DeviceId, ch: u16, data: &[u8], with_response: bool) -> Result<RequestId, AdapterError> {
        let r = self.req();
        self.send(Cmd::Write(id.clone(), ch, data.to_vec(), with_response, r))?;
        Ok(r)
    }
    fn poll(&mut self, _now_ms: u64) -> Vec<AdapterEvent> {
        self.events.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

struct Ctx {
    central: Adapter,
    q: Queue,
    by_id: Arc<Mutex<HashMap<String, PeripheralId>>>,
}

impl Ctx {
    async fn peripheral(&self, id: &DeviceId) -> Result<Peripheral, AdapterError> {
        let pid = self.by_id.lock().ok().and_then(|m| m.get(&id.0).cloned()).ok_or_else(|| AdapterError::new(AdapterErrorKind::NotFound, "Device not seen in this scan. Scan again."))?;
        self.central.peripheral(&pid).await.map_err(map_err)
    }
}

fn find_char(p: &Peripheral, ch: u16) -> Option<btleplug::api::Characteristic> {
    let u = uuid_from_u16(ch);
    p.characteristics().into_iter().find(|c| c.uuid == u)
}

async fn announce(ctx: &Ctx, pid: &PeripheralId) {
    let Ok(p) = ctx.central.peripheral(pid).await else { return };
    let Ok(Some(props)) = p.properties().await else { return };
    let id = id_string(pid);
    if let Ok(mut m) = ctx.by_id.lock() {
        m.insert(id.clone(), pid.clone());
    }
    let services: Vec<u16> = props.services.iter().filter_map(short).collect();
    push(&ctx.q, AdapterEvent::Discovered(Advertisement { id: DeviceId(id), name: props.local_name.clone(), rssi: props.rssi, services }));
}

async fn run(mut rx: UnboundedReceiver<Cmd>, q: Queue) {
    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            push(&q, AdapterEvent::AdapterState { available: false, message: Some(map_err(e).message) });
            drain_unavailable(rx, q).await;
            return;
        }
    };
    let central = match manager.adapters().await.ok().and_then(|a| a.into_iter().next()) {
        Some(c) => c,
        None => {
            push(&q, AdapterEvent::AdapterState { available: false, message: Some("No Bluetooth adapter was found. Turn Bluetooth on (or plug in an adapter), then restart Ridgeline.".into()) });
            drain_unavailable(rx, q).await;
            return;
        }
    };
    let mut central_events = match central.events().await {
        Ok(s) => s,
        Err(e) => {
            push(&q, AdapterEvent::AdapterState { available: false, message: Some(map_err(e).message) });
            drain_unavailable(rx, q).await;
            return;
        }
    };
    push(&q, AdapterEvent::AdapterState { available: true, message: None });
    let ctx = Arc::new(Ctx { central, q: q.clone(), by_id: Arc::new(Mutex::new(HashMap::new())) });
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                handle_cmd(ctx.clone(), cmd).await;
            }
            ev = central_events.next() => {
                let Some(ev) = ev else { break };
                match ev {
                    CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => announce(&ctx, &id).await,
                    CentralEvent::DeviceDisconnected(id) => {
                        push(&ctx.q, AdapterEvent::Disconnected { id: DeviceId(id_string(&id)), reason: Some("Bluetooth link lost".into()) });
                    }
                    CentralEvent::StateUpdate(state) => {
                        let on = matches!(state, CentralState::PoweredOn);
                        push(&ctx.q, AdapterEvent::AdapterState { available: on, message: if on { None } else { Some("Bluetooth is turned off.".into()) } });
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Answer every request with an error when no adapter is usable.
async fn drain_unavailable(mut rx: UnboundedReceiver<Cmd>, q: Queue) {
    let err = || AdapterError::new(AdapterErrorKind::AdapterUnavailable, "Bluetooth is unavailable");
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::Connect(id) => push(&q, AdapterEvent::ConnectFailed { id, error: err() }),
            Cmd::Read(id, ch, r) => push(&q, AdapterEvent::ReadResult { id, req: r, characteristic: ch, result: Err(err()) }),
            Cmd::Subscribe(id, ch, r) => push(&q, AdapterEvent::SubscribeResult { id, req: r, characteristic: ch, result: Err(err()) }),
            Cmd::Write(id, ch, _, _, r) => push(&q, AdapterEvent::WriteComplete { id, req: r, characteristic: ch, result: Err(err()) }),
            _ => {}
        }
    }
}

async fn handle_cmd(ctx: Arc<Ctx>, cmd: Cmd) {
    match cmd {
        Cmd::StartScan => match ctx.central.start_scan(ScanFilter::default()).await {
            Ok(()) => {
                push(&ctx.q, AdapterEvent::ScanStarted);
                // Announce already-known peripherals (e.g. paired/connected ones).
                if let Ok(ps) = ctx.central.peripherals().await {
                    for p in ps {
                        announce(&ctx, &p.id()).await;
                    }
                }
            }
            Err(e) => push(&ctx.q, AdapterEvent::AdapterState { available: false, message: Some(map_err(e).message) }),
        },
        Cmd::StopScan => {
            let _ = ctx.central.stop_scan().await;
            push(&ctx.q, AdapterEvent::ScanStopped);
        }
        Cmd::Connect(id) => {
            tokio::spawn(async move {
                let result = connect(&ctx, &id).await;
                match result {
                    Ok((p, services, chars)) => {
                        push(&ctx.q, AdapterEvent::Connected { id: id.clone(), services, characteristics: chars });
                        // Forward notifications until the stream ends (disconnect).
                        if let Ok(mut stream) = p.notifications().await {
                            while let Some(n) = stream.next().await {
                                if let Some(ch) = short(&n.uuid) {
                                    push(&ctx.q, AdapterEvent::Notification { id: id.clone(), characteristic: ch, data: n.value });
                                }
                            }
                        }
                    }
                    Err(error) => push(&ctx.q, AdapterEvent::ConnectFailed { id, error }),
                }
            });
        }
        Cmd::Disconnect(id) => {
            tokio::spawn(async move {
                if let Ok(p) = ctx.peripheral(&id).await {
                    let _ = p.disconnect().await;
                }
                push(&ctx.q, AdapterEvent::Disconnected { id, reason: Some("disconnected by app".into()) });
            });
        }
        Cmd::Read(id, ch, req) => {
            tokio::spawn(async move {
                let result = async {
                    let p = ctx.peripheral(&id).await?;
                    let c = find_char(&p, ch).ok_or_else(|| AdapterError::new(AdapterErrorKind::Unsupported, "characteristic not present"))?;
                    tokio::time::timeout(OP_TIMEOUT, p.read(&c)).await.map_err(|_| AdapterError::new(AdapterErrorKind::Timeout, "read timed out"))?.map_err(map_err)
                }
                .await;
                push(&ctx.q, AdapterEvent::ReadResult { id, req, characteristic: ch, result });
            });
        }
        Cmd::Subscribe(id, ch, req) => {
            tokio::spawn(async move {
                let result = async {
                    let p = ctx.peripheral(&id).await?;
                    let c = find_char(&p, ch).ok_or_else(|| AdapterError::new(AdapterErrorKind::Unsupported, "characteristic not present"))?;
                    tokio::time::timeout(OP_TIMEOUT, p.subscribe(&c)).await.map_err(|_| AdapterError::new(AdapterErrorKind::Timeout, "subscribe timed out"))?.map_err(map_err)
                }
                .await;
                push(&ctx.q, AdapterEvent::SubscribeResult { id, req, characteristic: ch, result });
            });
        }
        Cmd::Write(id, ch, data, with_response, req) => {
            // Writes are awaited in order on this task (the controller keeps
            // only one control-point command in flight anyway).
            let result = async {
                let p = ctx.peripheral(&id).await?;
                let c = find_char(&p, ch).ok_or_else(|| AdapterError::new(AdapterErrorKind::Unsupported, "characteristic not present"))?;
                let wt = if with_response { WriteType::WithResponse } else { WriteType::WithoutResponse };
                tokio::time::timeout(OP_TIMEOUT, p.write(&c, &data, wt)).await.map_err(|_| AdapterError::new(AdapterErrorKind::Timeout, "write timed out"))?.map_err(map_err)
            }
            .await;
            push(&ctx.q, AdapterEvent::WriteComplete { id, req, characteristic: ch, result });
        }
    }
}

async fn connect(ctx: &Ctx, id: &DeviceId) -> Result<(Peripheral, Vec<u16>, Vec<u16>), AdapterError> {
    let p = ctx.peripheral(id).await?;
    let already = p.is_connected().await.unwrap_or(false);
    if !already {
        tokio::time::timeout(CONNECT_TIMEOUT, p.connect())
            .await
            .map_err(|_| AdapterError::new(AdapterErrorKind::Timeout, "connection timed out"))?
            .map_err(map_err)?;
    }
    tokio::time::timeout(CONNECT_TIMEOUT, p.discover_services())
        .await
        .map_err(|_| AdapterError::new(AdapterErrorKind::Timeout, "service discovery timed out"))?
        .map_err(map_err)?;
    let services: Vec<u16> = p.services().iter().filter_map(|s| short(&s.uuid)).collect();
    let chars: Vec<u16> = p.characteristics().iter().filter_map(|c| short(&c.uuid)).collect();
    Ok((p, services, chars))
}
