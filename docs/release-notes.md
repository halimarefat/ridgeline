# Ridgeline 0.1 preview — release notes

First public preview. **Unsigned builds** for evaluation, built and smoke-tested by GitHub Actions on Windows, macOS and Linux runners. No physical trainer has been tested yet. Please read the "Before you install" section.

## What's in it

- **Devices:** Bluetooth smart trainers (FTMS: ERG, road simulation, resistance level), heart-rate straps, power meters and speed/cadence sensors, several at once. You can choose the source for each metric. Stale data is shown as stale. The trainer command log is visible.
- **Onboarding and coach:** a short interview, FTP history (optional, never guessed from age), an offline rules-based planner bound by a written coaching policy, and an optional local AI model (Ollama / LM Studio / any OpenAI-compatible server on your computer). Every plan change is a proposal you accept or reject, and accepted changes can be undone. There's a readiness check and post-ride feedback, and warning symptoms trigger a safety stop.
- **Workouts:** 30 original workouts in 11 categories, a library with filters, a workout editor, ERG with ramp-in, low-cadence protection, intensity ±, skip and lap. You can also ride by perceived effort.
- **Routes:** GPX import, a route builder snapped to cycling roads, elevation fetch, a cleaning pipeline (spikes, duplicates, gaps, segments), manual corrections, reversed copies, and gradient-coloured profiles.
- **Free rides:** the trainer follows the road gradient with a difficulty setting, grade limits and smooth changes. You can also ride an ERG workout while moving along a map.
- **Recording:** crash-safe journals, recovery of interrupted rides, FIT (verified with Garmin's FIT SDK), CSV and GPX export.
- **Privacy:** local files only, no account, export everything, delete everything, opt-in diagnostics.
- **Demo mode:** simulated trainer and sensors with fault injection, plus synthetic routes.

## Before you install

- **Windows:** SmartScreen may warn because the installer isn't signed. Choose **More info → Run anyway** only for files from this release page whose SHA-256 matches `SHA256SUMS.txt`. The setup installs for your user only. WebView2 is downloaded if it's missing.
- **macOS 11+ (universal):** the app isn't notarized. After the first launch attempt, open **System Settings → Privacy & Security → Open Anyway**, then allow Bluetooth.
- **Linux:** `sudo apt install ./Ridgeline_*_amd64.deb`, or run the AppImage. BlueZ is required.
- `smoke-<platform>.json` files are the CI reports from installing and launching each package.

## Known limitations

See [docs/limitations-and-backlog.md](https://github.com/halimarefat/ridgeline/blob/main/docs/limitations-and-backlog.md). Most important: hardware compatibility is unverified, the builds are unsigned, and the coaching policy hasn't yet been reviewed by a coach or clinician.

Training suggestions are not medical advice.
