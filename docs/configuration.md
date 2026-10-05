# Configuration reference

All settings are edited in the app (**Settings**) and stored in `settings.json` in the data folder ([privacy.md](privacy.md) lists the location). Values outside the ranges below are rejected with a message; nothing is silently clamped.

## General

| Key | Default | Range / values | Meaning |
|---|---|---|---|
| `units` | `metric` | `metric`, `imperial` | Display units. Storage is always SI. |
| `theme` | `dark` | `dark`, `light`, `system` | Appearance |
| `demo_mode` | `false` | bool | Adds simulated devices; demo rides are labelled and never change plan or FTP |
| `diagnostics_opt_in` | `false` | bool | Enables *Export diagnostics* |

## Trainer (`trainer.*`) — locked during a ride

| Key | Default | Range | Meaning |
|---|---|---|---|
| `difficulty_pct` | 100 | 0–100 | Share of the road gradient sent to the trainer in free rides (positive and negative). Ride distance and climbing stay real. Adjustable during a ride. |
| `grade_min_pct` | −10 | −20…0 | Steepest descent sent |
| `grade_max_pct` | 20 | 0…25 | Steepest climb sent |
| `slew_pct_per_s` | 1.5 | 0.2–10 | Maximum change of the commanded grade per second |
| `lookahead_m` | 0 | 0–30 | Sample the grade this far ahead to compensate for trainer lag |
| `stale_ms` | 3000 | 1000–15000 | Telemetry older than this is shown as stale and recorded as missing |
| `target_interval_ms` | 1000 | 500–5000 | Minimum time between control-point targets |
| `ack_timeout_ms` | 3000 | 1000–10000 | Wait for a control-point response before marking the command uncertain |

Fixed safety behaviour (not configurable): 10 s ramp-in at start and after resuming control; ERG low-cadence protection at < 40 rpm for 5 s (recovery: rider acknowledgement and > 60 rpm for 3 s); intensity adjustment −20 % … +10 % during a workout.

## AI coach (`ai.*`)

| Key | Default | Values | Meaning |
|---|---|---|---|
| `provider` | `offline` | `offline`, `local`, `remote` | Offline coach only; a model on this computer; an endpoint you configure |
| `preset` | `ollama` | `ollama`, `lmstudio`, `custom` | Fills endpoint/model defaults |
| `base_url` | `http://localhost:11434/v1` | URL | OpenAI-compatible base URL (`/chat/completions`, `/models`). `local` accepts localhost/loopback only |
| `model` | `llama3.2` | 1–120 chars | Model name as the server knows it |
| `remote_enabled` | `false` | bool | Must be turned on by you before `remote` is used |
| `max_requests_per_day` | 100 | 1–1000 | Cap on AI requests (a plan request with one repair counts as 2) |
| `timeout_s` | 180 | 10–900 | Per-request timeout |
| `max_tokens` | 1800 | 200–8000 | Maximum reply length |

Consent lives in the profile (`ai_consent.enabled`, `share_activity_summaries`, `share_limitations`), all off by default. The remote API key is stored in the OS credential store under service `io.github.halimarefat.ridgeline`, entry `ai_api_key`. On Linux this is the kernel keyring, which does not survive a reboot; re-enter the key after restarting.

Presets: Ollama `http://localhost:11434/v1`; LM Studio `http://localhost:1234/v1`; llama.cpp server and others usually `http://localhost:8080/v1`.

## Ride coach (`ride_coach.*`)

| Key | Default | Range / values | Meaning |
|---|---|---|---|
| `cues` | `true` | bool | Rule-based ride cues (interval previews, cadence, climbs, "ease off?"). Offline. Applies from the next ride |
| `ai_moments` | `true` | bool | With the AI coach on: comment at key moments (hard interval start, halfway, last interval, long climb) |
| `moment_gap_s` | 120 | 60–1800 | Minimum time between AI ride comments; the rider's own prompts are always answered |
| `voice` | `false` | bool | Read cues and replies aloud with the operating system's voice (Web Speech API; offline) |

Ride requests use at most 160 output tokens and a 60 s timeout regardless of `ai.max_tokens` / `ai.timeout_s`, and count one request each towards `max_requests_per_day`.

## Map and services

| Key | Default | Meaning |
|---|---|---|
| `map.enabled` | `true` | Load background map tiles |
| `map.style_url` | `https://tiles.openfreemap.org/styles/liberty` | MapLibre style JSON. A different host also has to be allowed by the app's content security policy (`tauri.conf.json`), so custom tile hosts need a rebuild |
| `map.attribution` | OpenFreeMap © OpenMapTiles Data from OpenStreetMap | Shown on the map |
| `providers.routing_enabled` | `true` | Route builder available |
| `providers.routing_url` | `https://valhalla1.openstreetmap.de` | Valhalla server (`/route`, bicycle costing) |
| `providers.elevation_enabled` | `true` | Fetch elevation for routes without it |
| `providers.elevation_url` | `https://api.open-meteo.com` | Open-Meteo-compatible elevation API (`/v1/elevation`) |

## Route processing (fixed, versioned)

Profile version 1 (`rl_domain::route::ProfileConfig`): resample every 10 m; Hampel spike filter (±5 samples, 3 MAD, ≥ 3 m); 30 m smoothing; grade over a 40 m window; gaps under 200 m of missing elevation are interpolated, longer gaps block simulation until elevation is fetched or the flat fallback is chosen; jumps over 2 km split the route into segments that are never bridged; grades over 25 % and changes over 8 % between samples are flagged. GPX imports are limited to 20 MB, 200,000 points and 500 segments.

## Environment variables

| Variable | Effect |
|---|---|
| `RIDGELINE_SMOKE_TEST=<file>` | CI self-test: uses `<file>.data` as a throwaway data folder, opens demo mode once the UI is running, writes a JSON report to `<file>` and exits (0 = passed). Not for normal use. |

## Developer server flags

`rl-devserver [--port 1420] [--data DIR] [--ui DIR] [--exports DIR]` — serves `--ui` and the command API on 127.0.0.1 only, with simulated devices.
