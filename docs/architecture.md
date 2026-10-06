# Architecture

Ridgeline is a single-user, local-first desktop application. Everything time-critical (trainer control, session timing, recording) runs in a native Rust service; the UI only displays state and sends validated commands. Map tiles, routing, elevation and AI are optional network services outside the control path.

```text
React UI (apps/desktop/ui)                 ── polls getState at 4 Hz, sends commands
   │  one Tauri command: rpc(method, JSON)  (desktop)   or  POST /rpc (developer server)
   ▼
rl-app  Runtime / App                      ── command dispatcher, background jobs, settings
   ├─ rl-session  SessionCoordinator       ── sole owner of the ride: state machine, control ownership,
   │     ├─ WorkoutEngine (ERG targets)       ramp-in, low-cadence protection, recording
   │     ├─ RouteEngine (position, grade)
   │     └─ Recorder → rl-storage journals
   ├─ rl-device  DeviceManager              ── connections, roles, telemetry freshness, source assignment
   │     ├─ TrainerController               ── FTMS control-point queue (one in flight, acks, timeouts)
   │     ├─ adapters: BLE (btleplug, desktop) · Simulator (deterministic, fault injection)
   │     └─ parsers: FTMS, Heart Rate, Cycling Power, CSC
   ├─ rl-domain  workouts, library, plans, coaching policy, routes, elevation pipeline, physics, GPX
   ├─ rl-coach   offline planner, adaptation rules, AI provider (OpenAI-compatible), validation + repair
   ├─ rl-net     HTTP abstraction, Valhalla routing, Open-Meteo elevation
   └─ rl-storage local document store, crash-safe journals, FIT/CSV encoders, single-instance lock
```

## Threads and timing

- **Runtime thread.** `rl_app::Runtime` owns the `App` behind a mutex and runs a 20 Hz tick (every 50 ms) on its own thread with a monotonic clock (`Instant`). The tick drains device events, advances the session, issues trainer commands and flushes the recorder. It does not depend on the UI, animation frames or window focus.
- **BLE thread.** The desktop BLE adapter runs a Tokio runtime with btleplug in its own thread and exchanges commands/events with the device manager through channels. Writes are acknowledged asynchronously; the device manager never blocks on Bluetooth.
- **Jobs.** Slow work (AI requests, route building, elevation fetches) runs as cancellable background jobs (`jobs.rs`) and reports progress; the UI polls `getJob`. A job never holds the app lock while waiting on the network.
- **UI.** Polls `getState` every 250 ms. Rendering cadence has no effect on control or recording.

Timestamps: samples carry the monotonic offset (`t`), active time (`a`) and UTC (`u`); activities store the timezone name and offset so calendar dates stay correct.

## Control path and ownership

Exactly one component — the `SessionCoordinator` — decides what the trainer should do, and only the `TrainerController` talks to the trainer's FTMS control point.

- Ride modes: `erg` (workout targets), `free_ride` (route simulation), `manual` (resistance level), `read_only` (no control). ERG with a route (`erg` + `route_id`) moves along the map while the workout keeps control; the terrain never changes resistance in that mode.
- Session states: `prepared → starting → running ⇄ paused → stopping → finished` (or `discarded`).
- Every command carries the session **generation**. Switching mode, pausing or stopping bumps the generation, so queued commands from the previous mode are discarded before they are sent (A08).
- **Newest-target-wins:** only the latest pending target is kept; targets are rate-limited (default ≥ 1 s apart) and each waits for the trainer's response (3 s timeout). A timed-out start/stop is reported as *uncertain*, never as success.
- **Ramp-in:** ERG targets ramp in over 10 s at start and after any resumption of control.
- **Low-cadence protection:** in ERG, once the rider has pedalled at 60+ rpm (re-armed at every start, resume and resumption of control, so starting from standstill never counts), cadence under 40 rpm for 5 s drops the target to a low load and posts a coach cue; full targets return only after the rider acknowledges and cadence has been above 60 rpm for 3 s, and then ramp in (prevents the "ERG death spiral").
- **Stop:** pending targets are dropped, a low-load target is sent, then FTMS *Stop*; the UI says whether the trainer confirmed it.
- **Loss of control or connection:** the session pauses, telemetry is marked stale, the gap is recorded, and control is only renegotiated when the rider presses **Resume control** (A09). A reconnecting trainer never silently resumes a hard target.

