# AI Indoor Cycling App — Design and Implementation Brief

Version: 1.1 • Prepared: October 4, 2026 • Updated: October 5, 2026

## 1. Instructions to the AI building this app

Act as the product designer, desktop software engineer, Bluetooth integration engineer, and test engineer for this project. Build a usable indoor cycling application for Windows and macOS from this specification. Deliver working source code, tests, setup instructions, and native installation packages. A web mockup, simulated dashboard alone, or source repository without a working desktop build does not satisfy the project.

Work through the milestones below. Make reasonable, reversible engineering decisions and document them. Ask only for missing information that blocks progress, such as access to a physical trainer or existing release signing credentials. Continue independent work while those are unavailable. Never claim that a physical device, operating system, installer, or integration was tested unless it actually was.

### Mandatory restriction: no purchases or payment actions

The implementing AI has no authorization to spend money or perform payment-related actions. This restriction overrides any milestone or requirement below that would otherwise involve a charge.

- Do not purchase hardware, software, licenses, domains, certificates, developer memberships, credits, or services.
- Do not start subscriptions, paid plans, auto-renewing trials, or trials requiring a payment method. Do not enter, retrieve, or submit payment-card or banking details, accept paid terms, change billing settings, or upgrade accounts.
- Do not provision billable cloud infrastructure, paid CI runners, storage, hosting, map services, or AI services. Do not make metered requests that consume paid credits or can create charges, even if credentials or a payment method already exist.
- Use local tools, simulators, local models where feasible, or verified no-charge services. A free tier is acceptable only when the intended usage cannot incur charges or automatically upgrade; if uncertain, use a local fixture or mock instead.
- Existing credentials are not spending authorization. Use them only for operations verified to incur no additional charge. Do not ask the owner to paste secrets into chat.
- You may research costs, write integration code, prepare configuration templates, and explain optional paid dependencies. Leave all purchases, paid activation, and billing actions for the owner to perform personally.
- If a requirement needs a paid service, finish the code and no-cost tests, document the owner's setup steps and the unverified integration, and continue other work. Never mark a mocked paid integration as live-tested.
- Keep paid integrations disabled by default in the delivered app. Do not implement automatic subscription, top-up, purchase, or billing flows. The owner may separately configure a service for their own later use; this does not authorize the implementing AI to incur charges while building or testing.

This is an instruction boundary, not a technical guarantee of enforcement. Where the coding environment permits, also disable purchasing/billing tools and withhold payment credentials. The app must remain demonstrable without any purchase. Public signing/notarization, paid-provider validation, and other dependent release gates may remain explicitly blocked without preventing delivery of the completed no-cost work.

This document specifies the product to build; it is not evidence that the app already exists. The requirements come from the owner's current description. Details from an earlier app discussion were not available. Suggested architecture and defaults below are recommendations, not additional owner commitments.

### Product vision

Create an approachable indoor cycling app that connects to a smart trainer and cycling sensors, interviews the rider, generates an appropriate training plan, runs structured workouts, and provides free rides on real mapped roads. During a free ride, the rider moves along the route and trainer resistance responds to the road's elevation profile. A clear map and elevation chart replace an animated virtual world.

Zwift and ROUVY are functional references for indoor riding, not sources of proprietary assets, routes, branding, or interfaces to copy.

### Non-negotiable outcomes

- Install and run as a desktop app on Windows and macOS without requiring the rider to install developer tools.
- Connect simultaneously to a compatible smart trainer, heart-rate sensor, and additional supported cycling sensors.
- Control a compatible trainer in ERG mode for workouts and simulation mode for road gradients.
- Provide an AI coach that uses onboarding answers, rider level, available time, preferences, and completed activity to propose and adapt a plan.
- Offer multiple workout categories and an editable workout library.
- Offer map-based free rides with elevation-derived gradients, realistic progression, and visible terrain limitations.
- Keep ride execution, recording, and trainer control operational without an AI service or internet connection once the necessary content is downloaded.
- Handle interruption, disconnection, and restart without silently corrupting activity data or unexpectedly restoring a hard resistance target.

## 2. Scope and default decisions

### First complete release

Single-rider, local-first desktop app with no required account. Support Bluetooth Low Energy (BLE) FTMS trainers, BLE heart rate, cycling power, and cycling speed/cadence sensors. Include onboarding, training calendar, AI planning, structured workouts, GPX route import, map route selection, a minimal start/end/waypoint route builder, free riding, activity history, and FIT/CSV export.

Build Windows x64 and macOS Apple Silicon and Intel packages. Select minimum supported OS versions during the initial feasibility milestone using the actual dependency support matrix; document and test them. Windows ARM and mobile apps are later work unless explicitly added.

