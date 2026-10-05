# Architecture decision log

Each entry records a decision, the alternatives considered, and why. Status is *accepted* unless noted.

## ADR-001 Tauri 2 + Rust native service + React UI

**Context.** The spec asks for Windows and macOS installers, native BLE, a time-critical control loop independent of the UI, and restricted web content.
**Decision.** Tauri 2 shell, Rust core, React + TypeScript UI bundled locally, MapLibre GL JS for maps (the spec's recommended stack).
**Why.** Small installers using the system webview (WebView2 / WKWebView / WebKitGTK), a capability system that lets us expose a single command, and Rust for the BLE and control code. Electron would bundle Chromium (larger, weaker isolation for this use); a fully native UI per platform would double UI work.

## ADR-002 Core crates use only the Rust standard library

**Context.** Development happened in a sandbox without access to crates.io; CI has registry access.
**Decision.** All domain, device, session, storage, coach, net and app crates depend only on `std` and a small in-repo JSON crate (`rl-json`, with `json_struct!`/`json_enum!` derive-style macros). Third-party crates (tauri, btleplug, tokio, ureq, keyring) are confined to `apps/desktop/src-tauri`, a separate Cargo workspace.
**Why.** The whole core (≈120 tests) builds and runs offline, which also gives a tiny dependency surface for the safety-relevant code. Cost: a hand-written JSON layer and FIT encoder; both are covered by tests and the FIT output is verified by Garmin's SDK in CI.

## ADR-003 File-based document store and journals instead of SQLite

**Context.** The spec recommends SQLite with WAL and migrations.
**Decision.** Versioned JSON documents written atomically (temp file, fsync, rename) and append-only JSON-lines journals for ride samples, events and laps (`crates/storage`). The store has a version (`store.json`) with a tested migration step on open, and a store written by a newer Ridgeline is refused rather than modified; documents carry their own schema version and missing fields take documented defaults.
**Why.** SQLite would need a C library via a third-party crate, which conflicts with ADR-002 for the core. The data is small (a two-hour ride is ≈7,200 samples) and access patterns are simple (whole documents, append-only rides), so a document store gives the same crash-safety properties: the journal is flushed every tick and fsync'd at least every 2 s, partial trailing lines are ignored on recovery, and writes never tear existing files. **Trade-offs:** no ad-hoc queries; the history list reads one summary file per ride (fine for thousands of rides). Moving to SQLite later is a contained change behind `Store`.

## ADR-004 No-cost map, routing and elevation providers, all replaceable

**Decision.** OpenFreeMap tiles (no key), Valhalla on the FOSSGIS public server for route building (bicycle costing), Open-Meteo Elevation API (Copernicus GLO-90) for elevation. Each URL is configurable and can be turned off; rides never need the network once a route is prepared.
**Why.** The no-payment restriction rules out keyed commercial providers. These services require no account or key for the intended low-volume, non-commercial use. Their fair-use expectations are documented in [limitations-and-backlog.md](limitations-and-backlog.md); heavy users should self-host (all three are open source).

## ADR-005 Coach = offline planner + optional local model via an OpenAI-compatible adapter

**Decision.** The deterministic planner and rule-based adaptation are the default and the fallback. The single real AI adapter speaks the OpenAI-compatible Chat Completions API, which Ollama, LM Studio, llama.cpp server and most hosted services offer. "Local" mode accepts only localhost endpoints; "remote" mode is disabled until the rider explicitly enables it and keeps the API key in the OS credential store. Every AI output is parsed as JSON, validated against the coaching policy and the rider's schedule, given at most one repair attempt with the validation errors, and otherwise replaced by the offline plan with the reason shown.
**Why.** Meets the "one real provider adapter" requirement without any paid service: live validation can use a free local model. The AI can only propose; it never touches trainer control.

## ADR-006 Unsigned installers built on GitHub's free runners

**Decision.** A public repository, standard GitHub-hosted runners (free for public repositories), Tauri bundler targets NSIS + MSI (Windows x64), universal DMG (macOS arm64 + x86_64), deb + AppImage (Linux x64). Releases are GitHub pre-releases with SHA-256 checksums.
**Why.** Code signing (Windows certificate) and notarization (Apple Developer Program) cost money and are outside the no-payment restriction. They are explicit open release gates; the workflow has a place to add them when the owner supplies credentials.

## ADR-007 UI polls state at 4 Hz instead of streaming events

**Decision.** The UI calls `getState` every 250 ms; the response contains devices, live readings with ages, the session and notices.
**Why.** One code path for the desktop app (Tauri IPC) and the developer server (HTTP), trivially testable, and no event-ordering bugs. The payload is a few kilobytes; at 4 Hz this is negligible. Control timing is unaffected because it runs in the native tick.

## ADR-008 Minimum OS versions

**Decision.** Windows 10 1809+ x64 (WebView2 is installed by the setup if missing), macOS 11+ (universal binary), Ubuntu 22.04+ x64.
**Why.** These are Tauri 2's supported baselines; btleplug supports WinRT Bluetooth LE on Windows 10 and CoreBluetooth on macOS 11. **Verified:** installation and launch on the GitHub-hosted runner images (`windows-latest`, `macos-latest` on Apple Silicon, `ubuntu-22.04`) by the CI smoke test, where it passes. **Not verified:** Windows 10/11 desktop editions and Intel Macs, which need a manual check.

## ADR-009 Single resistance owner and session generations

**Decision.** Only the session coordinator decides trainer targets; the trainer controller serializes control-point writes and drops commands from an older generation.
**Why.** Prevents the classic bugs of two features fighting over the trainer (ERG vs. simulation) and stale targets arriving after a mode switch or stop (A08).

## ADR-010 Policy-bounded coaching with a written, versioned policy

**Decision.** All training limits live in `crates/domain/src/policy.rs` with a version string recorded on every plan and proposal; the rationale and sources are in [coaching-policy.md](coaching-policy.md). Warning-symptom detection runs before any AI call.
**Why.** The plan must be explainable and safe even if a model misbehaves; changing a limit is a reviewed, versioned change. **Open item:** review by a qualified coach/clinician.
