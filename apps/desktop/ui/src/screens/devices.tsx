// Devices: scan/connect, roles and data sources, trainer control status,
// reconnect help, and the demo simulator panel.
import { useState } from "react";
import { rpc, type J } from "../api";
import { useApp } from "../state";
import { Button, Empty, Field, Spinner, Status, Toggle } from "../ui";
import { round } from "../format";
import { useAction } from "../hooks";

const STATE_KIND: Record<string, "ok" | "warn" | "bad" | "idle" | "busy"> = {
  ready: "ok",
  connecting: "busy",
  discovering: "busy",
  subscribing: "busy",
  reconnecting: "warn",
  faulted: "bad",
  disconnected: "idle",
};

const ROLE_LABEL: Record<string, string> = { trainer: "Smart trainer", power: "Power", cadence: "Cadence", heart_rate: "Heart rate" };
const SERVICE_LABEL: Record<string, string> = { ftms: "trainer data", cycling_power: "power service", heart_rate: "heart-rate service", csc: "speed/cadence service" };

function age(ms: number | null | undefined): string {
  if (ms == null) return "no data yet";
  if (ms < 1500) return "live";
  return `${Math.round(ms / 1000)} s ago`;
}

export function Devices() {
  const { live, boot } = useApp();
  const { run, busy } = useAction();
  const [showAll, setShowAll] = useState(false);
  const d = live?.devices;
  const devices: J[] = (d?.devices ?? []).filter((x: J) => showAll || x.roles.length > 0 || x.want_connected);
  const scanning = (d?.adapters ?? []).some((a: J) => a.scanning);
  const bleAdapter = (d?.adapters ?? []).find((a: J) => a.kind === "ble");
  const ctl = d?.control;
  const rideActive = !!live?.ride_active;
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Devices</h1>
          <p className="sub">Connect a smart trainer (Bluetooth FTMS) and optional heart-rate, power and cadence sensors. Several can be connected at once.</p>
        </div>
        <div className="actions">
          <Toggle checked={showAll} onChange={setShowAll} label="Show all Bluetooth devices" />
          <Button kind="primary" disabled={scanning || !!busy} onClick={() => run("scan", () => rpc("scanDevices", {}))}>
            {scanning ? <Spinner label="Scanning…" /> : "Scan for devices"}
          </Button>
        </div>
      </div>

      {!boot?.ble_available && (
        <div className="banner banner-info" style={{ marginBottom: 16 }}>
          <p>This build has no Bluetooth access (developer server). Only simulated devices are available — turn on demo mode in Settings.</p>
        </div>
      )}
      {bleAdapter?.available === false && (
        <div className="banner banner-bad" style={{ marginBottom: 16 }}>
          <p>{bleAdapter.message ?? "Bluetooth is unavailable. Turn Bluetooth on, and on macOS allow Ridgeline in System Settings → Privacy & Security → Bluetooth."}</p>
        </div>
      )}

      <section className="panel">
        <div className="panel-head">
          <h2>Nearby and connected</h2>
          <span className="muted small">{scanning ? "Scanning for 15 seconds…" : "Not scanning"}</span>
        </div>
        {devices.length === 0 && (
          <Empty title="No devices found yet">
            Wake your devices first: pedal the trainer for a few seconds, wear the heart-rate strap (moisten the contacts), spin the cranks for a cadence sensor. Then scan. If a device
            doesn't appear, close other training apps and phone apps that may be connected to it — most sensors accept only one or two connections.
          </Empty>
        )}
        {devices.map((x) => (
          <div className="dev" key={x.key}>
            <div>
              <div className="dev-name">
                {x.name} {x.simulated && <span className="tag tag-sign">simulated</span>} {x.is_trainer && <span className="tag">controllable trainer</span>}
              </div>
              <div className="dev-meta">
                <span>{x.roles.map((r: string) => ROLE_LABEL[r] ?? r).join(" · ") || "Unknown device type"}</span>
                {x.rssi != null && <span>signal {x.rssi} dBm</span>}
                {x.battery != null && <span className={x.battery < 15 ? "warn-text" : ""}>battery {x.battery}%{x.battery < 15 ? " — low" : ""}</span>}
                {x.state === "ready" && <span>data {age(x.last_data_age_ms)}</span>}
              </div>
              {(x.manufacturer || x.model || x.firmware) && (
                <div className="dev-meta">
                  {[x.manufacturer, x.model, x.firmware && `firmware ${x.firmware}`].filter(Boolean).join(" · ")}
                </div>
              )}
              {x.caps && (
                <div className="dev-meta">
                  Supports: {x.caps.erg ? "ERG target power" : "no ERG"} · {x.caps.simulation ? "road simulation" : "no road simulation"} · {x.caps.resistance ? "resistance level" : "no level control"}
                  {x.caps.power_max != null && ` · power range ${x.caps.power_min}–${x.caps.power_max} W`}
                </div>
              )}
            </div>
            <div>
              <Status kind={STATE_KIND[x.state] ?? "idle"}>
                {x.state}
                {x.state === "reconnecting" && ` (attempt ${x.reconnect_attempt})`}
              </Status>
              {x.error && <div className="small error-text">{x.error}</div>}
              {x.parse_errors > 0 && <div className="small muted">{x.parse_errors} unreadable packet(s) ignored</div>}
            </div>
            <div className="actions">
              {(x.state === "disconnected" || x.state === "faulted") && (
                <Button kind="primary" disabled={!!busy} onClick={() => run("connect", () => rpc("connectDevice", { key: x.key }))}>
                  Connect
                </Button>
              )}
              {x.state !== "disconnected" && x.state !== "faulted" && (
                <Button disabled={!!busy || (rideActive && x.is_trainer)} onClick={() => run("disconnect", () => rpc("disconnectDevice", { key: x.key }))}>
                  Disconnect
                </Button>
              )}
              {x.roles.includes("trainer") && !x.is_trainer && (
                <Button disabled={!!busy || rideActive} onClick={() => run("trainer", () => rpc("setTrainer", { key: x.key }))}>
                  Use as trainer
                </Button>
              )}
              <Button kind="quiet" disabled={!!busy || rideActive} onClick={() => run("forget", () => rpc("forgetDevice", { key: x.key }))}>
                Forget
              </Button>
            </div>
          </div>
        ))}
      </section>

      <div className="grid-2" style={{ marginTop: 18 }}>
        <section className="panel">
          <h2>Data sources</h2>
          <p className="muted small">
            Ridgeline uses one source per metric. “Automatic” prefers the trainer for power and cadence. Choosing an external power meter records its values; ERG targets are still controlled by
            the trainer (no power matching), so a difference between the two is normal and shown here.
          </p>
          <div className="sources">
            <SourcePicker metric="power" label="Power" unit="W" candidates={live?.live?.power_candidates} assigned={live?.live?.assigned?.power} reading={live?.live?.power} />
            <SourcePicker metric="cadence" label="Cadence" unit="rpm" candidates={live?.live?.cadence_candidates} assigned={live?.live?.assigned?.cadence} reading={live?.live?.cadence} />
            <SourcePicker metric="heart_rate" label="Heart rate" unit="bpm" candidates={live?.live?.hr_candidates} assigned={live?.live?.assigned?.heart_rate} reading={live?.live?.heart_rate} />
          </div>
        </section>
        <section className="panel">
          <h2>Trainer control</h2>
          {!ctl?.device && <p className="muted">No controllable trainer is selected.</p>}
          {ctl?.device && (
            <div className="stack">
              <Status kind={ctl.state === "controlled" ? "ok" : ctl.state === "denied" || ctl.state === "lost" ? "bad" : ctl.state === "uncertain" ? "warn" : "idle"}>
                Control: {ctl.state.replace("_", " ")}
              </Status>
              <p className="small muted">
                Ridgeline requests control when a ride starts. If control is denied, another app (or a phone) may be connected to the trainer — close it and try again. After a disconnect,
                control is never resumed automatically.
              </p>
              <div className="small">
                Last acknowledged target: <b>{ctl.acked_target ?? "—"}</b>
                <br />
                Commands sent: {ctl.commands_sent} · generation {ctl.generation}
                {ctl.in_flight && (
                  <>
                    <br />
                    Waiting for acknowledgment: {ctl.in_flight}
                  </>
                )}
              </div>
              {!rideActive && ctl.state !== "controlled" && (
                <Button disabled={!!busy} onClick={() => run("ctl", () => rpc("requestControl", {}), "Control requested.")}>
                  Test control request
                </Button>
              )}
              <details>
                <summary>Command log</summary>
                <div className="log">
                  {(live?.control_log ?? []).map((l: J, i: number) => (
                    <div key={i}>
                      {l.command} → {l.outcome}
                      {l.latency_ms != null && ` (${l.latency_ms} ms)`}
                    </div>
                  ))}
                </div>
              </details>
              <p className="small muted">Calibration (spin-down) is not offered: use the trainer manufacturer's app and procedure. Ridgeline never sends guessed proprietary commands.</p>
            </div>
          )}
        </section>
      </div>

      {live?.demo && live?.sim && <SimPanel sim={live.sim} />}

      <section className="panel">
        <h3>Device log</h3>
        <div className="log">
          {(d?.log ?? []).map((l: J, i: number) => (
            <div key={i}>
              <span className={l.level === "error" ? "error-text" : ""}>{l.device}</span>: {l.message}
            </div>
          ))}
          {(d?.log ?? []).length === 0 && <span>Nothing yet.</span>}
        </div>
      </section>
    </div>
  );
}