Provide an obvious demo mode with simulated hardware and a bundled synthetic route so the app can be evaluated without hardware, accounts, maps credentials, or AI credentials. Never label demo data as a real ride.

### Later extensions

ANT+ FE-C and ANT+ sensors, proprietary trainer protocols, external power matching, virtual shifting, steering, climb accessories, social/group rides, multiplayer racing, video routes, cloud sync, external activity-platform sync, voice coaching, and mobile companion apps. Keep adapter boundaries for these, but do not let them delay the core release. ANT+ is separate from BLE and requires its own hardware, driver, protocol, and licensing assessment.

### Initial assumptions to confirm when convenient

The owner's trainer model, firmware, available sensors, target Mac/Windows machines, map-service budget, and preferred AI provider are unknown. Start with standards-based BLE support and provider interfaces. Do not assume every smart trainer implements every feature or that a specific brand is compatible without testing.

## 3. Rider journeys and interface

### First launch

Explain the app in one screen; offer “Set up my bike” and “Try a demo.” Ask Bluetooth permission when needed, explain how to wake sensors, scan, and let the rider choose device roles. Show live values and supported control modes before starting a ride. Present onboarding in short steps with progress and save/resume.

### Primary screens

| Screen | Required content and actions |
|---|---|
| Home | Today's proposed ride, readiness check, start workout, free ride, device status |
| Devices | Scan/connect, role selection, signal/last update, battery when available, supported modes, reconnect help |
| Coach | Conversational questions, proposed plan, reasoning, change preview, accept/reject |
| Calendar | Weekly plan, rest days, move/replace/shorten workouts, completed versus planned |
| Workouts | Search/filter, category, duration, target profile, difficulty, create/edit/duplicate |
| Routes | Map, import GPX, start/end/waypoints, distance, climbing, grade profile, download readiness |
| Ride | Large power/cadence/HR/time, interval or map, progress/elevation, pause, stop, mode and connection status |
| History | Summary, charts, laps, route, adherence, subjective feedback, export/delete |
| Settings | Units, rider/bike mass, FTP history, devices, trainer difficulty, providers, privacy, storage, updates |

Design for a rider viewing a laptop from a bicycle: large numbers, strong contrast, large click targets, minimal text entry during rides, keyboard support, and accessible color choices. Support km/miles and kg/lb while storing SI units internally. Normal use must not require a terminal.

Keep Stop and Pause visible at all times during a ride. Provide keyboard shortcuts without triggering them while typing in coach chat. Show disconnected/stale values explicitly, never as plausible current measurements. Preserve the ride view when an AI request fails.

## 4. Onboarding and training logic

### Collect only useful information

- Goal: general fitness, endurance, climbing, event preparation, FTP improvement, returning to riding, or maintaining fitness.
- Experience level, recent weekly riding time, consistency over recent weeks, and time away from training.
- Available days, session lengths, preferred long-ride day, rest days, and competing activities.
- Current FTP if known, test method/date, confidence, and optional known heart-rate thresholds. Never invent these from age alone.
- Rider and bike mass for simulation, units, and optional age range where relevant to the reviewed coaching policy.
- Equipment, trainer modes, available sensors, and whether power data is measured or estimated.
- Workout preferences, event date if any, fatigue/sleep/stress feedback, injuries or other limitations the rider chooses to disclose.
- Consent for sending selected profile and activity summaries to an external AI service.

Do not require detailed medical records. A missing FTP must not prevent easy riding. Offer conservative perceived-effort guidance and an optional guided assessment appropriate to the rider; never make a maximal test mandatory. Store FTP as a dated value and identify provisional estimates.

### Plan generation

Generate an initial four-week plan by default, adjustable to the rider's schedule and goal. Use a reviewed template/rules library for session selection, progression, recovery, and eligibility. The AI explains and customizes within those constraints. Do not let a language model invent unrestricted workloads.

Each planned session includes date, duration, workout reference, target basis, purpose, difficulty, alternatives, and a short explanation of why it fits this rider. Show weekly volume and hard-session count before acceptance. Respect available time and rest days; avoid stacking missed sessions into later days automatically.

Create a versioned coaching-policy configuration covering beginner/returning/experienced riders, intensity eligibility, recovery spacing, maximum permitted progression, and symptom-related exclusions. Numerical training limits must be sourced and reviewed before release, not improvised by the model. These are coaching rules, not medical diagnoses or guarantees.

Use a pre-ride readiness check and post-ride effort/enjoyment/fatigue feedback. Propose shortening, substituting, or rescheduling when appropriate. A plan change takes effect only after the rider accepts it. Record the prior plan, proposal, explanation, and acceptance so changes can be understood and undone.

