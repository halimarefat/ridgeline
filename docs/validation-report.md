# Validation report

Version **0.1.0** (preview), branch `dev`. CI evidence: GitHub Actions workflow **CI** on the commit noted in each row. Automated checks run on every push; links point to the run used for this report.

**Summary.** All automated checks pass, including installing and launching the packaged app on Windows, macOS and Linux runners. **No real trainer or sensor has been tested**, so every hardware acceptance test (A02, A04, A07, A09 on hardware, A17 lifecycle on hardware) is open. All "pass" results below come from software, simulator or CI evidence and are labelled as such.

## Test environments

| Environment | Used for |
|---|---|
| Development sandbox (Linux x64, no Bluetooth, no registry access) | Rust workspace tests, UI build/typecheck/unit tests, Playwright walkthrough against the developer server (Chromium) |
| GitHub-hosted `ubuntu-24.04` | Core tests, FIT verification with Garmin's FIT JavaScript SDK, secrets scan, UI checks, Playwright end-to-end walkthrough |
| Owner's PC: Windows 11 Pro 10.0.26200, Intel Core Ultra 7 265K, 31 GB RAM, Bluetooth adapter, Ollama 0.35.1 | Installed release smoke test, Rust workspace tests, live local-AI run ([live-ai-validation.md](live-ai-validation.md)); hardware tests with a Tacx Flux 2 and Garmin HRM 200 pending |
| GitHub-hosted `windows-latest` (x64) | Desktop crate tests, NSIS + MSI build, silent per-user install and launch smoke test |
| GitHub-hosted `macos-latest` (Apple Silicon) | Desktop crate tests, universal (arm64 + x86_64) app + DMG build, DMG mount, copy and launch smoke test |
| GitHub-hosted `ubuntu-22.04` (x64) | Desktop crate tests, deb + AppImage build, apt install and launch under Xvfb smoke test |

## Automated results

| Check | Result |
|---|---|
| Rust workspace tests (`cargo test --workspace --release --locked`) | 129 passed, 0 failed (CI on Linux; also on the owner's Windows 11 PC with the `x86_64-pc-windows-gnu` toolchain, where they exposed the plain HTTP client's Windows defects fixed on 2026-10-05) |
| Golden packet fixtures (`fixtures/packets/golden.json`) | 5 test groups pass |
| FIT export decoded by the Garmin FIT SDK (independent decoder) | Pass: record count, laps, timer/elapsed time, average power, missing HR as missing, `virtualActivity` sub-sport |
| Secrets scan of tracked files and built UI | Pass |
| UI typecheck (TypeScript strict), unit tests (8), production build | Pass |
| End-to-end walkthrough (`apps/desktop/ui/e2e/demo-flow.mjs`, 29 steps) | Pass locally and in CI: welcome → demo → all 9 screens → simulated devices ready → offline plan proposed and accepted → calendar edit proposal accepted and undone → offline coach chat → ERG workout launched, keyboard pause/resume, stop and save with trainer stop confirmed → feedback → history with FIT export → route detail → all settings tabs. No page errors. |
| Desktop crate tests (Windows, macOS, Linux) | Pass |
| Installers built (unsigned) | Windows NSIS + MSI, macOS universal DMG, Linux deb + AppImage |
| Installed-app smoke test (A01) | See below |

### A01 installed-app smoke test

Each package is installed the way a rider would install it, then launched with `RIDGELINE_SMOKE_TEST`. The app must report that its bundled UI loaded and polled the native service over IPC, that demo mode opened, and that at least three simulated devices reached *ready*. Details are in `scripts/smoke-installed.sh`.