**In-ride coach (outside the control path).** Once per recorded second the session evaluates rule-based cues (`rl_session::ride_coach`) from the workout timeline, recorded samples and route profile, and appends them to a bounded feed that is part of the session state and recorded as `coach` events. Cues are text plus at most one *suggested* action (`intensity ±5` or `stop`); they never call the controller. Some cues are marked as moments. After the tick, the app may start one `ride_coach` background job (rate-limited, consent-gated) that asks the AI for a short comment (`rl_coach::ride`). Rider prompts (`rideCoach`) run the same way, or inline for the offline coach and the safety path. Replies land in the feed when the job finishes. A suggestion takes effect only when the rider presses its button, which calls the same `adjustIntensity` command as the **+ / −** keys (tagged `via: coach`).

Free-ride gradients come from the processed route profile at the rider's virtual position (`RouteEngine`), scaled by the difficulty setting, clamped to the configured grade range and slew-limited (default 1.5 %/s) before being sent as FTMS *Set Indoor Bike Simulation Parameters*. Virtual speed comes from a deterministic bicycle model (`rl_domain::physics`: gravity, rolling resistance Crr 0.004, aerodynamic drag with CdA 0.32 m² and ρ 1.225 kg/m³, drivetrain efficiency 0.976, a small rotating-mass allowance) using the rider's and bike's masses.

## Telemetry

Each metric (power, cadence, heart rate, trainer speed) has one assigned source, chosen automatically (trainer preferred) or by the rider. Readings carry an age and a freshness state: `fresh`, `stale` (older than the configured stale time, default 3 s), `disconnected` or `no_source`. Stale values are shown as stale, never as live, and are recorded as missing (null), not zero. Alternative sources are listed so the rider can compare a power meter with the trainer.

## Storage

See `crates/storage/src/store.rs`. Data lives in the platform app-data directory (`%APPDATA%\io.github.halimarefat.ridgeline` on Windows, `~/Library/Application Support/io.github.halimarefat.ridgeline` on macOS, `~/.local/share/io.github.halimarefat.ridgeline` on Linux) as versioned JSON documents written atomically (write temp file → fsync → rename), plus append-only JSON-lines journals for samples, events and laps. The journal is flushed every tick and fsync'd at least every two seconds; an interrupted ride is detected on the next launch and can be recovered or discarded (the trainer is never restarted automatically). A lock file enforces a single running instance. Why not SQLite: see [decisions.md](decisions.md#adr-003-file-based-document-store-and-journals-instead-of-sqlite).

## Command interface

The UI can only call named commands (about 80, listed in `crates/app/src/app.rs`) with JSON parameters that are validated in Rust: string lengths, numeric ranges, finite values, known enum values. Responses are `{"ok":true,"result":…}` or `{"ok":false,"error":"<message for the rider>"}`. The UI never sees GATT bytes, file paths it can write to, or secrets. In the desktop app this is the single Tauri command `rpc`; the Tauri capability set grants only `core:default` and the content security policy allows scripts only from the bundled UI and network connections only to the map tile host.

## Desktop shell

`apps/desktop/src-tauri` provides the platform pieces behind traits defined in the core:

- `DeviceAdapter` → btleplug (WinRT on Windows, CoreBluetooth on macOS, BlueZ on Linux);
- `HttpClient` → ureq with rustls (routing, elevation, local/remote AI);
- `Platform` → OS credential store via `keyring` (Windows Credential Manager, macOS Keychain, Linux kernel keyutils), keep-awake during rides (`SetThreadExecutionState` on Windows, `caffeinate` on macOS, `systemd-inhibit` on Linux);
- window close during a ride is intercepted; the UI asks the rider to stop and save first.

`RIDGELINE_SMOKE_TEST=<report.json>` puts the app into a self-test used by CI after installing the package (throwaway data folder, opens demo mode, writes a report, exits).

## Developer server

`apps/devserver` serves the built UI and the same `/rpc` command API over HTTP on localhost with only the simulator adapter. It's used for UI development and the Playwright end-to-end test. It binds to 127.0.0.1 only.

## Map

MapLibre GL JS renders the route, rider position and waypoints over a configurable vector style (default OpenFreeMap "liberty"). If tiles fail to load, the network is offline, the map is disabled, or the route is synthetic, the UI draws the route line locally as SVG. The elevation chart is always drawn locally from the processed profile.