Never increase resistance directly because of a chat response. During an active ride the coach may suggest an adjustment; the rider explicitly accepts it through the normal ride controls. If a rider reports concerning symptoms, stop performance-oriented recommendations and follow reviewed safety copy. Do not diagnose from heart-rate trends.

### AI implementation contract

- Define an `AIProvider` interface; implement one real configurable provider and an offline deterministic planner. Label the latter clearly as an offline plan, not an AI conversation.
- Use validated structured outputs for onboarding summaries and plan proposals, with a versioned JSON Schema and allowed workout IDs.
- Validate dates, schedule feasibility, durations, units, target ranges, progression policy, and session eligibility locally before presenting a plan.
- Reject invalid output; permit at most one bounded repair attempt before using the deterministic fallback.
- Use request timeouts, cancellation, bounded retries, cost limits, and concise user-visible error messages.
- Limit tools to reading approved summaries, finding workouts, and proposing changes. No arbitrary code execution, database queries, filesystem access, or direct trainer commands.
- Treat route names, imported notes, and external text as untrusted content, not instructions.
- Keep model/provider configuration replaceable. Store secrets in the operating-system credential store; never in frontend bundles or logs.
- For a personal first release, support an optional user-supplied API key for the owner's later configuration. During implementation, use a local model, a verified no-charge provider, or mocks under the no-payment restriction above. A distributed commercial version must not embed a shared provider secret; document any future authenticated backend design without provisioning billable services.
- Send compact summaries by default, not raw second-by-second records, location history, or unnecessary personal details. Cloud inference is opt-in; accepted plans remain available offline.

## 5. Workouts and ride modes

Include original, reviewed templates for recovery, aerobic endurance, tempo, sweet spot, threshold, over-unders, VO2-style intervals, cadence drills, climbing preparation, and optional assessment sessions. Include short/medium/long variants where appropriate. Advanced anaerobic/sprint sessions must be explicitly gated by the coaching policy and trainer limits.

Use a versioned workout definition with warm-up, steady intervals, repeats, ramps, recovery, cool-down, optional cadence cues, and instructional text. Store power targets as percentage-of-FTP or absolute watts, and allow perceived-effort targets for riders without reliable power. Calculate derived watts from the session's saved FTP snapshot so historical workouts do not change when FTP changes.

The editor must validate total duration, repeat counts, target ranges, and sensor/control requirements. Supply at least 20 distinct runnable workout templates across at least eight categories by the first complete release. Short technical test workouts are separate fixtures, not part of that count.

| Mode | Resistance authority | Behavior |
|---|---|---|
| ERG workout | Workout engine | Send supported target watts; rider follows cadence cues |
| Manual resistance | Rider | Set supported resistance level; record all available metrics |
| Free ride / simulation | Route engine | Send validated road simulation parameters as the rider advances |
| Workout with map background | Workout engine | Map may advance, but terrain does not also control resistance |
| Read-only ride | None | Record/display sensors; clearly indicate no trainer control |

Exactly one controller owns resistance at a time. Mode transitions are explicit, acknowledged, and logged. Allow pause/resume, skip interval, modest intensity adjustment, and early finish. Store intensity changes separately from the original workout. Do not silently fall back from simulation to ERG as if they were equivalent.

## 6. Trainer and sensor integration

### Native BLE, not browser-only Bluetooth

