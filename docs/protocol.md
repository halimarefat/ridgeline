# Bluetooth protocol notes

Ridgeline speaks standard Bluetooth SIG GATT profiles only. No proprietary trainer protocols are implemented. Parsers live in `crates/device/src` (`ftms.rs`, `sensors.rs`, `bytes.rs`), are pure functions over byte slices, and are covered by golden-packet tests (A03): flags, optional fields, signed values, truncated packets, counter rollover and unit conversion. Malformed packets are counted per device (`parse_errors`) and dropped; they never become readings.

## Services and characteristics

| Service | UUID | Characteristics used |
|---|---|---|
| Fitness Machine (FTMS) | 0x1826 | Feature 0x2ACC (read), Indoor Bike Data 0x2AD2 (notify), Training Status 0x2AD3, Supported Resistance Level Range 0x2AD6 (read), Supported Power Range 0x2AD8 (read), Control Point 0x2AD9 (write + indicate), Machine Status 0x2ADA (notify) |
| Cycling Power | 0x1818 | Measurement 0x2A63 (notify), Feature 0x2A65 (read) |
| Heart Rate | 0x180D | Measurement 0x2A37 (notify) |
| Cycling Speed and Cadence | 0x1816 | Measurement 0x2A5B (notify) |
| Battery | 0x180F | Level 0x2A19 (read/notify) |
| Device Information | 0x180A | Manufacturer 0x2A29, Model 0x2A24, Firmware 0x2A26 |

A device can play several roles (a smart trainer usually exposes FTMS and Cycling Power). Scans show devices advertising any of the four sensor services; **Show all Bluetooth devices** lists everything nearby.

## Parsing

- **Indoor Bike Data.** Flag bit 0 ("More Data") is *inverted*: when 0, instantaneous speed (uint16, 0.01 km/h) is present. Then, per flag: average speed, instantaneous cadence (uint16, 0.5 rpm), average cadence, total distance (uint24, m), resistance level (sint16), instantaneous power (sint16, W), average power, expended energy (total uint16 kcal, per hour uint16, per minute uint8), heart rate (uint8), MET (uint8, 0.1), elapsed time (uint16 s), remaining time. Absent fields are `None`, never 0.
- **Cycling Power Measurement.** Flags (uint16), instantaneous power (sint16 W), then optional pedal power balance, accumulated torque, wheel revolution data (uint32 count + uint16 time, 1/2048 s), crank revolution data (uint16 count + uint16 time, 1/1024 s) and further optional fields, skipped by length. Cadence is derived from crank revolutions.
- **CSC Measurement.** Wheel revolutions (uint32 + uint16 time, 1/1024 s) and crank revolutions (uint16 + uint16 time, 1/1024 s).
- **Revolution rates** (`RevolutionRate`) handle counter and event-time rollover, ignore repeated events, reject implausible jumps (above a per-sensor maximum rpm), and fall to 0 rpm only after a no-new-event timeout (coasting), rather than on the first repeated packet.
- **Heart Rate Measurement.** uint8 or uint16 value by flag bit 0; sensor-contact, energy and RR-interval fields are skipped safely.

## Trainer capabilities

From Fitness Machine Feature (two uint32 bit fields): power measurement and cadence support; target settings for power (ERG), resistance, indoor bike simulation, spin-down and wheel circumference. Power and resistance ranges are read when available and used to clamp targets. The UI shows exactly what the trainer reported; a mode the trainer doesn't support is not offered.

## Control point

Writes are serialized: one command in flight, matched to its response indication (`0x80, request op code, result code`).

| Op code | Command | Parameters as encoded |
|---|---|---|
| 0x00 | Request Control | — (sent only after the indication subscription succeeds) |
| 0x01 | Reset | — |
| 0x05 | Set Target Power | sint16 W |
| 0x04 | Set Target Resistance Level | sint16, 0.1 resolution (as deployed by common trainers; confirm per device) |
| 0x11 | Set Indoor Bike Simulation Parameters | wind sint16 0.001 m/s, grade sint16 0.01 %, Crr uint8 0.0001, Cw uint8 0.01 kg/m |
| 0x07 | Start or Resume | — |
| 0x08 | Stop or Pause | uint8: 0x01 stop, 0x02 pause |

Result codes: 0x01 success, 0x02 op code not supported, 0x03 invalid parameter, 0x04 operation failed, 0x05 control not permitted. Every encode rejects non-finite and out-of-range values.

Simulation commands send wind 0, the route grade after difficulty scaling, clamping and slew limiting, Crr 0.004 and Cw = ½·ρ·CdA ≈ 0.20 kg/m (the same constants as the in-app physics model), so the trainer's own physics and Ridgeline's virtual speed agree.

### Rules (`crates/device/src/controller.rs`)

- Control is requested when a ride starts, not on connect.
- Acknowledgement timeout 3 s (configurable 1–10 s). A timed-out request leaves control **uncertain**; start/stop are never retried blindly.
- Targets: newest wins (an older pending target is dropped), minimum interval 1 s (configurable 0.5–5 s), each tagged with the session generation and discarded if the generation changed before sending.
- Stop: pending targets dropped → low-load target → Stop (0x01). The stop counts as confirmed only on a success response.
- Machine Status "control permission lost" or a disconnect moves control to **lost**; resuming requires the rider's action and re-requests control. Targets then ramp in over 10 s.
- The controller's command log (command, outcome, latency) is shown on the Devices screen and included in the opt-in diagnostics export.

## Connection state machine

`disconnected → connecting → discovering → subscribing → ready`, with `reconnecting` (bounded backoff while the rider still wants the device connected) and `faulted` (with the reason shown). Telemetry freshness is tracked separately per metric (see [architecture.md](architecture.md#telemetry)).

## Simulator

`crates/device/src/simulator.rs` implements the same adapter contract as the BLE adapter: it advertises a smart trainer (FTMS + Cycling Power), power pedals, a heart-rate strap and a cadence sensor, encodes real packets, decodes control-point writes and answers with indications. Rider power, cadence and heart rate can be adjusted. Fault injection (Devices → Simulator): out of range/off (disconnect), stops sending data (stale), malformed packets, deny control, drop acknowledgements, slow acknowledgements, write failures, held by another app, no simulation mode, a 300 W power limit (saturation), a 6 % grade limit, and low battery. Simulator results are evidence about Ridgeline's logic, not about any real device.
