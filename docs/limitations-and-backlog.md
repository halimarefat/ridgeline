# Known limitations, costs and backlog

## Open release gates

| Gate | Why it's open | What the owner can do |
|---|---|---|
| Real-hardware validation (A02, A04, A07, A09, A16 on hardware, A17) | No physical trainer or sensor has been connected yet | Run [hardware-test-checklist.md](hardware-test-checklist.md) on Windows and macOS with your trainer |
| Broad compatibility claim | Needs ≥ 2 FTMS trainers from different makers plus separate HR and power/cadence sensors | Borrow/test more devices; record in [hardware-matrix.md](hardware-matrix.md) |
| Windows code signing | Requires a paid code-signing certificate (or a paid signing service) | Buy a certificate yourself and add it to the release workflow as a secret; Tauri's `bundle.windows.certificateThumbprint` / signing command supports it |
| macOS signing, hardened runtime, notarization | Requires the paid Apple Developer Program | Join the program yourself, then add `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific) and `APPLE_TEAM_ID` as repository secrets; Tauri signs and notarizes during `tauri build` |
| Coaching policy review | Limits chosen by the implementer from published sources | Have a qualified coach/clinician review [coaching-policy.md](coaching-policy.md) |
| Intel Mac and Windows 10 desktop launch | CI runs on Apple Silicon and Windows Server images; Windows 11 desktop passed on the owner's PC | Install on an Intel Mac and a Windows 10 PC |

Ridgeline did not and will not purchase anything. Paid integrations are off by default.

## Provider costs and quotas (as of October 2026, verify before relying on them)

| Service | Cost | Limits / terms |
|---|---|---|
| OpenFreeMap tiles | Free, no key | Donation-funded public instance; no hard rate limit published. Heavy use: self-host (open source). |
| Valhalla public server (FOSSGIS, valhalla1.openstreetmap.de) | Free, no key | Fair use for low volume; may rate-limit or change. Self-host for anything heavier. |
| Open-Meteo Elevation API | Free for non-commercial use, no key | Daily request limits apply to the free API (about 10,000 calls/day); commercial use requires a paid plan from Open-Meteo. |
| Ollama / LM Studio / llama.cpp | Free, runs locally | Needs RAM/VRAM for the model (≈ 2–3 GB for 3B models) |
| Remote OpenAI-compatible APIs | Usually paid per token | Disabled by default; daily request cap; you configure and pay the provider yourself |

## Known limitations

- **BLE only.** No ANT+ or proprietary protocols; virtual shifting, steering and climb accessories are not supported. Trainers without FTMS (some older models with only proprietary control) can't be controlled.
- **Resistance mode** encodes the level at 0.1 resolution per common practice; the scale differs between trainers and is unverified.
- **No spin-down calibration UI.** Use the manufacturer's app for calibration.
- **Storage is not encrypted** (relies on OS account protection). Not SQLite (see ADR-003).
- **Linux API keys** in the kernel keyring don't survive a reboot.
- **Custom map styles on other hosts** need a rebuild to update the content security policy.
- **Route builder** depends on the public Valhalla server's availability; there's no offline routing.
- **Elevation** from the 90 m Copernicus DEM smooths short steep pitches and can show bridges/tunnels as dips/humps. Corrections are manual.
- **Virtual speed** uses a fixed aerodynamic/rolling model (CdA 0.32, Crr 0.004). No drafting, no wind.
- **Workout editor** supports power (%FTP or watts), RPE and cadence cues; no heart-rate-target workouts or free-text intervals with ramps inside repeats.
- **AI plan summaries are not fact-checked.** The plan itself is validated, but the model's prose can misstate it (a live llama3.2 run claimed "one hard session per day" for a plan with 6 in 4 weeks). The weekly volume and hard-session counts the app shows before acceptance are computed locally and are authoritative.
- **AI plans keep the offline draft's training days.** The model may swap workouts and shorten sessions but not move, add or drop days; ask the coach chat to move a session instead.
- **No automatic updates** by design (updates must never apply during a ride). New versions come from the Releases page.
- **Accessibility** has been checked for keyboard operation of the ride, labels and non-colour status. No screen-reader session has been recorded yet.
- **Performance targets** (input < 100 ms, telemetry < 500 ms, map 30 fps) have not been measured on reference hardware. Control timing is verified in simulator tests only.
- **Long sessions (A16)** are covered by an accelerated two-hour simulated session test (timer drift, sample counts, command queue bounds). No two-hour real ride has been recorded.

## Prioritized backlog

1. Hardware validation with the owner's trainer on Windows and macOS; fix device quirks found.
2. Code signing and notarization once the owner provides credentials.
3. Live local-model runs with larger models and LM Studio (llama3.2 3B on Ollama is recorded in [live-ai-validation.md](live-ai-validation.md)); flag AI summary sentences that contradict the computed plan counts.
4. Coach/clinician review of the coaching policy; bump the policy version.
5. Screen-reader pass (NVDA, VoiceOver) and measured performance on reference hardware.
6. Spin-down calibration flow and per-trainer quirk table.
7. Offline route cache management UI and map tile caching for prepared routes.
8. ANT+ FE-C adapter (needs hardware and a licensing review).
9. Sync/export to intervals.icu or Strava (owner-configured OAuth, free tiers only).
10. Optional SQLite storage backend if history grows beyond thousands of rides.