| Platform | Install method | Result |
|---|---|---|
| Windows x64 | Silent NSIS setup (`/S`, per user, under `%LOCALAPPDATA%`) → launch the installed exe | **Pass**: installed, UI loaded, demo opened with ≥ 3 simulated devices ready ([run 37316490332](https://github.com/halimarefat/ridgeline/actions/runs/37316490332)) |
| macOS universal (run on arm64) | Mount DMG (accept license) → copy `.app` → launch | **Pass** ([run 37316490332](https://github.com/halimarefat/ridgeline/actions/runs/37316490332)) |
| Linux x64 | `apt install ./Ridgeline_0.1.0_amd64.deb` → launch under Xvfb | **Pass** ([run 37316490332](https://github.com/halimarefat/ridgeline/actions/runs/37316490332)) |
| **Windows 11 Pro 10.0.26200, real desktop PC** (owner's machine) | Released `Ridgeline_0.1.0_x64-setup.exe` from [v0.1.0-preview.1](https://github.com/halimarefat/ridgeline/releases/tag/v0.1.0-preview.1), SHA-256 checked against `SHA256SUMS.txt`, silent per-user install (`/S`, no admin) → launch with `RIDGELINE_SMOKE_TEST` | **Pass** (2026-10-05): UI loaded over IPC, demo opened with 3 simulated devices ready, and the **BLE adapter reported available** (CI runners have none) |

Not covered: Intel Macs (the x86_64 slice is built but wasn't launched), Windows 10, real Bluetooth permission prompts, and upgrade/uninstall flows.

## Acceptance tests

| ID | Status | Evidence |
|---|---|---|
| A01 Clean installation | **Pass on CI runners** (see table above); manual check on rider machines open | `scripts/smoke-installed.sh`, CI annotations |
| A02 Multi-device pairing | **Open: needs hardware.** Logic passes in the simulator | `manager::full_connection_flow_and_simultaneous_sensors`; [checklist §1](hardware-test-checklist.md) |
| A03 Protocol parsing | **Pass** (software) | `fixtures/packets/golden.json`, `ftms.rs`/`sensors.rs` unit tests (flags, optional fields, signed values, truncation, rollover, units) |
| A04 ERG workout | **Simulator pass; hardware open** | `session_sim::a04_erg_workout_runs_pauses_and_finishes`, e2e walkthrough; [checklist §2](hardware-test-checklist.md) |
| A05 Road-grade control | **Pass** (software): flat / +5 % / −3 % produce the encoded grades 0, 500 and −300 (0.01 % units) and the matching displayed grades | `session_sim::a05_free_ride_sends_signed_grades`, `a05_signed_commands_follow_route_position`, golden simulation packets |
| A06 Elevation quality | **Pass** (software) | `route::spikes_duplicates_missing_and_reverse`, `segments_are_not_bridged`, `bridge_correction_flattens_dip`, `dateline_crossing`; missing elevation blocks simulation until fetched or flat fallback |
| A07 Physical free ride | **Open: needs hardware** | [checklist §3](hardware-test-checklist.md) |
| A08 Mode ownership | **Pass** (simulator) | `session_sim::a08_mode_switch_leaves_one_controller_and_drops_stale_commands`, `controller::stale_generation_commands_are_dropped` |
| A09 Disconnect under load | **Simulator pass; hardware open** | `session_sim::a09_disconnect_under_load_requires_controlled_resumption`, `controller::stop_without_ack_is_not_reported_as_success`; [checklist §4](hardware-test-checklist.md) |
| A10 Network/AI outage | **Pass** (software): rides use no network; AI outage falls back to the offline plan | `coach::a10_ai_outage_falls_back_to_offline_plan`, `app_flow::routes_free_ride_offline_and_coach`; [checklist §5](hardware-test-checklist.md) for a real offline ride |
| A11 AI validity | **Pass with fixtures and a live local model** (llama3.2 via Ollama on the owner's Windows 11 PC, 2026-10-05). The live run found and led to fixes for a plan that dropped 11 of 12 sessions and for token-limit truncation; the model obeyed an injected instruction and the validator rejected it. Details: [live-ai-validation.md](live-ai-validation.md) | `a11_dropped_or_added_days_are_rejected`, `a11_truncated_reply_is_reported_and_repair_does_not_echo_it`, `crates/coach/examples/live_local_model.rs`, plus `a11_valid_ai_plan_is_accepted`, `a11_invalid_json_gets_one_repair_then_fallback`, `a11_injection_text_is_quoted_and_symptoms_short_circuit`, `planner::tight_schedule_and_no_ftp`, `property_random_profiles_always_validate` |
| A12 Plan adaptation | **Pass** (software + e2e): feedback creates an explained, versioned proposal; nothing changes until accepted; undo works | `adapt::*`, `app_flow::demo_onboarding_plan_ride_export_recovery`, e2e calendar edit → accept → undo |
| A13 Recovery | **Pass** (software): an interrupted activity is detected and recovered with ≤ 2 s loss and no trainer restart; torn journal lines are skipped | `store::activity_crash_recovery`, `journal_survives_torn_line`, `app_flow::demo_onboarding_plan_ride_export_recovery`; force-quit on hardware in [checklist §6](hardware-test-checklist.md) |
| A14 Export | **Pass**: FIT decoded by the Garmin FIT SDK agrees with internal duration, laps and records; CSV keeps units in headers and missing values empty | CI FIT step, `csv::missing_values_are_empty_and_units_in_header`, `fit::structure_and_crc` |
| A15 Secrets/privacy | **Pass**: no keys in shipped assets; keys only in the OS credential store; diagnostics exclude profile, routes, locations and messages | `scripts/check-secrets.sh` in CI, `settings::defaults_are_valid_and_private`, [privacy.md](privacy.md) |
| A16 Long session | **Simulator pass** (accelerated two-hour session: 7,200 samples, no duplicated or skipped seconds, empty command queue); real two-hour ride open | `session_sim::a16_long_session_has_no_drift_or_duplication` |
| A17 Lifecycle | **Partial**: close during ride is intercepted (shell code); keep-awake implemented per OS; single-instance lock tested. Sleep/wake, minimize, Bluetooth off/on and low disk are not yet exercised on hardware | `app_flow::single_instance_lock`; [checklist §6](hardware-test-checklist.md) |
| A18 Accessibility | **Partial**: the ride works fully by keyboard (verified in e2e: Space, S); status uses text and icons, not colour alone; contrast follows the design tokens. No screen-reader session recorded | e2e walkthrough |

## Performance targets

Not measured on reference hardware. By construction, the native tick runs at 20 Hz on a monotonic clock and recomputes the target every tick, so a scheduled transition is queued within one tick (≤ 50 ms); this is not yet measured separately. Trainer acknowledgement latency is logged per command (Devices → Trainer control) and will be reported with the hardware tests.
