# Ridgeline

Ridgeline is a free, open-source indoor cycling app for Windows and macOS (Linux builds too). It connects to a Bluetooth smart trainer and sensors, builds and adapts a training plan with a coach that explains itself, runs structured ERG workouts, and lets you ride real mapped roads while the trainer follows the road's gradient. It is local-first: no account, no subscription, and nothing leaves your computer unless you turn on a map, routing or AI service.

> **Status: preview (0.1).** Everything below is implemented and runs in demo mode with simulated devices. **No physical trainer or sensor has been tested yet**, and the installers are **unsigned**. See [docs/validation-report.md](docs/validation-report.md) for exactly what was tested how, and [docs/hardware-matrix.md](docs/hardware-matrix.md) for device status.

| Area | State |
|---|---|
| Bluetooth FTMS trainer (ERG, road simulation, resistance), heart rate, cycling power, speed/cadence | Implemented with golden-packet tests and a fault-injecting simulator; **unverified on real hardware** |
| Onboarding, FTP history, 30 original workouts in 11 categories, workout editor | Implemented, tested |
| Calendar plans: offline rules-based planner + coaching policy; proposals you accept/undo | Implemented, tested |
| AI coach via a local model (Ollama, LM Studio or any OpenAI-compatible server on your computer) | Implemented; tested with fixtures and live with llama3.2 on Ollama ([report](docs/live-ai-validation.md)) |
| GPX import, map route builder, elevation fetch and cleaning, free rides with gradient simulation | Implemented, tested with synthetic routes |
| Recording, crash recovery, FIT/CSV/GPX export | Implemented; FIT verified with Garmin's FIT SDK decoder in CI |
| Windows x64 / macOS universal / Linux x64 packages | Built by GitHub Actions; each is installed and launched in CI (A01 smoke test). Unsigned |

## Quick start (riders)

1. Download the installer for your computer from the repository's **Releases** page:
   - Windows 10/11 (x64): `Ridgeline_<version>_x64-setup.exe` (or the `.msi`).
   - macOS 11 or later (Apple Silicon and Intel): `Ridgeline_<version>_universal.dmg`.
   - Linux x64: `.deb` or `.AppImage`.
2. Check the file against `SHA256SUMS.txt` if you like.
3. Install:
   - **Windows:** run the setup file. It installs for your user only (no administrator rights). Because the preview is not code-signed, Microsoft Defender SmartScreen may say it "protected your PC"; choose **More info → Run anyway** only if you downloaded it from this repository.
   - **macOS:** open the `.dmg` (accept the MIT license shown) and drag Ridgeline to Applications. Because the preview is not notarized, macOS blocks the first launch; open **System Settings → Privacy & Security** and choose **Open Anyway** for Ridgeline (this allows only this app; it does not turn off Gatekeeper). Allow Bluetooth when asked.
   - **Linux:** `sudo apt install ./Ridgeline_<version>_amd64.deb`, or make the AppImage executable and run it. BlueZ must be running.
4. Start Ridgeline and choose **Try a demo** to explore with a simulated trainer, power meter and heart-rate strap, or **Set up my bike** for the short onboarding interview.

Signed and notarized installers need paid publisher credentials (an Apple Developer membership and a Windows code-signing certificate). The project does not buy them; this is an open release gate (see [docs/limitations-and-backlog.md](docs/limitations-and-backlog.md)).

## Demo mode

Demo mode adds a simulated smart trainer, power pedals, heart-rate strap and cadence sensor, and two synthetic routes (a 4 km "grade steps" test route and a 12 km loop). Demo rides are labelled **DEMO** everywhere, never change your plan or FTP, and can be deleted from History. You can turn demo mode on or off in **Settings → Trainer & display**. On the **Devices** screen, the simulator panel lets you change rider power/cadence/heart rate and inject faults (disconnects, stale data, control denied, slow acknowledgements, malformed packets) to see how the app reacts.

## Using your own trainer and sensors