function SourcePicker(props: { metric: string; label: string; unit: string; candidates: J[] | undefined; assigned: J | null | undefined; reading: J | undefined }) {
  const { run } = useAction();
  const cands = props.candidates ?? [];
  const cur = props.assigned ? `${props.assigned.device}|${props.assigned.service}` : "";
  return (
    <Field label={props.label}>
      <select
        value={cur}
        onChange={(e) => {
          const v = e.target.value;
          const [device, service] = v.split("|");
          run("assign", () => rpc("assignSource", v ? { metric: props.metric, device, service } : { metric: props.metric, device: null }));
        }}
      >
        <option value="">Automatic</option>
        {cands.map((c) => {
          const k = `${c.source.device}|${c.source.service}`;
          return (
            <option key={k} value={k}>
              {c.source.device.replace(/^(ble|simulator):/, "")} — {SERVICE_LABEL[c.source.service] ?? c.source.service}: {c.freshness === "fresh" ? `${round(c.value)} ${props.unit}` : c.freshness}
            </option>
          );
        })}
      </select>
      <span className="field-hint">
        Now: {props.reading?.freshness === "fresh" ? `${round(props.reading.value)} ${props.unit}` : props.reading?.freshness === "no_source" ? "no source" : `${props.reading?.freshness} (not shown as a value)`}
      </span>
    </Field>
  );
}

