# Hardware and platform matrix

Status labels: **tested** — exercised end to end on that exact combination with results recorded below; **partial** — some functions exercised; **unverified** — expected to work from the standard, never run. Standards support is not a compatibility guarantee: trainers differ in which FTMS features they implement and how they behave.

## Devices

| Device (model / firmware) | OS | Transport | Telemetry | ERG | Simulation | Resistance | Status | Notes |
|---|---|---|---|---|---|---|---|---|
| Ridgeline simulator (trainer, power pedals, HR strap, cadence sensor) | Windows, macOS, Linux | in-process | yes | yes | yes | yes | **tested** (logic only) | Automated tests, e2e walkthrough and installed-app smoke test. Proves Ridgeline's logic, not any real device. |
| Owner's FTMS smart trainer (model TBD) | — | BLE | — | — | — | — | **unverified** | To be tested with [the checklist](hardware-test-checklist.md) |
| Any BLE heart-rate strap (0x180D) | — | BLE | — | n/a | n/a | n/a | **unverified** | |
| Any BLE power meter (0x1818) | — | BLE | — | n/a | n/a | n/a | **unverified** | |
| Any BLE speed/cadence sensor (0x1816) | — | BLE | — | n/a | n/a | n/a | **unverified** | |

The spec's bar for claiming broad compatibility — two FTMS trainer models from different manufacturers, separate HR and cadence/power sensors, and one trainer on both Windows and macOS — has **not** been met.

## Platforms (installers)

| Platform | Package | Built in CI | Installed + launched + demo opened (CI smoke test) | Bluetooth on real hardware | Signed |
|---|---|---|---|---|---|
| Windows x64 (`windows-latest` runner) | NSIS setup (per-user), MSI | yes | see [validation report](validation-report.md) | unverified | no (gate) |
| macOS universal (`macos-latest`, Apple Silicon runner) | DMG | yes | see validation report | unverified | no (gate: signing + notarization) |
| macOS Intel | same universal DMG | yes (x86_64 slice) | not run | unverified | no |
| Linux x64 (`ubuntu-22.04`) | deb, AppImage | yes | see validation report | unverified | n/a |

## How to add a result

Run the [hardware test checklist](hardware-test-checklist.md), then add a row with the exact trainer model and firmware (Devices screen shows both), OS version, Ridgeline version, which modes worked, and anything odd (for example "resistance level scale is 0–100, not 0–200", "simulation grade capped at 10 %", "needs 2 s between targets"). Attach the diagnostics export (Settings → Privacy & data) to an issue so the command log can be reviewed.
