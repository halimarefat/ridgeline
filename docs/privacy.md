# Privacy

Ridgeline has no account, no server and no analytics. It works fully offline; network services are optional and named below.

## What is stored, and where

All data is stored as files in the app-data folder of your operating-system account (shown in **Settings → Privacy & data**):

- Windows: `%APPDATA%\io.github.halimarefat.ridgeline\`
- macOS: `~/Library/Application Support/io.github.halimarefat.ridgeline/`
- Linux: `~/.local/share/io.github.halimarefat.ridgeline/`

| Data | Contents |
|---|---|
| Profile | Goal, schedule, experience, masses, optional heart-rate values, preferences, limitations, AI consent |
| FTP history | Values, dates, method, confidence |
| Rides | Second-by-second power, cadence, heart rate, speed, virtual distance and position on the route, events, laps, summaries, your feedback |
| Routes | Geometry, elevation and its source, your corrections |
| Plans | Every plan version, proposals, accept/reject/undo history |
| Coach conversation | Your messages and the coach's replies |
| Devices | Bluetooth identifiers and names of devices you connected, assigned roles |
| Settings | Preferences, provider URLs (no secrets) |

**The files are not encrypted.** They are protected by your operating-system account's file permissions, like other documents in your profile. Use full-disk encryption (BitLocker, FileVault, LUKS) if your computer is shared or portable.

**Secrets.** An API key for a remote AI service is stored only in the operating-system credential store (Windows Credential Manager, macOS Keychain, Linux kernel keyring), never in Ridgeline's files, logs or diagnostics. Exports are written to your Downloads folder under `Ridgeline/`.

## What leaves your computer

| When | Sent to | What |
|---|---|---|
| Background map shown | Map tile host (default OpenFreeMap) | Tile requests for the visible area (reveal the region you're viewing) |
| You build a route | Routing service (default Valhalla, FOSSGIS) | The points you placed |
| Elevation fetched for a route | Elevation service (default Open-Meteo) | Sampled route coordinates |
| AI coach enabled **and** consent given | The AI endpoint you configured (default: a model on your own computer) | The compact summary described in [coaching-policy.md](coaching-policy.md#what-is-sent-to-an-ai-model) and your chat message |
| AI coach enabled **and** consent given, during a ride, when you tap a coach prompt or at a key moment (if "AI comments at key moments" is on) | The same AI endpoint | A compact live ride snapshot (see [coaching-policy.md](coaching-policy.md#during-a-ride)) and your prompt or message. No location, route geometry or second-by-second data |

Otherwise nothing is sent for rides, and nothing for history, exports or diagnostics. With the offline coach, the ride coach runs entirely on your computer. Turn off any service in Settings; the app keeps working (routes draw without a background map, and the offline coach takes over). Bluetooth communication stays between your computer and your devices.

## Your controls

- **Export all my data** — copies every file to a dated folder.
- **Export a ride** as FIT, CSV or GPX from History.
- **Diagnostics** — off by default. When enabled, *Export diagnostics* writes a file you can attach to a bug report: app version, platform, policy version, settings with AI details reduced to provider/model, Bluetooth adapter status, connected-device capabilities, firmware strings, parse-error counts and the trainer command log. It contains no profile, rides, routes, locations, coach messages or keys. Nothing is uploaded.
- **Delete everything** — removes all Ridgeline data and any stored API key from this computer. It can't remove copies you exported, or anything already sent to a service you configured (for example, requests already processed by a remote AI provider).
- **Clear** the coach conversation from the Coach screen.

Logs printed by the app (when started from a terminal) contain no keys, profile details or locations.