const FAULTS: { id: string; label: string }[] = [
  { id: "offline", label: "Out of range / off" },
  { id: "stale", label: "Stops sending data" },
  { id: "malformed", label: "Malformed packets" },
  { id: "deny_control", label: "Deny control" },
  { id: "drop_acks", label: "Drop acknowledgments" },
  { id: "slow_acks", label: "Slow acknowledgments" },
  { id: "fail_writes", label: "Write failures" },
  { id: "busy", label: "Held by another app" },
  { id: "no_simulation", label: "No simulation mode" },
  { id: "low_power_limit", label: "300 W limit" },
  { id: "low_grade_limit", label: "6% grade limit" },
  { id: "low_battery", label: "Low battery" },
];

function SimPanel({ sim }: { sim: J }) {
  const { run } = useAction();
  const [dev, setDev] = useState("sim-trainer");
  const f = (sim.faults ?? []).find((x: J) => x.device === dev) ?? {};
  return (
    <section className="panel">
      <div className="panel-head">
        <h2>Demo simulator</h2>
        <span className="tag tag-sign">simulated — not hardware evidence</span>
      </div>
      <div className="cols">
        <Field label={`Simulated rider effort: ${Math.round(sim.effort_w)} W`} hint="Used in free rides and manual mode (ERG sets power itself).">
          <input type="range" min={0} max={600} step={5} value={sim.effort_w} onChange={(e) => run("sim", () => rpc("simRider", { effort_w: Number(e.target.value) }))} />
        </Field>
        <Field label={`Simulated cadence: ${Math.round(sim.cadence_rpm)} rpm`} hint="Drop below 40 rpm in ERG to see low-cadence protection.">
          <input type="range" min={0} max={130} step={1} value={sim.cadence_rpm} onChange={(e) => run("sim", () => rpc("simRider", { cadence_rpm: Number(e.target.value) }))} />
        </Field>
        <Field label="Inject faults on">
          <select value={dev} onChange={(e) => setDev(e.target.value)}>
            <option value="sim-trainer">Sim Trainer</option>
            <option value="sim-hrm">Sim HR Strap</option>
            <option value="sim-cadence">Sim Cadence Sensor</option>
            <option value="sim-power">Sim Power Pedals</option>
          </select>
        </Field>
      </div>
      <div className="fault-grid">
        {FAULTS.map((x) => (
          <Toggle key={x.id} checked={!!f[x.id]} onChange={(on) => run("fault", () => rpc("simFault", { device: dev, fault: x.id, on }))} label={x.label} />
        ))}
      </div>
    </section>
  );
}