1. Wake the trainer by pedalling. Close other apps that might be connected to it (Zwift, the manufacturer app, a bike computer): most trainers accept only one controlling connection.
2. **Devices → Scan.** Connect the trainer and any heart-rate, power or cadence sensors. Several can be connected at once.
3. Press **Use as trainer** on the trainer. Its capabilities (ERG, road simulation, resistance, power range) are read from the trainer and shown.
4. Under **Data sources**, choose which device supplies power, cadence and heart rate (Automatic prefers the trainer).
5. Start a workout from **Home**, **Workouts** or **Calendar**, or a free ride from **Routes**. Ridgeline requests control only when the ride starts, ramps resistance in, and eases off and releases the trainer when you stop.

The space bar pauses and resumes, **S** stops, **N** skips an interval, **+ / −** change intensity, **L** marks a lap. If the trainer disconnects mid-ride, values are marked stale, the gap is recorded, and you must press **Resume control** before resistance returns (it ramps in over 10 seconds).

Please help fill in [docs/hardware-matrix.md](docs/hardware-matrix.md) with the [hardware test checklist](docs/hardware-test-checklist.md).

## Maps, routing and elevation (free services, configurable)

| Purpose | Default | Notes |
|---|---|---|
| Map tiles | [OpenFreeMap](https://openfreemap.org) (`https://tiles.openfreemap.org/styles/liberty`) | Free, no key. Turn off in Settings to draw routes without a background. |
| Route builder | [Valhalla](https://valhalla.github.io/valhalla/) public server by FOSSGIS (`https://valhalla1.openstreetmap.de`) | Free, fair-use; bicycle costing. You can point it at your own Valhalla. |
| Elevation | [Open-Meteo Elevation API](https://open-meteo.com/en/docs/elevation-api) (Copernicus DEM GLO-90) | Free for non-commercial use, no key. |

Only the coordinates needed for each request are sent. Rides never depend on the network: a prepared route rides offline, and the map falls back to a locally drawn line.

## AI coach (free, local)

The **offline coach** is always available: a deterministic planner and rule-based adaptation, reviewed against the [coaching policy](docs/coaching-policy.md). For open conversation and personalised explanations you can connect a model that runs on your own computer, at no cost:

1. Install [Ollama](https://ollama.com), then run `ollama pull llama3.2` (about 2 GB; any instruction model of 3B+ parameters works).
2. In Ridgeline: **Settings → AI coach → Local AI model**, keep the Ollama preset (`http://localhost:11434/v1`, model `llama3.2`), **Save**, then **Test connection**.
3. Turn on **Allow sending a compact summary…** under Consent. You can preview exactly what is sent.

The model can only propose plans; every proposal is validated against the policy locally (with one repair attempt), and nothing changes until you accept it. If the model is slow, unavailable or returns invalid output, the offline plan is used and you're told why. Warning symptoms (for example chest pain or fainting) are detected before any model is asked and always produce the same safety message.

**During rides** the coach sits on the Ride screen:
- **Ride cues** preview the next interval, flag cadence drifting outside its cue, announce climbs ahead, and offer **Easier 5 %** when you've been well under target. They work offline.
- **Quick prompts** are *How am I doing?*, *Too hard*, *Too easy* and *Motivate me* (keys **1–4**), plus a message box (**C**).
- With a local model, the coach also comments when hard intervals start, at halfway and before long climbs.
- Suggestions are buttons. Resistance changes only when you press one.
- An optional voice reads the coach aloud. Settings for all of this are under **Settings → AI coach → During rides**.

A **remote service** option exists for an OpenAI-compatible endpoint you configure yourself. It is off by default, needs an explicit opt-in, keeps its API key in the operating-system credential store, and is capped per day. Some remote services charge for use; Ridgeline never signs up, buys credits or enters payment details.

## Your data

Everything is stored as files in your user profile's app-data folder (shown in **Settings → Privacy & data**), protected by your operating-system account; it is not encrypted. You can export everything, export single rides as FIT/CSV/GPX, opt in to a diagnostics file, or delete all data. Details: [docs/privacy.md](docs/privacy.md).

## Development

Requirements: Rust (stable, 1.89+) and Node.js 22. The core crates use only the Rust standard library, so `cargo test` works offline.

```sh
scripts/rl.sh setup      # install UI dependencies (npm ci)
scripts/rl.sh test       # Rust tests + UI typecheck and unit tests
scripts/rl.sh demo       # build the UI and run it in your browser against the developer server (simulated devices)
scripts/rl.sh e2e        # browser end-to-end walkthrough (needs: npx playwright install chromium)
scripts/rl.sh app        # run the desktop app in development (Tauri; needs the platform prerequisites below)
scripts/rl.sh package    # build installers for this platform (unsigned)
```

On Windows use `scripts\rl.ps1 <command>` in PowerShell with the same commands.

Desktop prerequisites follow [Tauri's guide](https://v2.tauri.app/start/prerequisites/): on Windows, Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11); on macOS, Xcode Command Line Tools; on Linux, `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libdbus-1-dev pkg-config`.

The developer server (`cargo run -p rl-devserver`, port 1420) serves the built UI and the same command API as the desktop app, with simulated devices only. It's how the UI is developed and end-to-end tested without Bluetooth.

### Repository layout

```text
apps/desktop/ui          React + TypeScript UI (esbuild), MapLibre GL map
apps/desktop/src-tauri   Tauri 2 shell: btleplug BLE adapter, HTTP client, OS credential store, keep-awake
apps/devserver           Browser development server (simulated devices)
crates/json              Minimal JSON + derive macros (std only)
crates/domain            Workouts, library, plans, coaching policy, routes, elevation pipeline, physics, GPX
crates/device            FTMS / HR / CPS / CSC parsing, device manager, trainer control queue, simulator
crates/session           Session coordinator (control ownership), workout and route engines, recording
crates/storage           Local document store, crash-safe journals, FIT and CSV encoders
crates/coach             Offline planner, adaptation, AI provider interface (OpenAI-compatible), validation
crates/net               HTTP abstraction, routing and elevation providers
crates/app               Application service: command dispatcher, jobs, settings
fixtures/                FIT fixture and expected values
docs/                    Architecture, decisions, protocol, policy, privacy, configuration, validation
scripts/                 Task runner, CI helpers, FIT verifier, installed-app smoke test
```

### Tests and CI

GitHub Actions (free standard runners, public repository) runs on every push: the Rust workspace tests, FIT verification with Garmin's FIT JavaScript SDK, a secrets scan, UI typecheck/unit tests/build, a Playwright end-to-end walkthrough of the demo, and desktop builds for Windows x64, macOS universal and Linux x64. Each built package is then **installed and launched** on its runner, which must report that the UI loaded, talked to the native service and opened demo mode with simulated devices ready. Pushing a `v*` tag publishes the installers, checksums and smoke-test reports as a GitHub pre-release.

## Documentation

- [Architecture](docs/architecture.md) and [decision log](docs/decisions.md)
- [Bluetooth protocol notes](docs/protocol.md) and [hardware matrix](docs/hardware-matrix.md)
- [Coaching policy, rationale and sources](docs/coaching-policy.md)
- [Configuration reference](docs/configuration.md)
- [Privacy](docs/privacy.md) · [Troubleshooting](docs/troubleshooting.md)
- [Validation report](docs/validation-report.md) · [Live local-AI validation](docs/live-ai-validation.md) · [Hardware test checklist](docs/hardware-test-checklist.md)
- [Known limitations, costs and backlog](docs/limitations-and-backlog.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

Ridgeline is not affiliated with any trainer manufacturer or with Zwift or ROUVY. Training suggestions are not medical advice.

## License

MIT — see [LICENSE](LICENSE). Bundled fonts (Barlow) are under the SIL Open Font License 1.1.
