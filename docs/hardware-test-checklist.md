# Hardware test checklist

Use this with your own trainer to produce the evidence for acceptance tests A02, A04, A07, A09, A10 and A17. It takes about 45 minutes. Each step says what to do, what should happen, and what to write down.

**Safety first.** Ride on a stable trainer setup, keep the first targets low, and be ready to stop pedalling: with no pedalling, any smart trainer's load drops quickly. Stop immediately if anything feels wrong. Close other apps that connect to the trainer (Zwift, manufacturer apps, bike computers).

Before you start: note your **Ridgeline version** (Settings → About), **OS and version**, **trainer model and firmware** (Devices screen after connecting), and sensors. Turn on **Settings → Privacy & data → Allow diagnostic export**.

## 1. Pairing (A02)

1. Wake the trainer, open **Devices**, press **Scan**.
   - Expect: the trainer appears with "controllable trainer". Write down: did it appear? How long did it take?
2. Connect the trainer and press **Use as trainer**.
   - Expect: state goes to *ready*; capabilities show ERG / road simulation / resistance and a power range.
   - Write down the capabilities line exactly.
3. Connect a heart-rate strap and (if you have one) a power meter or cadence sensor.
   - Expect: all show *ready*, with battery levels where reported.
4. Pedal. In **Data sources**, check power, cadence and heart rate are live, and compare the candidate values.
   - Write down: does trainer power roughly match your power meter? Does cadence look right?

## 2. ERG workout (A04)

1. **Workouts → Easy Spin 30** (or another easy workout), then **Ride this workout**. Check the preflight is green, then **Start ride**.
   - Expect: "trainer controlled"; the load ramps in over about 10 s and holds the target within a few watts once your cadence is steady.
2. Change cadence between 75 and 95 rpm.
   - Expect: power stays near the target.
3. Press **Space** to pause, wait 10 s, press **Space** again.
   - Expect: the load eases while paused and ramps back in on resume.
4. Press **+** twice and **−** twice.
   - Expect: the target changes by 5 % each step.
5. Press **N** to skip to the next interval.
   - Expect: the target changes within about a second.
6. Drop below 40 rpm for more than 5 s.
   - Expect: a low-cadence warning appears and the load drops.
7. Acknowledge the warning and pedal above 60 rpm.
   - Expect: the target ramps back in.
8. Press **S** → **Stop and save**.
   - Expect: "The trainer confirmed the stop." and the load releases.
   - Write down: any target that wasn't followed, acknowledgement latencies (Devices → Trainer control log), whether the stop was confirmed.

## 3. Free ride with road simulation (A07)

1. **Routes → Test: Grade steps (A05, synthetic) → Free ride this route**, difficulty 100 %.
   - Expect: flat, then a +5 % section, then −3 %. The Ride screen shows the road grade and the commanded grade.
2. Ride at a steady effort through each section.
   - Write down: does the load clearly increase on +5 % and decrease on −3 %? About how long after the grade changes do you feel it?
3. Set difficulty to 50 % on a climb.
   - Expect: the commanded grade halves and the effort eases.
4. Optional: import a GPX of a road you know (or build one), fetch elevation, and ride a few kilometres.
   - Write down: impressions, and any grade the trainer couldn't hold.

## 4. Disconnect under load (A09)

1. During an ERG workout at moderate load, switch the trainer off (or unplug it) for 10–20 s, then switch it on.
   - Expect: the ride pauses and the **Trainer control was interrupted** banner appears. Power shows as stale, not frozen at a value. When the trainer is back, nothing resumes until you press **Resume control**; the load then ramps in.
   - Write down: anything that resumed by itself, and how long reconnection took.
2. Turn the heart-rate strap off for 10 s.
   - Expect: HR shows stale; the gap appears in the History chart.

## 5. Network and AI outage (A10)

1. Turn off Wi-Fi or unplug the network, start the Grade steps route or a workout, ride 2 minutes, and stop and save.
   - Expect: the ride completes and saves normally. The map shows the local route line.

## 6. Lifecycle (A17)

1. During a ride, try to close the window.
   - Expect: Ridgeline asks you to stop and save first.
2. Minimize the window for 30 s mid-ride.
   - Expect: the ride and control continue (check the timer).
3. Let the computer sit idle during a ride for longer than its sleep timeout.
   - Expect: it doesn't sleep while a ride is active.
4. Turn Bluetooth off for 10 s mid-ride, then back on.
   - Expect: the same behaviour as step 4.
5. Force-quit Ridgeline mid-ride (Task Manager / Force Quit), then reopen it.
   - Expect: it offers to recover the interrupted ride and doesn't start the trainer by itself. Recovered data should end within about 2 s of the quit.

## 7. Export (A14, spot check)

1. In **History**, export the ERG ride as FIT and upload it to a tool you use (Garmin Connect, Strava, intervals.icu, or the [FIT File Viewer](https://www.fitfileviewer.com/)).
   - Write down: does it import? Do duration, laps, power and HR look right?

## Report

Export diagnostics (Settings → Privacy & data). Then open an issue with:
- the version, OS, trainer and sensor details;
- your notes for each step;
- the diagnostics file.

Results go into [hardware-matrix.md](hardware-matrix.md).