Implement Bluetooth in a native service. Recommended stack: Tauri 2, Rust, and `btleplug`, with platform-specific adapters if the feasibility tests reveal gaps. `btleplug` lists Windows and macOS support, but application-specific behavior still needs physical testing. See [btleplug project documentation](https://github.com/deviceplug/btleplug).

Base FTMS control on the official [Bluetooth Fitness Machine Service specification](https://www.bluetooth.com/specifications/specs/fitness-machine-service-1-0-1/), and verify related GATT definitions using [Bluetooth assigned numbers](https://www.bluetooth.com/wp-content/uploads/Files/Specification/Assigned_Numbers.pdf). Protocol details must come from the specification and tested packets, not language-model memory.

| Role / service | Baseline behavior |
|---|---|
| FTMS, `0x1826` | Discover capabilities/ranges; consume Indoor Bike Data; request control; start/stop as supported; set target power and simulation parameters |
| Cycling Power, `0x1818` | Parse measured power and optional crank/wheel data correctly |
| Heart Rate, `0x180D` | Parse 8/16-bit measurements and optional fields according to flags |
| Cycling Speed and Cadence, `0x1816` | Compute speed/cadence from counter and event-time deltas; handle wraparound |
| Battery / Device Information | Show supported battery, manufacturer, model, and firmware information |

For FTMS, implement feature discovery, control-point indications, result-code handling, supported ranges, machine-status notifications, and a serialized command queue. Subscribe before requesting control. Match each response to its request, use timeouts, and handle denied control or ownership loss. Distinguish transport write success from machine acceptance. Do not issue an unbounded stream of commands.

Choose one active source per metric and one controllable trainer. Allow the same device to fill several roles. Let the rider choose an external power meter, independent cadence, and HR sensor. Recording an external power meter does not imply power matching: initial ERG targets remain trainer-controlled and any discrepancy is visible. Deduplicate observations from multiple services and persist source provenance.

Store device preferences without assuming a BLE MAC address is a stable cross-platform identity. Handle adapter unavailable, permission denied, sleeping sensors, device already occupied by another app, unsupported modes, low battery, dropouts, and reconnect. Explain that another training app may hold the trainer connection.

Calibration/spindown is offered only when supported and implemented according to the relevant device procedure. Otherwise link to a tested manufacturer procedure. Never send guessed proprietary commands.

### Connection/control state machine

Implement explicit states: `disconnected -> scanning -> connecting -> discovering -> subscribing -> ready -> requesting_control -> controlled -> riding -> paused/stopping`, with `reconnecting` and `faulted` paths. Separate a sensor's connection state from the session's control state. A reconnected device does not automatically regain permission to resume a hard target.

Recommended initial engineering parameters, to tune using hardware tests:

- Mark a normally 1 Hz metric stale after 3 seconds; make per-device thresholds configurable.
- Serialize control commands, start with a maximum 1 Hz target update, and use device-specific limits when known.
- Use a bounded acknowledgment timeout, initially 3 seconds. Treat timed-out state as uncertain and reconcile it; do not blindly retry start/stop operations.
- Drop obsolete pending target updates so a reconnection cannot replay a queue of old hills or intervals.
- Stamp commands with session generation and monotonic time; reject commands from previous sessions.

Stop/pause must enter the controller's stopping path immediately, discard pending intensity increases, and attempt a tested low-load/stop action while connected. Confirm acknowledged state or show that control is unknown. A disconnected trainer may retain its previous resistance: the app cannot guarantee a physical stop without communication. Tell the rider to stop pedaling if needed; never present a successful emergency stop without evidence.

Detect prolonged low/zero cadence in ERG using reliable cadence or an explicitly tested substitute. Implement a validated low-load recovery behavior to avoid escalating load while a rider struggles. Do not invent cadence when absent. Recovery requires clear rider feedback and a controlled ramp.

## 7. Map rides and road-gradient simulation

### Route sources

Support GPX tracks/routes, bundled test routes, and a basic map route builder using a routing-provider interface. A route builder must follow routable roads, not straight lines between arbitrary map clicks. Preview the route before riding; show distance, climbing, grade distribution, road/surface metadata where available, and elevation quality.

Implement separate interfaces for base-map tiles, road routing, and elevation. A map rendering library is not a routing engine or an authoritative source of road elevations. Use MapLibre GL JS for the map and an explicitly selected, licensed provider for routing/elevation. Choose and document at least one real provider combination during implementation, including credentials, quotas, caching rights, attribution, and expected cost. Provide setup instructions and a usable no-key demo.

MapLibre provides map rendering functionality; consult its [official documentation](https://maplibre.org/maplibre-gl-js/docs/). For tile hosting, use a service that permits the intended usage or self-host. Do not bulk-download the public OpenStreetMap standard tile service for offline routes: its [tile usage policy](https://operations.osmfoundation.org/policies/tiles/) prohibits that use.

### Elevation pipeline

1. Parse and validate route coordinates; handle missing elevations, duplicate points, malformed XML, disconnected track segments, dateline crossings, and unreasonable sizes. Disable XML external-entity expansion.
2. Preserve the route's track segments; do not create an artificial rideable jump across missing geometry.
3. Compute cumulative horizontal ground distance in meters along the selected route.
4. Use credible imported road elevations when available; otherwise sample a licensed elevation dataset. Save source, resolution, timestamp/version, and quality flags.
5. Resample at approximately 10-meter intervals as an initial target, adjusted for source resolution. More samples must not be presented as more accurate source data.
6. Remove isolated spikes and smooth the elevation profile in distance space. Start with a robust filter and roughly 30–50-meter slope window; validate against known climbs and document the tradeoff between noise and preserving short hills.
7. Calculate grade from elevation change divided by horizontal distance. Never divide by zero or confuse percent grade with degrees.
8. Cache the processed profile and processing version. Use it for resistance, displayed grade, elevation chart, and climbing calculations consistently; retain raw data for audit/reprocessing.

DEM terrain is not necessarily the road surface. Bridges, tunnels, switchbacks, cliffs, and coarse datasets can produce false gradients. Flag suspicious sections and permit curated corrections. Never claim exact road feel where elevation is estimated or the trainer saturates. Missing elevation must not silently become a flat road: block realistic simulation for that segment until resolved, or offer explicitly labeled flat/manual fallback before starting.

### Grade-to-trainer behavior

Let `s` be route distance in meters and `h(s)` the processed elevation. Compute:

```text
grade_percent = 100 * (h(s + d/2) - h(s - d/2)) / d
requested_grade = grade_percent * trainer_difficulty
command_grade = clamp_and_slew(requested_grade, tested_device_limits)
```

Use one-sided windows at endpoints. Default trainer difficulty to 100% for terrain matching, with an optional clearly explained reduction. Difficulty changes the physical grade command, not the route elevation, recorded climbing, or virtual road physics. Display road grade and applied grade separately when scaling, filtering, or device limits make them differ.

Use FTMS indoor bike simulation parameters where supported. Convert to the exact signed fields, units, and byte order specified by FTMS; for example, a 5% grade must not be encoded as 500% or 0.05%. Add byte-level golden tests. Apply configured wind/rolling-resistance parameters using correct protocol semantics, rather than mapping aerodynamic coefficients by guesswork.

Respect advertised trainer limits and documented/tested limits where capabilities are incomplete. Use smooth transitions and bounded command frequency. A negative grade may reduce resistance but many trainers cannot actively drive the flywheel; represent that honestly. Trainer gearing, flywheel behavior, calibration, and mechanical limits affect how closely the sensation matches a road.

An optional small, configurable look-ahead may compensate measured control latency. It must be bounded and tested so resistance does not noticeably arrive before the displayed hill. Do not build prediction from the AI coach into this loop.

### Virtual motion

Prefer measured rider power and a deterministic bicycle model for route progression. Raw trainer wheel speed alone is not a reliable virtual road speed because gearing and flywheel dynamics affect it. Model rider+bike mass, road slope, gravity, rolling resistance, aerodynamic drag, drivetrain efficiency, acceleration, and coasting.

Use a numerically stable formulation such as an energy update or force solver with explicit low-speed handling. Do not evaluate `force = power / speed` at zero. Make the physical assumptions and units testable. Zero wind and documented default drag/rolling parameters are acceptable initially. Virtual speed must remain nonnegative and bounded by numerical/physical sanity checks.

Advance route distance with virtual speed and a monotonic simulation clock. Interpolate the map position along route geometry by distance; choose the correct segment at crossings using route progress. No GPS is required indoors. Downhill coasting can continue after pedaling stops, while explicit Pause freezes progression. Define auto-pause consistently without erasing valid coasting.

At the route finish, stop advancing, ease trainer load using the tested transition, and offer finish or an explicit new lap. Reverse routes must reverse geometry and recompute grade. For riders without measured power, allow a clearly labeled approximate progression model or manual ride, not falsely precise physics.

### Offline and connectivity

Preflight the ride: route geometry, full elevation profile, selected workout, and any licensed offline map assets must be ready. Route control must never wait for a map tile request. If the background map fails, keep riding with a locally rendered route line and elevation chart. Ride recording, resistance control, and cached workouts remain independent of network state.

## 8. Recommended architecture

Use a modular local application rather than a distributed system for the first release.

| Component | Recommended implementation / responsibility |
|---|---|
| Desktop shell | Tauri 2; native lifecycle, restricted command permissions, packaging |
| UI | React + TypeScript; accessible components and responsive ride dashboard |
| Map | MapLibre GL JS; route/position/elevation presentation |
| Native runtime | Rust/Tokio; BLE adapters, session state machine, control queue, recording |
| Domain engine | Rust modules for workouts, route physics, validation, safety policies |
| Storage | SQLite with migrations; local route/cache assets in app-data directories |
| AI | Provider interface behind native commands; schema-validated proposals |
| Tests | Rust unit/property/integration tests; TypeScript unit/component tests; packaged-app smoke tests |

This is a recommended starting architecture, not a claim that integration is already proven. Run the BLE feasibility milestone before substantial UI investment. If native permissions, BLE indications, or packaging prove unsuitable, propose a documented stack adjustment while retaining the architecture boundaries.

```text
Desktop UI
  | typed, validated IPC commands/events
Native application service
  |-- Session coordinator (sole authority over session state)
  |-- Workout engine / route engine
  |-- Trainer controller + capability checks + command queue
  |      |-- BLE adapter -> trainer / sensors
  |      |-- Simulator adapter -> deterministic test devices
  |-- Telemetry normalization -> recorder -> SQLite
  |-- Route/routing/elevation providers + cache
  |-- Coaching policy + AI provider -> proposed plan only
```

Use monotonic clocks for intervals and control; UTC timestamps plus local timezone for calendar/activity display. Do not accumulate interval drift from UI render timers. The native runtime must not depend on animation frames or whether the ride window has focus.

Proposed repository layout:

```text
apps/desktop/              # UI, assets, desktop configuration
crates/domain/             # Workouts, plans, routes, physics, policies
crates/device/             # Protocol parsing, BLE, capabilities, simulator
crates/session/            # Control ownership, timing, recording
crates/storage/            # SQLite, migrations, import/export
crates/coach/              # Provider interfaces, schemas, plan validation
fixtures/                  # Protocol packets, routes, synthetic activities
docs/                      # Setup, architecture, protocol, hardware matrix
scripts/                   # Reproducible checks and platform build helpers
```

Expose a narrow typed API such as `scanDevices`, `connectDevice`, `assignSource`, `prepareSession`, `startSession`, `pauseSession`, `resumeSession`, `stopSession`, `proposePlan`, and `acceptPlan`. The frontend must not write raw GATT bytes or mutate session rows directly.

## 9. Data model and recording

Use UUIDs, schema versions, explicit units, foreign keys, and migration tests. Suggested entities:

| Entity | Key fields |
|---|---|
| RiderProfile | Goals, schedule, level, units, masses, consent/preferences |
| ThresholdHistory | FTP value, effective date, method, confidence |
| Device / DeviceCapabilities | Local identity, roles, firmware, ranges, supported commands, tested quirks |
| Workout / WorkoutVersion | Name, category, steps, targets, requirements, coaching-policy version |
| TrainingPlan / PlannedSession | Version, dates, workout reference, rationale, acceptance state |
| Route / RouteProfile | Geometry, cumulative distance, elevation, quality, provenance, licenses, processing version |
| Activity | Start/end, timezone, elapsed/moving time, mode, route/workout/FTP snapshots, status |
| Sample | Monotonic offset, timestamp, watts, cadence, HR, virtual speed, distance, grade, source/quality |
| Lap / SessionEvent | Interval boundaries, pause/resume, target changes, disconnects, mode changes |
| RiderFeedback | Effort, fatigue, enjoyment, notes, optional readiness |
| CoachProposal | Input-summary version, provider/model ID, validated proposal, acceptance/rejection |

Preserve incoming telemetry as appropriate, normalize independently for display/control, and record at least one-second time-series samples. Missing is null, not zero. Distinguish measured zero power from missing power; avoid forward-filling stale values into ride totals. Store commanded versus reported trainer state separately.

Persist in small transactions with a recovery journal/checkpoints and no more than a few seconds of potential sample loss on process failure. Use SQLite WAL where appropriate. On next launch, offer recovery of incomplete rides; do not automatically start the trainer. Handle low disk space with visible recording status and a defined stop/save policy.

Summaries include duration, distance, climbing, average/max power/HR/cadence with coverage indicators, time in zones, interval adherence, and effort feedback. Explain metric definitions and handle pause periods consistently. Estimated energy expenditure must be labeled as an estimate. Advanced load metrics are optional and must use documented algorithms and terminology/licensing checks.

Export FIT activity files and CSV records; use the [Garmin FIT SDK documentation](https://developer.garmin.com/fit/overview/) to validate file structure and semantics. FIT export must include correct timestamps, timer events, records, lap/session summaries, units, and an appropriate indoor/virtual cycling classification. Verify exports in an independent decoder. GPX export may be provided for route geometry but does not replace a full sensor-data export.

## 10. Reliability, privacy, and security

- The AI service, map provider, and UI rendering are outside the time-critical control path.
- Prevent system sleep during an active ride when permitted; release the request afterward. Handle unavoidable sleep/wake as a session interruption, with fresh control negotiation.
- Enforce a single active controller/session, including across multiple application instances.
- Restrict Tauri capabilities and content security policy; bundle executable UI code locally. Do not grant remote map content native filesystem or shell privileges.
- Validate imported file sizes and structures, AI outputs, device packets, and native IPC payloads. Reject non-finite and out-of-range numeric values.
- Store credentials in the platform credential store. Redact keys, sensitive profile details, and precise locations from diagnostics.
- Clearly explain that a normal SQLite database is not encrypted by default. Use OS account protection for the initial local store; never claim encryption unless implemented and tested.
- Provide data export, delete-all-data, provider disconnect, cache management, and opt-in diagnostic export. Explain what deletion cannot remove from already submitted provider requests.
- Keep route endpoints and coach conversations private by default. No analytics upload without consent.
- Pin dependencies, retain lockfiles, scan dependencies, include license notices, and verify update signatures before installing updates.
- Never apply an update or restart during a ride. Offer updates afterward and preserve database compatibility/recovery.

## 11. Simulator, testing, and acceptance gates

Provide a deterministic trainer/sensor simulator using the same adapter contract as real devices. Include adjustable power/cadence/HR, control acknowledgment, latency, saturation, disconnect/reconnect, control denied, malformed/truncated packets, stale telemetry, command failure, and multiple devices. Clearly separate simulator testing from hardware proof.

### Required acceptance tests

| ID | Test | Passing evidence |
|---|---|---|
| A01 | Clean installation | Packaged app installs, launches, and opens demo mode on each declared platform/architecture |
| A02 | Multi-device pairing | Compatible trainer + HR + cadence or power sensor report simultaneously in a real ride |
| A03 | Protocol parsing | Golden packet tests cover flags, optional fields, signed values, truncation, counters, rollover, and unit conversion |
| A04 | ERG workout | Real trainer accepts warm-up, steady targets, interval transitions, pause/resume, and finish; targets stay within limits |
| A05 | Road-grade control | Synthetic profile with flat, +5%, and -3% sections yields correct signed simulation commands and displayed grades |
| A06 | Elevation quality | Duplicate points, spikes, missing elevation, bridges/tunnels, and reversed routes never produce uncontrolled targets |
| A07 | Physical free ride | A real compatible trainer changes load uphill/downhill in correspondence with route position; record device behavior and limits |
| A08 | Mode ownership | Switching between ERG and simulation leaves only one controller; stale queued commands cannot take effect |
| A09 | Disconnect under load | No false stop acknowledgment; stale values are visible, data gaps recorded, and reconnect requires controlled resumption |
| A10 | Network/AI outage | Existing workout or prepared free ride finishes and saves normally with network disabled |
| A11 | AI validity | Missing FTP, tight schedules, returning rider, contradictory constraints, injection text, and invalid AI JSON trigger appropriate validation/fallback |
| A12 | Plan adaptation | Feedback creates an explained, versioned proposal; nothing changes until accepted; changes can be undone |
| A13 | Recovery | Forced app termination recovers an incomplete activity with documented maximum data loss and no automatic trainer restart |
| A14 | Export | FIT decodes independently and agrees with internal duration/laps/records; CSV preserves units and missing values |
| A15 | Secrets/privacy | No shared key in shipped assets; no credentials or sensitive location/profile data in ordinary logs |
| A16 | Long session | Two-hour ride/soak has no timer drift, runaway memory growth, stale-command accumulation, or sample duplication |
| A17 | Lifecycle | Sleep/wake, window minimize, close during ride, Bluetooth off/on, and low disk space have tested outcomes |
| A18 | Accessibility | Main riding workflow works by keyboard and readable contrast; status is not communicated by color alone |

Initial performance targets: local input feedback within 100 ms, fresh telemetry displayed within 500 ms of receipt, map at 30 fps on documented reference hardware, and a target change queued within 250 ms of its scheduled transition. Trainer acknowledgment latency is measured separately and depends on hardware. Use simulator timing tests and hardware logs to evaluate these targets; report actual results.

Before claiming broad compatibility, test at least two FTMS trainer models from different manufacturers plus separate HR and cadence/power sensors. At minimum, test one physical trainer on both Windows and macOS before declaring cross-platform trainer support. Publish a model/firmware/OS/transport/mode matrix labeled “tested,” “partial,” or “unverified”; standards support alone is not a compatibility guarantee.

## 12. Windows and macOS builds

Use native CI runners for each operating system, pinned toolchains, repeatable dependency installation, and cached builds. Include development setup scripts/instructions for PowerShell and a macOS shell. The final rider installation must not require Rust, Node.js, Python, or a command line.

For Windows, build an x64 installer and handle the WebView2 runtime in a supported way. Tauri supports MSI or NSIS-based setup packages; see [Windows installer documentation](https://v2.tauri.app/distribute/windows-installer/). Test install, upgrade, uninstall, non-admin behavior where supported, and data retention choices. Sign release binaries with appropriate publisher credentials when available; do not promise a reputation warning will never appear.

For macOS, build Apple Silicon and Intel artifacts, or a tested universal build, with required Bluetooth privacy descriptions and applicable entitlements. Verify permission prompts in the packaged app, not only a development terminal. Use release signing, hardened-runtime configuration as required, notarization, and stapling for direct distribution; see [macOS signing documentation](https://v2.tauri.app/distribute/sign/macos/). Never tell end users to disable platform protections as the normal installation method.

Keep signing secrets out of source control. If signing credentials or a Mac are unavailable, produce the source/build configuration and any locally testable artifacts, then explicitly report the remaining release gate. Do not call an unbuilt macOS package complete.

Ship version information, release notes, checksums, license notices, and a troubleshooting guide. Document exact minimum OS versions and architectures after verification. Platform packaging choices should follow current [Tauri distribution guidance](https://v2.tauri.app/distribute/).

## 13. Implementation milestones

### M0 — Feasibility and decisions

Create the repository and architecture decision log. Confirm OS targets, test hardware, dependencies, map/routing/elevation provider terms, and AI credential mode. Build a small native BLE proof that reads trainer telemetry, obtains control, sets a conservative test ERG target and simulation grade, receives acknowledgments, and stops using a tested procedure on both operating systems. Use a supervised hardware test with the rider able to stop pedaling. Document missing access rather than fabricating results.

Exit: technology choices justified, device capability report produced, minimal packages launch, and any hardware/platform blockers named.

### M1 — Local riding foundation

Build the UI shell, device manager, simulator, normalized telemetry, session state machine, recording, pause/stop, history, and recovery. Deliver one short real ERG workout with working export. Establish CI and protocol fixtures.

Exit: a rider can install, pair, complete a short session, and reopen/export the saved activity; simulator tests cover core faults.

### M2 — Structured training

Add onboarding, FTP history, full workout schema/editor/library, calendar, deterministic planning policy, readiness/feedback, and mode transitions.

Exit: a new or experienced rider can create an appropriate template-based schedule and execute its sessions with validated targets.

### M3 — Map free rides

Implement GPX, provider integration, route builder, elevation processing, profile preview, deterministic cycling physics, simulation commands, route caching, finish/laps, and map-loss fallback.

Exit: a prepared route produces explainable road grades and corresponding real trainer behavior, with no internet dependency during control.

### M4 — AI coaching

Implement one real AI provider adapter, structured proposals, local validation, consent, bounded costs/retries, acceptance/undo, and adaptive suggestions from recorded feedback. Validate live requests only with a local or verified no-charge provider. If live validation would incur charges, test the adapter with fixtures and report live-provider validation as blocked under the no-payment restriction.

Exit: the coach creates and revises valid plans from rider answers and summaries; malformed responses and outages are handled.

### M5 — Complete cross-platform release

Finish accessibility, long-session tests, hardware matrix, installer/upgrade checks, privacy controls, signing/notarization when credentials are supplied, and user/developer documentation.

Exit: acceptance gates pass on declared supported platforms. List any remaining unverified device claims or release-signing dependencies explicitly.

## 14. Deliverables and completion report

Deliver all of the following:

1. Complete source repository with lockfiles, migrations, original assets, and dependency/license notices.
2. Working Windows and macOS packages, with precise architecture/version/test status.
3. A README covering quick start, demo mode, real device setup, map/AI configuration, development, tests, and builds.
4. Architecture and protocol documentation, device capability matrix, configuration reference, and coaching-policy rationale/source records.
5. Original workout library, synthetic/demo routes, packet fixtures, and simulator fault scenarios.
6. Automated test results and a concise manual/hardware validation report tied to build versions.
7. Known limitations, provider costs/quotas, unverified assumptions, and a prioritized backlog.

Provide exact working development and release commands in the resulting repository. Implement a simple documented command interface for install, dev, test, demo, and package tasks, rather than leaving placeholders that look executable but are not.

At the end of each milestone, report: what works; how it was tested; which real hardware/OS combinations were exercised; remaining blockers; and the next milestone. Keep UI screenshots useful, but do not use screenshots as a substitute for runtime or hardware evidence.

The first complete release is done only when the rider can install the app, pair compatible equipment, answer onboarding questions, accept an AI-assisted plan, finish a structured workout, ride a mapped route with elevation-based trainer simulation, and recover/export activities on both supported desktop platforms.

## 15. First action for the implementing AI

Read this document, inspect the repository and available build environment, and start M0. Create a short implementation checklist and architecture decision log, then produce the smallest runnable desktop app with a simulator and native BLE feasibility path. Do not spend the first phase building a polished dashboard around fake connectivity. Keep advancing through the milestones and clearly distinguish implemented, simulated, physically tested, and blocked work.
