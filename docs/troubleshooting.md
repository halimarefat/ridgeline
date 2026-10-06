# Troubleshooting

**The trainer doesn't appear when scanning.** Pedal to wake it. Close every other app that might be connected to it (Zwift, the manufacturer's app, a bike computer, a phone): many trainers accept only one Bluetooth connection. Make sure Bluetooth is on. On macOS, check **System Settings → Privacy & Security → Bluetooth** allows Ridgeline. On Linux, check `systemctl status bluetooth`. Try **Show all Bluetooth devices**: some trainers advertise their services only after connection.

**"Control not permitted" / control denied.** Another app or device holds control of the trainer. Close it, or power-cycle the trainer, then press **Resume control** (or start the ride again).

**ERG target isn't followed.** Ride at a steady cadence (75–95 rpm). Check the trainer's power range on the Devices screen: targets outside it are clamped. Some trainers need a few seconds to settle after each change. If the Devices → Trainer control log shows *timeout* or *failed*, export diagnostics and open an issue.

**Resistance feels stuck after a disconnect.** Stop pedalling. The load on any smart trainer falls quickly without pedalling. Ridgeline never re-applies a target after a disconnect until you press **Resume control**, but the trainer itself may keep its last setting until it is power-cycled.

**Power or heart rate shows "stale".** No fresh data has arrived for 3 s. Check the sensor's battery and distance from the computer. USB 3 ports and Wi-Fi on 2.4 GHz can interfere with Bluetooth; a short USB extension for a Bluetooth dongle can help.

**The map is blank.** The route line is still drawn locally. Tiles need the internet; check your connection or turn the map off in **Settings → Map & services**.

**"This route needs elevation".** The GPX file had no (or too little) elevation. Use **Fetch elevation** (needs internet), or **Use flat fallback** to ride it flat.

**The AI coach says the offline plan was used.** The model was unreachable, too slow, or returned something that failed validation twice; the reason is shown on the proposal. For Ollama: make sure it's running (`ollama list`), the model is pulled (`ollama pull llama3.2`), and **Settings → AI coach → Test connection** succeeds. Small models sometimes struggle with the plan format; try a larger one (for example `qwen2.5:7b`), or raise the timeout.

**Windows: "Windows protected your PC".** The preview is not code-signed. Choose **More info → Run anyway** only if you downloaded the installer from this repository's Releases page and its checksum matches.

**macOS: "Ridgeline can't be opened".** The preview is not notarized. Open **System Settings → Privacy & Security** and choose **Open Anyway** for Ridgeline. This allows only this app.

**"This data was written by a newer Ridgeline".** You opened your data with an older version after using a newer one. Install the newer version again; your data was not changed.

**Ridgeline is already running.** Only one instance can run at a time, so two copies never control the trainer together. Close the other window. If none is open, a crashed instance may still be exiting; wait a few seconds.

**A ride was interrupted.** On the next start Ridgeline offers to recover it. Up to about 2 seconds of data before the interruption can be lost. The trainer is not restarted automatically.

**Where are exported files?** In your Downloads folder under `Ridgeline/`. The full path appears in the confirmation message.

**Reporting a problem.** Turn on **Settings → Privacy & data → Allow diagnostic export**, then **Export diagnostics**, and attach the file to an issue. Describe what you did, what happened and what you expected. The file contains no profile, rides, routes or keys.
