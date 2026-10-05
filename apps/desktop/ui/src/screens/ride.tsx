// Ride: launch with preflight, the cockpit (large numbers, Pause/Stop always
// visible, keyboard shortcuts) and the post-ride summary with feedback.
import { useEffect, useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { typingInField, useApp, useRpc } from "../state";
import { Button, ErrorText, Field, Modal, NumberInput, Scale, Spinner, Stat, Status, Toggle, useKeys } from "../ui";
import { ElevationChart, SeriesChart, WorkoutChart } from "../charts";
import { MapView } from "../map";
import { clock, dist, elev, minutes, pct, round, speed, type Units } from "../format";
import { useAction, useDebounced } from "../hooks";
import { IntensityTag, ProposalModal } from "../plan";

type LaunchMode = "erg" | "erg_map" | "free_ride" | "manual" | "read_only";

export function Ride() {
  const { live } = useApp();
  const s: J = live?.session;
  if (s && s.state === "finished") return <Finished s={s} activityId={live.activity_id} />;
  if (s && s.state !== "discarded" && s.state !== "prepared") return <Cockpit s={s} />;
  return <Launch />;
}

// ------------------------------------------------------------ launch

function Launch() {
  const { screen, live, units, go, toast } = useApp();
  const params = (screen.params ?? {}) as Record<string, string | undefined>;
  const initialMode: LaunchMode = params.mode === "free_ride" ? "free_ride" : params.mode === "manual" ? "manual" : params.mode === "read_only" ? "read_only" : params.route_id && params.workout_id ? "erg_map" : params.route_id ? "free_ride" : "erg";
  const [mode, setMode] = useState<LaunchMode>(initialMode);
  const [workoutId, setWorkoutId] = useState<string>(params.workout_id ?? "");
  const [routeId, setRouteId] = useState<string>(params.route_id ?? "");
  const [rpeMode, setRpeMode] = useState(false);
  const [difficulty, setDifficulty] = useState<number | null>(null);
  const [level, setLevel] = useState(20);
  const planSessionId = params.plan_session_id;
  const workouts = useRpc("listWorkouts", { include_tests: true });
  const routes = useRpc("listRoutes", {});
  const prof = useRpc("getProfile", {});
  const settings = useRpc("getSettings", {});
  const needsWorkout = mode === "erg" || mode === "erg_map";
  const needsRoute = mode === "erg_map" || mode === "free_ride";
  const spec = useMemo(() => {
    const p: Record<string, unknown> = { mode: mode === "erg_map" ? "erg" : mode, rpe_mode: rpeMode, manual_level: level };
    if (needsWorkout && workoutId) p.workout_id = workoutId;
    if (needsRoute && routeId) p.route_id = routeId;
    if (difficulty != null) p.difficulty_pct = difficulty;
    if (planSessionId) p.plan_session_id = planSessionId;
    return p;
  }, [mode, workoutId, routeId, rpeMode, difficulty, level, needsWorkout, needsRoute, planSessionId]);
  const dspec = useDebounced(spec, 250);
  const [pre, setPre] = useState<J | null>(null);
  const [preErr, setPreErr] = useState<string | null>(null);
  const devKey = (live?.devices?.devices ?? []).map((d: J) => `${d.key}:${d.state}`).join(",") + (live?.live?.power?.freshness ?? "");
  useEffect(() => {
    let alive = true;
    rpc("preflight", dspec)
      .then((r) => alive && (setPre(r), setPreErr(null)))
      .catch((e) => alive && (setPre(null), setPreErr(e.message)));
    return () => {
      alive = false;
    };
  }, [dspec, devKey]);
  const { run, busy } = useAction();
  const w = (workouts.data?.workouts ?? []).find((x: J) => x.id === workoutId);
  const r = (routes.data ?? []).find((x: J) => x.id === routeId);
  const ftp = prof.data?.ftp?.watts;
  const start = () =>
    run("start", async () => {
      await rpc("startSession", spec);
      toast(mode === "read_only" ? "Recording started." : "Requesting trainer control…");
    });
  const routeDetail = useRpc(routeId ? "getRoute" : "", { id: routeId, max_points: 1500 }, [routeId]);
  const defDifficulty = settings.data?.settings?.trainer?.difficulty_pct ?? 100;

  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Start a ride</h1>
          <p className="sub">Choose what to ride. Ridgeline checks your equipment before you start.</p>
        </div>
      </div>
      <div className="grid-2">
        <section className="panel">
          <h2>Ride type</h2>
          <div className="choice-grid" role="radiogroup" aria-label="Ride type">
            {(
              [
                ["erg", "Structured workout", "The trainer holds each interval's target power (ERG)."],
                ["erg_map", "Workout on a map", "ERG workout; the map advances, terrain doesn't change resistance."],
                ["free_ride", "Free ride a route", "The trainer follows the road's gradient."],
                ["manual", "Manual resistance", "You set the resistance level."],
                ["read_only", "Record only", "No trainer control — just record your sensors."],
              ] as [LaunchMode, string, string][]
            ).map(([m, t, d]) => (
              <button key={m} className="choice" role="radio" aria-checked={mode === m} onClick={() => setMode(m)}>
                <b>{t}</b>
                <span>{d}</span>
              </button>
            ))}
          </div>
          <div className="cols" style={{ marginTop: 16 }}>
            {needsWorkout && (
              <Field label="Workout">
                <select value={workoutId} onChange={(e) => setWorkoutId(e.target.value)}>
                  <option value="">Choose a workout…</option>
                  {(workouts.data?.workouts ?? []).map((x: J) => (
                    <option key={x.id} value={x.id}>
                      {x.name} — {minutes(x.duration_s)}
                      {x.category === "test_fixture" ? " (hardware test)" : ""}
                    </option>
                  ))}
                </select>
              </Field>
            )}
            {needsRoute && (
              <Field label="Route">
                <select value={routeId} onChange={(e) => setRouteId(e.target.value)}>
                  <option value="">Choose a route…</option>
                  {(routes.data ?? []).map((x: J) => (
                    <option key={x.id} value={x.id}>
                      {x.name} — {dist(x.summary?.total_m, units)}
                      {x.summary && !x.summary.simulation_ready ? " (needs elevation)" : ""}
                    </option>
                  ))}
                </select>
              </Field>
            )}
            {mode === "free_ride" && (
              <Field label="Trainer difficulty" hint="Scales the trainer's resistance on climbs only; distance and climbing stay real.">
                <NumberInput value={difficulty ?? defDifficulty} min={0} max={100} step={5} suffix="%" onChange={(v) => setDifficulty(v == null ? null : Math.max(0, Math.min(100, v)))} />
              </Field>
            )}
            {mode === "manual" && (
              <Field label="Starting resistance level">
                <NumberInput value={level} min={0} max={100} suffix="%" onChange={(v) => setLevel(Math.max(0, Math.min(100, v ?? 0)))} />
              </Field>
            )}
          </div>
          {needsWorkout && (
            <div style={{ marginTop: 12 }}>
              <Toggle
                checked={rpeMode}
                onChange={setRpeMode}
                label="Ride by perceived effort (no ERG targets)"
                hint={ftp ? `Your FTP is ${ftp} W. Use this if your power reading isn't reliable.` : "No FTP is set: workouts with % FTP targets need this, or an FTP in Settings."}
              />
            </div>
          )}
          {planSessionId && <p className="small muted">This ride counts towards today's planned session.</p>}
        </section>

        <section className="panel">
          <h2>Preflight</h2>
          <ErrorText>{preErr}</ErrorText>
          {!pre && !preErr && <Spinner label="Checking…" />}
          {pre && (
            <ul className="preflight-list">
              {pre.items.map((i: J) => (
                <li key={i.key}>
                  <Status kind={i.status === "ok" ? "ok" : i.status === "warn" ? "warn" : "bad"}>{i.key.replace("_", " ")}</Status>
                  <span>{i.message}</span>
                </li>
              ))}
            </ul>
          )}
          <div className="actions" style={{ marginTop: 16 }}>
            <Button kind="primary" big disabled={!pre?.ok || !!busy} onClick={start}>
              {busy ? "Starting…" : "Start ride"}
            </Button>
            {pre && !pre.ok && <Button onClick={() => go("devices")}>Devices</Button>}
          </div>
        </section>
      </div>

      {needsWorkout && w && (
        <section className="panel">
          <div className="panel-head">
            <h3>{w.name}</h3>
            <div className="actions">
              <IntensityTag v={w.intensity} />
              <span className="tag">{minutes(w.duration_s)}</span>
            </div>
          </div>
          <WorkoutChart timeline={w.profile} height={120} />
          <p className="muted">{w.description}</p>
        </section>
      )}
      {needsRoute && r && routeDetail.data && (
        <section className="panel">
          <div className="panel-head">
            <h3>{r.name}</h3>
            <span className="muted">
              {dist(r.summary?.total_m, units)} · {elev(r.summary?.ascent_m, units)} climbing
            </span>
          </div>
          <ElevationChart chart={routeDetail.data.display.chart} units={units} height={120} flags={routeDetail.data.summary.flags} />
        </section>
      )}
    </div>
  );
}

// ------------------------------------------------------------ cockpit

function readingText(r: J, digits = 0): { value: string; note: string; stale: boolean } {
  if (!r || r.freshness === "no_source") return { value: "—", note: "no sensor", stale: true };
  if (r.freshness === "fresh") return { value: r.value == null ? "—" : digits ? Number(r.value).toFixed(digits) : String(Math.round(r.value)), note: "", stale: false };
  if (r.freshness === "disconnected") return { value: "—", note: "disconnected", stale: true };
  return { value: "—", note: `no data for ${Math.round((r.age_ms ?? 0) / 1000)} s`, stale: true };
}

function Gauge(props: { label: string; r: J; unit: string; sub?: React.ReactNode; main?: boolean }) {
  const t = readingText(props.r);
  return (
    <div className={`gauge ${props.main ? "gauge-main" : ""} ${t.stale ? "gauge-stale" : ""}`} aria-live="off">
      <span className="gauge-label">
        <span>{props.label}</span>
        {t.stale && <span className="warn-text">▲ {t.note}</span>}
      </span>
      <span className="gauge-value">
        {t.value}
        <span className="gauge-unit">{props.unit}</span>
      </span>
      <span className="gauge-sub">{props.sub}</span>
    </div>
  );
}

function Cockpit({ s }: { s: J }) {
  const { units, go } = useApp();
  const { run, busy } = useAction();
  const [confirmStop, setConfirmStop] = useState(false);
  const [switchTo, setSwitchTo] = useState<string | null>(null);
  const routeData = useRpc(s.route ? "getRoute" : "", { id: s.route?.id, max_points: 3000 }, [s.route?.id]);
  const settings = useRpc("getSettings", {});
  const running = s.state === "running";
  const paused = s.state === "paused";
  const cmd = (method: string, params: Record<string, unknown> = {}) => run(method, () => rpc(method, params));
  const wk: J = s.workout;
  const step: J = wk?.step;
  const rt: J = s.route;

  useKeys(
    (e) => {
      if (typingInField() || e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.code === "Space") {
        e.preventDefault();
        if (running) cmd("pauseSession");
        else if (paused) cmd(s.needs_resume_control ? "resumeControl" : "resumeSession");
      } else if (e.key === "s" || e.key === "S") {
        setConfirmStop(true);
      } else if ((e.key === "n" || e.key === "N") && wk) {
        cmd("skipInterval");
      } else if ((e.key === "+" || e.key === "=") && wk) {
        cmd("adjustIntensity", { delta: 5 });
      } else if (e.key === "-" && wk) {
        cmd("adjustIntensity", { delta: -5 });
      } else if (e.key === "l" || e.key === "L") {
        cmd("lap");
      }
    },
    [running, paused, wk, s.needs_resume_control],
  );

  const modeLabel: Record<string, string> = { erg: rt ? "ERG workout · map" : "ERG workout", free_ride: "Road simulation", manual: "Manual resistance", read_only: "Read-only (no trainer control)" };
  const title = wk?.name ?? rt?.name ?? modeLabel[s.mode];
  const ctlState: string = s.control?.state;
  const freePower = (s.power_alternatives ?? []).filter((a: J) => a.freshness === "fresh" && a.value != null);
  const switchable = ["erg", "free_ride", "manual", "read_only"].filter((m) => m !== s.mode && (m !== "erg" || wk) && (m !== "free_ride" || rt));

  return (
    <div className="ride">
      <div className="ride-bar" role="toolbar" aria-label="Ride controls">
        <span className="title">
          {s.demo && <span className="demo-flag">DEMO</span>} {title}
        </span>
        <span className="mode">{modeLabel[s.mode]}</span>
        <Status kind={s.state === "running" ? "ok" : s.state === "paused" ? "warn" : "busy"}>{s.state === "starting" ? "requesting control" : s.state}</Status>
        {s.mode !== "read_only" && <Status kind={ctlState === "controlled" ? "ok" : ctlState === "requesting" ? "busy" : "bad"}>trainer {ctlState?.replace("_", " ")}</Status>}
        <Status kind={s.recording_error ? "bad" : "ok"}>{s.recording_error ? "recording problem" : `recording · ${s.samples} s`}</Status>
        {running && (
          <Button kind="primary" big onClick={() => cmd("pauseSession")} kbd="Space">
            ❚❚ Pause
          </Button>
        )}
        {paused && (
          <Button kind="primary" big onClick={() => cmd(s.needs_resume_control ? "resumeControl" : "resumeSession")} kbd="Space">
            ▶ Resume
          </Button>
        )}
        {s.state === "starting" && (
          <Button big disabled>
            <Spinner label="Waiting for trainer" />
          </Button>
        )}
        <Button kind="danger" big onClick={() => setConfirmStop(true)} disabled={s.state === "stopping"} kbd="S">
          ■ Stop
        </Button>
      </div>

      {s.state === "starting" && (
        <div className="banner banner-info">
          <p>Asking the trainer for control. If this takes long, another app may be connected to the trainer.</p>
        </div>
      )}
      {s.state === "stopping" && (
        <div className="banner banner-info">
          <p>
            <Spinner label="Stopping: easing the trainer and waiting for it to confirm…" />
          </p>
        </div>
      )}
      {s.needs_resume_control && (
        <div className="banner banner-bad" role="alert">
          <p>
            <b>Trainer control was interrupted.</b> The ride is paused. The trainer may still hold its last resistance — stop pedalling if it feels wrong. When the trainer is connected
            again, resume: control is renegotiated and the load ramps in gently.
          </p>
          <Button kind="primary" onClick={() => cmd("resumeControl")} disabled={!!busy}>
            Resume control
          </Button>
          <Button onClick={() => go("devices")}>Devices</Button>
        </div>
      )}
      {s.low_cadence_active && (
        <div className="banner banner-warn" role="alert">
          <p>
            <b>Low cadence:</b> the target has been eased so you can get going again. Spin up above 60 rpm and it ramps back in, or resume the target now.
          </p>
          {!s.low_cadence_ack && <Button onClick={() => cmd("ackLowCadence")}>Resume target</Button>}
        </div>
      )}
      {s.recording_error && (
        <div className="banner banner-bad" role="alert">
          <p>
            <b>Recording problem:</b> {s.recording_error}. Data is kept in memory and retried. Free up disk space if possible.
          </p>
        </div>
      )}
      {rt?.finished && (
        <div className="banner banner-info">
          <p>
            <b>End of the route.</b> The trainer is easing to a flat road. Finish the ride or ride another lap.
          </p>
          <Button kind="primary" onClick={() => cmd("newRouteLap")}>
            New lap
          </Button>
          <Button onClick={() => setConfirmStop(true)}>Finish ride</Button>
        </div>
      )}
      {step?.free_effort && (
        <div className="banner banner-info">
          <p>
            <b>Ride this by feel:</b> effort {step.rpe}/10 — {step.rpe_words}. {wk?.rpe_mode ? "Perceived-effort mode: no ERG targets." : "The trainer is set to a flat road so you control the effort."}
          </p>
        </div>
      )}

      <div className="cockpit">
        <Gauge
          label="Power"
          r={s.power}
          unit="W"
          main
          sub={
            s.mode === "erg" && s.target_w != null && !step?.free_effort ? (
              <>
                Target <b>{s.target_w} W</b>
                {wk?.adjust_pct ? ` (${pct(wk.adjust_pct, 0)})` : ""}
              </>
            ) : freePower.length > 1 ? (
              freePower.map((a: J) => `${a.source.device.replace(/^(ble|simulator):/, "")} ${round(a.value)}`).join(" · ")
            ) : (
              ""
            )
          }
        />
        <Gauge label="Cadence" r={s.cadence} unit="rpm" sub={step?.cadence ? `Cue ${step.cadence[0]}–${step.cadence[1]} rpm` : ""} />
        <Gauge label="Heart rate" r={s.heart_rate} unit="bpm" />
        <div className="gauge">
          <span className="gauge-label">Ride time</span>
          <span className="gauge-value">{clock(s.active_s)}</span>
          <span className="gauge-sub">
            {dist(s.distance_m, units, 2)}
            {wk && ` · ${clock(wk.remaining_s)} left`}
          </span>
        </div>
      </div>

      {wk && (
        <section className="panel">
          {step && (
            <div className="stepline">
              <span className="step-name">{step.label}</span>
              <span className="step-left">{clock(step.remaining_s)}</span>
              {!step.free_effort && step.target_w != null && (
                <span>
                  <b>{Math.round(step.target_w)} W</b> · {Math.round(step.target_pct)}% FTP
                </span>
              )}
              {!step.free_effort && step.target_w == null && step.rpe != null && (
                <span>
                  Effort <b>{step.rpe}/10</b> — {step.rpe_words}
                </span>
              )}
              {step.text && <span className="cue">{step.text}</span>}
            </div>
          )}
          {wk.next && (
            <p className="small muted">
              Next: {wk.next.label} · {clock(wk.next.dur_s)}
            </p>
          )}
          <WorkoutChart timeline={wk.timeline} pos={wk.pos_s} height={110} />
          <div className="ride-tools" style={{ marginTop: 10 }}>
            <span className="adjust">
              <Button onClick={() => cmd("adjustIntensity", { delta: -5 })} kbd="−" title="Easier by 5%">
                Easier
              </Button>
              <b>{pct(wk.adjust_pct, 0) === "0%" ? "±0%" : pct(wk.adjust_pct, 0)}</b>
              <Button onClick={() => cmd("adjustIntensity", { delta: 5 })} kbd="+" title="Harder by 5%">
                Harder
              </Button>
            </span>
            <span className="small muted">Intensity can change from −20% to +10%. Changes are recorded separately from the workout.</span>
            <span className="grow" />
            <Button onClick={() => cmd("skipInterval")} kbd="N">
              Skip interval
            </Button>
            <Button kind="quiet" onClick={() => cmd("lap")} kbd="L">
              Lap
            </Button>
          </div>
        </section>
      )}

      {rt && (
        <section className="panel">
          <div className="grade-readout">
            <Stat label="Road grade" value={rt.road_grade == null ? "—" : pct(rt.road_grade)} note={rt.flat_fallback_active ? "flat fallback (no elevation here)" : undefined} />
            {rt.controls_resistance && (
              <Stat
                label="Trainer grade"
                value={rt.commanded_grade == null ? "—" : pct(rt.commanded_grade)}
                note={rt.saturated ? "limited by trainer settings" : rt.difficulty_pct < 100 ? `difficulty ${Math.round(rt.difficulty_pct)}%` : "matches the road"}
              />
            )}
            {!rt.controls_resistance && <Stat label="Terrain" value="map only" note="the workout sets resistance" />}
            <Stat label="Distance" value={`${dist(rt.s, units, 2)}`} note={`of ${dist(rt.total_m, units)}${rt.lap > 1 ? ` · lap ${rt.lap}` : ""}`} />
            <Stat label="Speed" value={speed(rt.speed, units)} note={rt.progression === "power" ? "from your power" : rt.progression === "coasting" ? "coasting (no power)" : "approximate (trainer speed)"} />
            <Stat label="Climbed" value={elev(rt.ascent_m, units)} />
            {rt.controls_resistance && (
              <Field label={`Trainer difficulty ${Math.round(rt.difficulty_pct)}%`}>
                <input type="range" min={0} max={100} step={5} value={rt.difficulty_pct} onChange={(e) => cmd("setDifficulty", { difficulty_pct: Number(e.target.value) })} />
              </Field>
            )}
          </div>
          {routeData.data && (
            <div className="grid-2" style={{ marginTop: 12 }}>
              <MapView
                line={routeData.data.display.line}
                pos={[rt.lon, rt.lat]}
                follow
                styleUrl={settings.data?.settings?.map?.style_url ?? ""}
                enabled={!!settings.data?.settings?.map?.enabled && routeData.data.elevation?.kind !== "synthetic"}
                synthetic={routeData.data.elevation?.kind === "synthetic"}
                height={300}
              />
              <ElevationChart chart={routeData.data.display.chart} pos={rt.s} units={units} height={300} flags={routeData.data.summary.flags} />
            </div>
          )}
        </section>
      )}

      {s.mode === "manual" && (
        <section className="panel">
          <h3>Resistance level: {Math.round(s.manual_level)}%</h3>
          <div className="ride-tools">
            <Button onClick={() => cmd("setManualLevel", { level: Math.max(0, s.manual_level - 5) })}>− 5</Button>
            <input type="range" min={0} max={100} step={1} value={s.manual_level} onChange={(e) => cmd("setManualLevel", { level: Number(e.target.value) })} style={{ flex: 1 }} aria-label="Resistance level" />
            <Button onClick={() => cmd("setManualLevel", { level: Math.min(100, s.manual_level + 5) })}>+ 5</Button>
          </div>
        </section>
      )}

      <section className="panel">
        <div className="panel-head">
          <h3>Last 10 minutes</h3>
          {switchable.length > 0 && s.state !== "stopping" && (
            <div className="actions">
              <span className="small muted">Switch control:</span>
              {switchable.map((m) => (
                <Button key={m} kind="quiet" onClick={() => setSwitchTo(m)}>
                  {modeLabel[m]}
                </Button>
              ))}
            </div>
          )}
        </div>
        <SeriesChart
          rows={s.recent}
          height={140}
          series={[
            { index: 1, label: "Power", cls: "s-power" },
            { index: 4, label: "Target", cls: "s-target" },
            { index: 2, label: "Heart rate", cls: "s-hr" },
            { index: 3, label: "Cadence", cls: "s-cad" },
          ]}
        />
        <p className="small muted">Shortcuts: Space pause/resume · S stop · N skip interval · + / − intensity · L lap. Shortcuts are off while typing.</p>
      </section>

      {confirmStop && <StopDialog onClose={() => setConfirmStop(false)} />}
      {switchTo && (
        <Modal title="Switch resistance control?" onClose={() => setSwitchTo(null)}>
          <p>
            Only one thing controls the trainer at a time. Switching to <b>{modeLabel[switchTo]}</b> cancels anything still queued from the current mode, and the change is recorded in the
            ride.
          </p>
          {switchTo === "read_only" && <p className="muted">The trainer is first eased to a low load, then Ridgeline stops controlling it.</p>}
          <div className="actions">
            <Button
              kind="primary"
              onClick={async () => {
                const m = switchTo;
                setSwitchTo(null);
                await run("switch", () => rpc("switchMode", { mode: m }));
              }}
            >
              Switch
            </Button>
            <Button onClick={() => setSwitchTo(null)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function StopDialog({ onClose }: { onClose: () => void }) {
  const { run } = useAction();
  const [discard, setDiscard] = useState(false);
  return (
    <Modal title="Stop the ride?" onClose={onClose}>
      {!discard ? (
        <>
          <p>Ridgeline eases the trainer, asks it to stop, and saves the ride.</p>
          <div className="actions">
            <Button
              kind="primary"
              big
              onClick={() => {
                onClose();
                run("stop", () => rpc("stopSession", { save: true }));
              }}
            >
              Stop and save
            </Button>
            <Button onClick={onClose}>Keep riding</Button>
            <Button kind="quiet" onClick={() => setDiscard(true)}>
              Discard…
            </Button>
          </div>
        </>
      ) : (
        <>
          <p>Discarding deletes this ride's recording. This can't be undone.</p>
          <div className="actions">
            <Button
              kind="danger"
              onClick={() => {
                onClose();
                run("discard", () => rpc("stopSession", { save: false }));
              }}
            >
              Stop and discard
            </Button>
            <Button onClick={() => setDiscard(false)}>Back</Button>
          </div>
        </>
      )}
    </Modal>
  );
}

// ------------------------------------------------------------ finished

export function SummaryStats({ sum, units }: { sum: J; units: Units }) {
  if (!sum) return null;
  const cov = (x: J) => (x?.coverage != null && x.coverage < 0.95 ? `${Math.round(x.coverage * 100)}% coverage` : undefined);
  return (
    <div className="metric-row">
      <Stat label="Moving time" value={clock(sum.moving_s)} note={`timer ${clock(sum.timer_s)}`} />
      <Stat label="Distance" value={dist(sum.distance_m, units)} note="virtual" />
      <Stat label="Avg power" value={round(sum.power?.avg)} unit="W" note={cov(sum.power) ?? (sum.power?.max != null ? `max ${round(sum.power.max)} W` : undefined)} />
      <Stat label="Avg heart rate" value={round(sum.hr?.avg)} unit="bpm" note={cov(sum.hr) ?? (sum.hr?.max != null ? `max ${round(sum.hr.max)}` : undefined)} />
      <Stat label="Avg cadence" value={round(sum.cadence?.avg)} unit="rpm" note="excludes coasting" />
      <Stat label="Work" value={round(sum.work_kj)} unit="kJ" note={sum.energy_kcal_estimate != null ? `≈${round(sum.energy_kcal_estimate)} kcal (estimate)` : undefined} />
      {sum.ascent_m != null && <Stat label="Climbing" value={elev(sum.ascent_m, units)} />}
      {sum.adherence != null && <Stat label="On target" value={`${Math.round(sum.adherence * 100)}%`} note="within ±10% of ERG target" />}
    </div>
  );
}

function Finished({ s, activityId }: { s: J; activityId: string | null }) {
  const { units, go, bumpData, toast } = useApp();
  const { run, busy } = useAction();
  const [proposal, setProposal] = useState<string | null>(null);
  useEffect(() => {
    bumpData();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Ride saved</h1>
          <p className="sub">
            {s.demo && <span className="demo-flag">DEMO</span>} {s.workout?.name ?? s.route?.name ?? "Ride"}
          </p>
        </div>
        <div className="actions">
          {activityId && (
            <Button
              disabled={!!busy}
              onClick={() =>
                run("fit", async () => {
                  const r = await rpc("exportActivity", { id: activityId, format: "fit" });
                  toast(`FIT file saved: ${r.path}`);
                })
              }
            >
              Export FIT
            </Button>
          )}
          {activityId && <Button onClick={() => go("history", { id: activityId })}>Open in history</Button>}
          <Button kind="primary" onClick={() => run("close", () => rpc("closeSession", {}))}>
            Done
          </Button>
        </div>
      </div>
      {s.stop_confirmed === true && (
        <div className="banner banner-info">
          <p>The trainer confirmed the stop.</p>
        </div>
      )}
      {s.stop_confirmed === false && (
        <div className="banner banner-bad" role="alert">
          <p>
            <b>The trainer did not confirm the stop.</b> It may keep its last resistance. Stop pedalling if it feels hard, and power-cycle the trainer if needed.
          </p>
        </div>
      )}
      <section className="panel">
        <SummaryStats sum={s.summary} units={units} />
      </section>
      <section className="panel">
        <h2>How did it feel?</h2>
        <FeedbackForm activityId={activityId} onProposal={setProposal} />
      </section>
      {proposal && <ProposalModal id={proposal} onClose={() => setProposal(null)} />}
    </div>
  );
}

/** Post-ride feedback (RPE, fatigue, enjoyment, notes); may yield a plan proposal. */
export function FeedbackForm({ activityId, initial, onProposal }: { activityId: string | null; initial?: J; onProposal: (id: string) => void }) {
  const { run, busy } = useAction();
  const [fb, setFb] = useState({ rpe: initial?.rpe ?? 6, fatigue: initial?.fatigue ?? 3, enjoyment: initial?.enjoyment ?? 4, notes: initial?.notes ?? "" });
  const [sent, setSent] = useState<J | null>(null);
  const setProposal = onProposal;
  return (
    <>
        {!sent ? (
      <div className="stack">
        <div className="field">
          <span className="field-label">Effort (RPE)</span>
          <Scale name="Effort" value={fb.rpe} min={1} max={10} onChange={(v) => setFb({ ...fb, rpe: v })} labels={["very easy", "maximal"]} />
        </div>
        <div className="field">
          <span className="field-label">Fatigue now</span>
          <Scale name="Fatigue" value={fb.fatigue} min={1} max={5} onChange={(v) => setFb({ ...fb, fatigue: v })} labels={["fresh", "exhausted"]} />
        </div>
        <div className="field">
          <span className="field-label">Enjoyment</span>
          <Scale name="Enjoyment" value={fb.enjoyment} min={1} max={5} onChange={(v) => setFb({ ...fb, enjoyment: v })} labels={["not at all", "loved it"]} />
        </div>
        <Field label="Notes (optional)" wide>
          <textarea value={fb.notes} maxLength={1000} onChange={(e) => setFb({ ...fb, notes: e.target.value })} />
        </Field>
        <div className="actions">
          <Button
            kind="primary"
            disabled={!activityId || !!busy}
            onClick={() =>
              run("fb", async () => {
                const r = await rpc("submitFeedback", { activity_id: activityId, ...fb });
                setSent(r);
                if (r.proposal_id) setProposal(r.proposal_id);
              })
            }
          >
            Save feedback
          </Button>
        </div>
      </div>
    ) : (
      <div className="stack">
        {sent.safety ? (
          <div className="banner banner-bad" role="alert">
            <p>{sent.safety}</p>
          </div>
        ) : (
          <p>Thanks — saved. {sent.note ?? (sent.proposal_id ? "The coach suggested a change to your plan." : "Your plan looks right for now.")}</p>
        )}
        {sent.proposal_id && <Button onClick={() => setProposal(sent.proposal_id)}>Review suggested change</Button>}
      </div>
    )}
    </>
  );
}
