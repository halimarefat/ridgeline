# Ridgeline 0.1 preview — release notes

## 0.1.0-preview.4 (2026-10-05)

- **Auto-pause:** stop pedalling for 3 s and the ride pauses (timer and workout stop, the trainer goes to a low load). Start pedalling and it resumes, with the target ramping back in. A ride started before you pedal waits for you ("Ready when you are"). Coasting downhill on a free ride doesn't count. Settings → Trainer & display → Rides; it can be changed mid-ride.
- **Your power on the workout profile:** the Ride screen's workout chart draws your actual power over the interval blocks, so you can see how closely you're following them. Skipped sections are left as gaps.

## 0.1.0-preview.3 (2026-10-05)

- **Fix: ERG workouts started before pedalling stayed at about 40 % of the target.** Found on a real ride with a Tacx Flux 2. Low-cadence protection now arms only after you've pedalled at 60+ rpm, and re-arms on every start, resume and reconnect, so a standstill start is never treated as a stall.
- When the protection does ease the load mid-effort, the banner now describes what happens accurately. **Resume target** has a big button, an **R** key, and a coach cue with the same button (read aloud if voice is on).

## 0.1.0-preview.2 (2026-10-05)

- **Coach on the Ride screen.** Ride cues preview the next interval, flag cadence drifting outside its cue, announce climbs ahead and the top, and offer **Easier 5 %** when you've been well under target. One-tap prompts (*How am I doing?*, *Too hard*, *Too easy*, *Motivate me*; keys **1–4**) and a message box (**C**) get answers from the offline coach or, with consent, your local AI model, which also comments at key moments. Suggestions are buttons: nothing changes until you press one. An optional voice reads the coach aloud. Settings → AI coach → *During rides*.
- AI plans must keep every training day of the offline draft. Truncated model replies are detected, and the repair prompt fits small context windows (found in a live llama3.2 run).
- The core's local HTTP client reaches Ollama/LM Studio on Windows (`localhost` → IPv4) and reports a service that isn't running as "not reachable".

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
- **macOS 11+ (universal):** opening the disk image shows the MIT license; choose Agree. The app isn't notarized. After the first launch attempt, open **System Settings → Privacy & Security → Open Anyway**, then allow Bluetooth.
- **Linux:** `sudo apt install ./Ridgeline_*_amd64.deb`, or run the AppImage. BlueZ is required.
- `smoke-<platform>.json` files are the CI reports from installing and launching each package.

## Known limitations

See [docs/limitations-and-backlog.md](https://github.com/halimarefat/ridgeline/blob/main/docs/limitations-and-backlog.md). Most important: hardware compatibility is unverified, the builds are unsigned, and the coaching policy hasn't yet been reviewed by a coach or clinician.

Training suggestions are not medical advice.
