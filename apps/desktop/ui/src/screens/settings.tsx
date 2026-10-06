// Settings: profile, FTP history, trainer behaviour, AI coach, map and
// online services, privacy and data, about. Paid/remote integrations are
// off by default and are never enabled automatically.
import { useEffect, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Button, ErrorText, Field, Modal, NumberInput, Segmented, Spinner, Toggle } from "../ui";
import { dateLabel, mass, massToKg, massUnit, WEEKDAYS, type Units } from "../format";
import { useAction, useJob } from "../hooks";
import { CATEGORIES, GOALS, MINUTE_OPTIONS } from "./onboarding";
import { speechAvailable } from "../ridecoach";

type Tab = "profile" | "fitness" | "trainer" | "ai" | "map" | "privacy" | "about";
const TABS: { value: Tab; label: string }[] = [
  { value: "profile", label: "Profile" },
  { value: "fitness", label: "FTP" },
  { value: "trainer", label: "Trainer & display" },
  { value: "ai", label: "AI coach" },
  { value: "map", label: "Map & services" },
  { value: "privacy", label: "Privacy & data" },
  { value: "about", label: "About" },
];

const PRESETS: Record<string, { label: string; url: string; model: string }> = {
  ollama: { label: "Ollama", url: "http://localhost:11434/v1", model: "llama3.2" },
  lmstudio: { label: "LM Studio", url: "http://localhost:1234/v1", model: "local-model" },
  custom: { label: "Other OpenAI-compatible server", url: "http://localhost:8080/v1", model: "local-model" },
};

export function Settings() {
  const { screen } = useApp();
  const [tab, setTab] = useState<Tab>((screen.params?.tab as Tab) ?? "profile");
  useEffect(() => {
    if (screen.params?.tab) setTab(screen.params.tab as Tab);
  }, [screen.params?.tab]);
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Settings</h1>
          <p className="sub">Everything is stored on this computer.</p>
        </div>
      </div>
      <div className="tabs" role="tablist" aria-label="Settings sections">
        {TABS.map((t) => (
          <button key={t.value} role="tab" aria-selected={tab === t.value} className={`tab ${tab === t.value ? "active" : ""}`} onClick={() => setTab(t.value)}>
            {t.label}
          </button>
        ))}
      </div>
      {tab === "profile" && <ProfileTab />}
      {tab === "fitness" && <FtpTab />}
      {(tab === "trainer" || tab === "ai" || tab === "map") && <SettingsForm tab={tab} />}
      {tab === "privacy" && <PrivacyTab />}
      {tab === "about" && <AboutTab />}
    </div>
  );
}

// ---------------------------------------------------------------- profile

function ProfileTab() {
  const { refreshBoot, bumpData } = useApp();
  const prof = useRpc("getProfile", {});
  const [p, setP] = useState<J | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const { run, busy } = useAction();
  useEffect(() => {
    if (prof.data?.profile) setP(prof.data.profile);
  }, [prof.data]);
  if (prof.loading || !p) return <Spinner />;
  const u: Units = p.units;
  const set = (k: string, v: unknown) => setP((x: J) => ({ ...x, [k]: v }));
  const cat = (id: string) => (p.preferred_categories.includes(id) ? "prefer" : p.avoided_categories.includes(id) ? "avoid" : "none");
  const setCat = (id: string, v: string) =>
    setP((x: J) => ({
      ...x,
      preferred_categories: v === "prefer" ? [...x.preferred_categories.filter((c: string) => c !== id), id] : x.preferred_categories.filter((c: string) => c !== id),
      avoided_categories: v === "avoid" ? [...x.avoided_categories.filter((c: string) => c !== id), id] : x.avoided_categories.filter((c: string) => c !== id),
    }));
  const save = () =>
    run(
      "save",
      async () => {
        setErr(null);
        if (p.availability_min.every((m: number) => m === 0)) throw new Error("Choose at least one day you can ride.");
        await rpc("saveProfile", { profile: p });
        await refreshBoot();
        bumpData();
      },
      "Profile saved. Ask the coach for a new plan if your week or goal changed.",
    );
  return (
    <section className="panel">
      <p className="small muted">Level: {prof.data?.level_label ?? "—"} (from your experience and recent riding; it sets which workouts the coach may schedule).</p>
      <div className="cols">
        <Field label="Name (optional)">
          <input type="text" value={p.display_name} maxLength={60} onChange={(e) => set("display_name", e.target.value)} />
        </Field>
        <Field label="Goal">
          <select value={p.goal} onChange={(e) => set("goal", e.target.value)}>
            {GOALS.map((g) => (
              <option key={g.v} value={g.v}>
                {g.t}
              </option>
            ))}
          </select>
        </Field>
        {p.goal === "event" && (
          <>
            <Field label="Event date">
              <input type="date" value={p.event_date ?? ""} onChange={(e) => set("event_date", e.target.value || null)} />
            </Field>
            <Field label="Event name">
              <input type="text" value={p.event_name} maxLength={120} onChange={(e) => set("event_name", e.target.value)} />
            </Field>
          </>
        )}
        <Field label="Experience">
          <Segmented
            value={p.experience}
            onChange={(v) => set("experience", v)}
            options={[
              { value: "new", label: "New" },
              { value: "some", label: "Some" },
              { value: "experienced", label: "Experienced" },
            ]}
          />
        </Field>
        <Field label="Riding per week lately">
          <NumberInput value={p.recent_weekly_min} min={0} max={2400} step={15} suffix="minutes" onChange={(v) => set("recent_weekly_min", v ?? 0)} />
        </Field>
        <Field label="Consistent weeks (of last 12)">
          <NumberInput value={p.consistent_weeks} min={0} max={12} onChange={(v) => set("consistent_weeks", Math.min(12, v ?? 0))} />
        </Field>
        <Field label="Weeks off">
          <NumberInput value={p.weeks_off} min={0} max={520} onChange={(v) => set("weeks_off", v ?? 0)} />
        </Field>
      </div>
      <h3>Your week</h3>
      <div className="days">
        {WEEKDAYS.map((d, i) => (
          <div className="day-col" key={d}>
            <b>{d}</b>
            <select
              aria-label={`${d} minutes`}
              value={p.availability_min[i]}
              onChange={(e) => {
                const a = [...p.availability_min];
                a[i] = Number(e.target.value);
                set("availability_min", a);
              }}
            >
              {MINUTE_OPTIONS.map((m) => (
                <option key={m} value={m}>
                  {m === 0 ? "Rest" : `${m} min`}
                </option>
              ))}
            </select>
          </div>
        ))}
      </div>
      <div className="cols" style={{ marginTop: 12 }}>
        <Field label="Long-ride day">
          <select value={p.long_ride_day ?? ""} onChange={(e) => set("long_ride_day", e.target.value === "" ? null : Number(e.target.value))}>
            <option value="">No preference</option>
            {WEEKDAYS.map((d, i) => (
              <option key={d} value={i} disabled={p.availability_min[i] === 0}>
                {d}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Other demanding activities">
          <input type="text" value={p.competing_activities} maxLength={300} onChange={(e) => set("competing_activities", e.target.value)} />
        </Field>
        <Field label="Units">
          <Segmented
            value={p.units}
            onChange={(v) => set("units", v)}
            options={[
              { value: "metric", label: "km · kg" },
              { value: "imperial", label: "mi · lb" },
            ]}
          />
        </Field>
        <Field label="Your mass">
          <NumberInput value={mass(p.rider_mass_kg, u)} min={1} max={600} step={0.5} suffix={massUnit(u)} onChange={(v) => set("rider_mass_kg", v == null ? 0 : massToKg(v, u))} />
        </Field>
        <Field label="Bike mass">
          <NumberInput value={mass(p.bike_mass_kg, u)} min={1} max={100} step={0.5} suffix={massUnit(u)} onChange={(v) => set("bike_mass_kg", v == null ? 0 : massToKg(v, u))} />
        </Field>
        <Field label="Max heart rate (optional)">
          <NumberInput value={p.max_hr} min={100} max={230} suffix="bpm" onChange={(v) => set("max_hr", v)} />
        </Field>
        <Field label="Threshold heart rate (optional)">
          <NumberInput value={p.threshold_hr} min={80} max={220} suffix="bpm" onChange={(v) => set("threshold_hr", v)} />
        </Field>
        <Field label="Power on your setup">
          <select value={p.power_source} onChange={(e) => set("power_source", e.target.value)}>
            <option value="measured">Measured</option>
            <option value="estimated">Estimated by the trainer</option>
            <option value="none">No power data</option>
          </select>
        </Field>
        <Field label="Limitations (private)" wide>
          <textarea value={p.limitations} maxLength={1000} onChange={(e) => set("limitations", e.target.value)} />
        </Field>
      </div>
      <h3>Workout preferences</h3>
      <div className="pref-grid">
        {CATEGORIES.map((c) => (
          <div key={c.id} className="pref-row">
            <span>{c.label}</span>
            <Segmented
              value={cat(c.id)}
              onChange={(v) => setCat(c.id, v)}
              label={c.label}
              options={[
                { value: "prefer", label: "More" },
                { value: "none", label: "—" },
                { value: "avoid", label: "Avoid" },
              ]}
            />
          </div>
        ))}
      </div>
      <ErrorText>{err}</ErrorText>
      <div className="form-foot">
        <Button kind="primary" onClick={save} disabled={!!busy}>
          Save profile
        </Button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- FTP

function FtpTab() {
  const { boot, refreshBoot, bumpData } = useApp();
  const prof = useRpc("getProfile", {});
  const { run, busy } = useAction();
  const [f, setF] = useState<J>({ watts: null, method: "known", effective: boot?.today, confidence: "medium", note: "" });
  const [del, setDel] = useState<J | null>(null);
  const hist: J[] = prof.data?.ftp_history ?? [];
  const after = async () => {
    prof.reload();
    await refreshBoot();
    bumpData();
  };
  return (
    <>
      <section className="panel">
        <h2>Current FTP</h2>
        {prof.data?.ftp ? (
          <p>
            <b className="big-num">{Math.round(prof.data.ftp.watts)} W</b> since {dateLabel(prof.data.ftp.effective)} · {prof.data.ftp.method.replace(/_/g, " ")} · {prof.data.ftp.confidence} confidence
            {prof.data.ftp.provisional && " · provisional"}
          </p>
        ) : (
          <p className="muted">Not set. Easy rides use perceived effort; power targets need an FTP. You can ride an assessment from the Workouts screen.</p>
        )}
        <p className="small muted">Changing FTP affects future workouts. Rides already recorded keep the FTP they were ridden with.</p>
      </section>
      <section className="panel">
        <h3>Add an FTP value</h3>
        <div className="cols">
          <Field label="FTP">
            <NumberInput value={f.watts} min={40} max={600} suffix="W" onChange={(v) => setF({ ...f, watts: v })} />
          </Field>
          <Field label="Source">
            <select value={f.method} onChange={(e) => setF({ ...f, method: e.target.value })}>
              <option value="known">From another app or coach</option>
              <option value="ramp_test">Ramp test</option>
              <option value="twenty_minute_test">20-minute test</option>
              <option value="manual">Rough guess</option>
            </select>
          </Field>
          <Field label="Effective date">
            <input type="date" value={f.effective ?? ""} max={boot?.today} onChange={(e) => setF({ ...f, effective: e.target.value })} />
          </Field>
          <Field label="Confidence">
            <Segmented
              value={f.confidence}
              onChange={(v) => setF({ ...f, confidence: v })}
              options={[
                { value: "low", label: "Low" },
                { value: "medium", label: "Medium" },
                { value: "high", label: "High" },
              ]}
            />
          </Field>
          <Field label="Note (optional)" wide>
            <input type="text" value={f.note} maxLength={300} onChange={(e) => setF({ ...f, note: e.target.value })} />
          </Field>
        </div>
        <div className="form-foot">
          <Button
            kind="primary"
            disabled={!!busy || f.watts == null}
            onClick={() =>
              run(
                "add",
                async () => {
                  await rpc("addFtp", f);
                  setF({ ...f, watts: null, note: "" });
                  await after();
                },
                "FTP saved.",
              )
            }
          >
            Add
          </Button>
        </div>
      </section>
      <section className="panel">
        <h3>History</h3>
        {hist.length === 0 && <p className="muted">No entries.</p>}
        {hist.length > 0 && (
          <table className="table">
            <thead>
              <tr>
                <th>Date</th>
                <th className="num">FTP</th>
                <th>Source</th>
                <th>Confidence</th>
                <th>Note</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {hist.map((e) => (
                <tr key={e.id}>
                  <td>{dateLabel(e.effective)}</td>
                  <td className="num">{Math.round(e.watts)} W</td>
                  <td>
                    {e.method.replace(/_/g, " ")}
                    {e.provisional && " (provisional)"}
                  </td>
                  <td>{e.confidence}</td>
                  <td className="small">{e.note}</td>
                  <td>
                    <Button kind="quiet" onClick={() => setDel(e)}>
                      Delete
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      {del && (
        <Modal title="Delete this FTP entry?" onClose={() => setDel(null)}>
          <p>
            {Math.round(del.watts)} W from {dateLabel(del.effective)} will be removed. Your current FTP becomes the most recent remaining entry.
          </p>
          <div className="actions">
            <Button kind="danger" onClick={() => run("del", () => rpc("deleteFtp", { id: del.id }).then(() => (setDel(null), after())))}>
              Delete
            </Button>
            <Button onClick={() => setDel(null)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </>
  );
}

// ---------------------------------------------------------------- settings (trainer / AI / map)

function SettingsForm({ tab }: { tab: "trainer" | "ai" | "map" }) {
  const { refreshBoot, bumpData, live, boot } = useApp();
  const res = useRpc("getSettings", {});
  const [s, setS] = useState<J | null>(null);
  const [dirty, setDirty] = useState(false);
  const [key, setKey] = useState("");
  const [consent, setConsent] = useState<J | null>(null);
  const [test, setTest] = useState<J | null>(null);
  const { run, busy } = useAction();
  const job = useJob();
  useEffect(() => {
    if (res.data?.settings) {
      setS(res.data.settings);
      setDirty(false);
    }
  }, [res.data]);
  useEffect(() => {
    if (boot?.profile) setConsent(boot.profile.ai_consent);
  }, [boot?.profile]);
  if (!s) return <Spinner />;
  const upd = (path: string, v: unknown) => {
    const [a, b] = path.split(".");
    setS((x: J) => (b ? { ...x, [a]: { ...x[a], [b]: v } } : { ...x, [a]: v }));
    setDirty(true);
  };
  const save = () =>
    run(
      "save",
      async () => {
        const saved = await rpc("saveSettings", { settings: s });
        setS(saved);
        setDirty(false);
        await refreshBoot();
        bumpData();
        res.reload();
      },
      "Settings saved.",
    );
  const saveConsent = (c: J) =>
    run("consent", async () => {
      const prof = await rpc("getProfile", {});
      await rpc("saveProfile", { profile: { ...prof.profile, ai_consent: c } });
      setConsent(c);
      await refreshBoot();
      res.reload();
    });
  const t = s.trainer;
  const ai = s.ai;
  const riding = !!live?.ride_active;
  return (
    <section className="panel">
      {tab === "trainer" && (
        <>
          <h2>Display</h2>
          <div className="cols">
            <Field label="Theme">
              <Segmented
                value={s.theme}
                onChange={(v) => upd("theme", v)}
                options={[
                  { value: "dark", label: "Dark" },
                  { value: "light", label: "Light" },
                  { value: "system", label: "System" },
                ]}
              />
            </Field>
            <Field label="Units" hint="Also changeable in your profile.">
              <Segmented
                value={s.units}
                onChange={(v) => upd("units", v)}
                options={[
                  { value: "metric", label: "Metric" },
                  { value: "imperial", label: "Imperial" },
                ]}
              />
            </Field>
          </div>
          <div style={{ marginTop: 12 }}>
            <Toggle checked={s.demo_mode} onChange={(v) => upd("demo_mode", v)} label="Demo mode" hint="Adds a simulated trainer, power meter and heart-rate strap. Demo rides are labelled and never change your plan or FTP." />
          </div>
          {s.ride && (
            <>
              <h2 style={{ marginTop: 20 }}>Rides</h2>
              <Toggle
                checked={s.ride.auto_pause}
                onChange={(v) => upd("ride.auto_pause", v)}
                label="Auto-pause when I stop pedalling"
                hint="The timer and workout pause after 3 s without pedalling and resume when you pedal again; the target ramps back in. On free rides, coasting doesn't count as stopping. Can be changed during a ride."
              />
            </>
          )}
          <h2 style={{ marginTop: 20 }}>Trainer behaviour on routes</h2>
          {riding && <p className="warn-text small">Trainer settings are locked during a ride.</p>}
          <div className="cols">
            <Field label="Default difficulty" hint="Share of the road gradient sent to the trainer. Distance and climbing in your ride stay real.">
              <NumberInput value={t.difficulty_pct} min={0} max={100} step={5} suffix="%" onChange={(v) => upd("trainer.difficulty_pct", v ?? 100)} />
            </Field>
            <Field label="Steepest descent sent" hint="Lower limit for the simulated gradient.">
              <NumberInput value={t.grade_min_pct} min={-20} max={0} step={1} suffix="%" onChange={(v) => upd("trainer.grade_min_pct", v ?? -10)} />
            </Field>
            <Field label="Steepest climb sent" hint="Upper limit; protects knees and trainers that can't hold steep grades.">
              <NumberInput value={t.grade_max_pct} min={0} max={25} step={1} suffix="%" onChange={(v) => upd("trainer.grade_max_pct", v ?? 20)} />
            </Field>
            <Field label="Gradient change rate" hint="Maximum change per second, so resistance never jumps.">
              <NumberInput value={t.slew_pct_per_s} min={0.2} max={10} step={0.1} suffix="%/s" onChange={(v) => upd("trainer.slew_pct_per_s", v ?? 1.5)} />
            </Field>
            <Field label="Look-ahead" hint="Compensates for trainer lag (0–30 m).">
              <NumberInput value={t.lookahead_m} min={0} max={30} step={1} suffix="m" onChange={(v) => upd("trainer.lookahead_m", v ?? 0)} />
            </Field>
          </div>
          <details style={{ marginTop: 14 }}>
            <summary>Advanced control timing</summary>
            <div className="cols" style={{ marginTop: 10 }}>
              <Field label="Data considered stale after" hint="1–15 s. Stale power is never shown as live.">
                <NumberInput value={t.stale_ms} min={1000} max={15000} step={500} suffix="ms" onChange={(v) => upd("trainer.stale_ms", v ?? 3000)} />
              </Field>
              <Field label="Minimum interval between targets" hint="0.5–5 s. Trainers dislike frequent commands.">
                <NumberInput value={t.target_interval_ms} min={500} max={5000} step={100} suffix="ms" onChange={(v) => upd("trainer.target_interval_ms", v ?? 1000)} />
              </Field>
              <Field label="Command acknowledgement timeout">
                <NumberInput value={t.ack_timeout_ms} min={1000} max={10000} step={500} suffix="ms" onChange={(v) => upd("trainer.ack_timeout_ms", v ?? 3000)} />
              </Field>
            </div>
          </details>
        </>
      )}

      {tab === "ai" && (
        <>
          <h2>Coach mode</h2>
          <Segmented
            value={ai.provider}
            label="Coach mode"
            onChange={(v) => upd("ai.provider", v)}
            options={[
              { value: "offline", label: "Offline coach" },
              { value: "local", label: "Local AI model" },
              { value: "remote", label: "Remote service" },
            ]}
          />
          {ai.provider === "offline" && (
            <p className="muted" style={{ marginTop: 10 }}>
              The offline coach builds and adapts plans with deterministic rules and answers common questions. Nothing leaves this computer.
            </p>
          )}
          {ai.provider === "local" && (
            <div className="stack" style={{ marginTop: 12 }}>
              <p className="small">
                Runs a free, open model on this computer — no account, no cost, no data leaves your machine. To set up Ollama: install it from ollama.com, run{" "}
                <code>ollama pull llama3.2</code>, and keep it running. A model of 3B+ parameters works; larger models give better explanations but need more memory.
              </p>
              <div className="cols">
                <Field label="Server">
                  <select
                    value={ai.preset}
                    onChange={(e) => {
                      const pr = PRESETS[e.target.value];
                      setS((x: J) => ({ ...x, ai: { ...x.ai, preset: e.target.value, base_url: pr.url, model: e.target.value === "custom" ? x.ai.model : pr.model } }));
                      setDirty(true);
                    }}
                  >
                    {Object.entries(PRESETS).map(([k, v]) => (
                      <option key={k} value={k}>
                        {v.label}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Endpoint" hint="Must be on this computer (localhost).">
                  <input type="url" value={ai.base_url} onChange={(e) => upd("ai.base_url", e.target.value)} />
                </Field>
                <Field label="Model">
                  <input type="text" value={ai.model} maxLength={120} onChange={(e) => upd("ai.model", e.target.value)} list="ai-models" />
                  <datalist id="ai-models">
                    {(test?.models ?? []).map((m: string) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                </Field>
              </div>
            </div>
          )}
          {ai.provider === "remote" && (
            <div className="stack" style={{ marginTop: 12 }}>
              <div className="banner banner-warn">
                <p>
                  <b>Remote AI services may charge for use.</b> Ridgeline never signs up, buys credits or enters payment details. Only enable this for a service you have set up yourself and whose
                  costs you understand. Requests are capped per day below.
                </p>
              </div>
              <Toggle checked={ai.remote_enabled} onChange={(v) => upd("ai.remote_enabled", v)} label="I set up this service myself and want Ridgeline to use it" />
              <div className="cols">
                <Field label="Endpoint (OpenAI-compatible)">
                  <input type="url" value={ai.base_url} onChange={(e) => upd("ai.base_url", e.target.value)} />
                </Field>
                <Field label="Model">
                  <input type="text" value={ai.model} maxLength={120} onChange={(e) => upd("ai.model", e.target.value)} />
                </Field>
              </div>
              <Field label="API key" hint={res.data?.secure_store ? "Stored in your operating system's credential store, never in Ridgeline's files or logs." : "No secure credential store is available, so keys can't be saved in this build."}>
                <div className="actions">
                  <input type="password" autoComplete="off" value={key} placeholder={res.data?.has_ai_key ? "•••••••• (saved)" : "Paste key"} onChange={(e) => setKey(e.target.value)} style={{ maxWidth: 320 }} />
                  <Button disabled={!key.trim() || !res.data?.secure_store} onClick={() => run("key", () => rpc("setSecret", { name: "ai_api_key", value: key }).then(() => (setKey(""), res.reload())), "Key saved.")}>
                    Save key
                  </Button>
                  {res.data?.has_ai_key && (
                    <Button kind="quiet" onClick={() => run("key", () => rpc("clearSecret", { name: "ai_api_key" }).then(res.reload), "Key removed.")}>
                      Remove key
                    </Button>
                  )}
                </div>
              </Field>
            </div>
          )}
          {ai.provider !== "offline" && (
            <>
              <details style={{ marginTop: 12 }}>
                <summary>Limits</summary>
                <div className="cols" style={{ marginTop: 10 }}>
                  <Field label="Requests per day">
                    <NumberInput value={ai.max_requests_per_day} min={1} max={1000} onChange={(v) => upd("ai.max_requests_per_day", v ?? 100)} />
                  </Field>
                  <Field label="Timeout" hint="Local models can be slow on the first request.">
                    <NumberInput value={ai.timeout_s} min={10} max={900} suffix="s" onChange={(v) => upd("ai.timeout_s", v ?? 180)} />
                  </Field>
                  <Field label="Max reply tokens">
                    <NumberInput value={ai.max_tokens} min={200} max={8000} step={100} onChange={(v) => upd("ai.max_tokens", v ?? 1800)} />
                  </Field>
                </div>
              </details>
              <div className="actions" style={{ marginTop: 12 }}>
                <Button
                  disabled={dirty || !!job.progress}
                  title={dirty ? "Save first" : undefined}
                  onClick={async () => {
                    setTest(null);
                    const r = await job.start("testAi", {});
                    if (r) setTest(r);
                  }}
                >
                  Test connection
                </Button>
                {job.progress && <Spinner label={job.progress} />}
                {dirty && <span className="small muted">Save your changes before testing.</span>}
              </div>
              {test && (
                <div className="banner banner-info" style={{ marginTop: 10 }}>
                  <p>
                    Connected to <b>{test.model}</b> in {(test.latency_ms / 1000).toFixed(1)} s{test.local ? " (on this computer)" : ""}. {test.models?.length ? `Available models: ${test.models.slice(0, 8).join(", ")}.` : ""}
                  </p>
                </div>
              )}
            </>
          )}
          <h2 style={{ marginTop: 22 }}>Consent</h2>
          {consent && (
            <div className="stack">
              <Toggle
                checked={consent.enabled}
                onChange={(v) => saveConsent({ ...consent, enabled: v })}
                label="Allow sending a compact summary of my profile and plan to the AI model"
                hint="Without this, the offline coach is used even if a model is configured."
              />
              <Toggle checked={consent.share_activity_summaries} onChange={(v) => saveConsent({ ...consent, share_activity_summaries: v })} label="Include short summaries of recent rides" />
              <Toggle checked={consent.share_limitations} onChange={(v) => saveConsent({ ...consent, share_limitations: v })} label="Include the limitations I described" />
              <details>
                <summary>Preview exactly what would be sent</summary>
                <pre className="code">{JSON.stringify(res.data?.ai_summary_preview ?? null, null, 2)}</pre>
              </details>
            </div>
          )}
        </>
      )}

      {tab === "ai" && s.ride_coach && (
        <>
          <h2 style={{ marginTop: 24 }}>During rides</h2>
          <p className="small muted">
            The coach appears on the Ride screen. It never changes resistance: suggestions such as “Easier 5%” are buttons you press, the same as the ride controls.
          </p>
          <div className="stack">
            <Toggle
              checked={s.ride_coach.cues}
              onChange={(v) => upd("ride_coach.cues", v)}
              label="Ride cues"
              hint="Previews of the next interval, cadence reminders, climbs ahead and an “ease off?” suggestion when you're well under target. Works offline."
            />
            <Toggle
              checked={s.ride_coach.ai_moments}
              onChange={(v) => upd("ride_coach.ai_moments", v)}
              label="AI comments at key moments"
              hint={
                ai.provider === "offline"
                  ? "Needs the local AI model (above). The quick prompts on the Ride screen use the offline coach until then."
                  : "When a hard interval starts, at halfway, for the last interval and before long climbs."
              }
            />
            <Field label="At most one AI comment every" hint="60–1800 seconds. Your own questions are always answered.">
              <NumberInput value={s.ride_coach.moment_gap_s} min={60} max={1800} step={30} suffix="s" onChange={(v) => upd("ride_coach.moment_gap_s", v ?? 120)} />
            </Field>
            <Toggle
              checked={s.ride_coach.voice}
              onChange={(v) => upd("ride_coach.voice", v)}
              label="Read the coach aloud"
              hint={speechAvailable() ? "Uses your computer's built-in voice; nothing is sent anywhere. You can also switch this on the Ride screen." : "Text-to-speech isn't available in this window."}
            />
          </div>
        </>
      )}

      {tab === "map" && (
        <>
          <h2>Background map</h2>
          <Toggle checked={s.map.enabled} onChange={(v) => upd("map.enabled", v)} label="Show map tiles" hint="Off: routes are drawn as a line without a background, and no tile requests are made." />
          <div className="cols" style={{ marginTop: 10 }}>
            <Field label="Map style URL" hint="Any MapLibre style. Default: OpenFreeMap (free, no key).">
              <input type="url" value={s.map.style_url} onChange={(e) => upd("map.style_url", e.target.value)} />
            </Field>
            <Field label="Attribution">
              <input type="text" value={s.map.attribution} onChange={(e) => upd("map.attribution", e.target.value)} />
            </Field>
          </div>
          <h2 style={{ marginTop: 20 }}>Route builder</h2>
          <Toggle checked={s.providers.routing_enabled} onChange={(v) => upd("providers.routing_enabled", v)} label="Use a routing service to snap routes to roads" />
          <div className="cols" style={{ marginTop: 10 }}>
            <Field label="Valhalla server" hint="Default: the public FOSSGIS server (free, fair use). You can run your own.">
              <input type="url" value={s.providers.routing_url} onChange={(e) => upd("providers.routing_url", e.target.value)} />
            </Field>
          </div>
          <h2 style={{ marginTop: 20 }}>Elevation</h2>
          <Toggle checked={s.providers.elevation_enabled} onChange={(v) => upd("providers.elevation_enabled", v)} label="Fetch elevation for routes without it" />
          <div className="cols" style={{ marginTop: 10 }}>
            <Field label="Open-Meteo server" hint="Free for non-commercial use, no key; Copernicus DEM 90 m.">
              <input type="url" value={s.providers.elevation_url} onChange={(e) => upd("providers.elevation_url", e.target.value)} />
            </Field>
          </div>
          <p className="small muted" style={{ marginTop: 10 }}>
            Only the coordinates needed for the request are sent to these services. They are community-run; please use them gently.
          </p>
        </>
      )}

      <div className="form-foot">
        {dirty && (
          <Button kind="quiet" onClick={() => (setS(res.data.settings), setDirty(false))}>
            Discard changes
          </Button>
        )}
        <Button kind="primary" onClick={save} disabled={!dirty || !!busy || (tab === "trainer" && riding && JSON.stringify(t) !== JSON.stringify(res.data?.settings?.trainer))}>
          Save
        </Button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- privacy

function kb(b: number | undefined) {
  if (b == null) return "—";
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / 1024 / 1024).toFixed(1)} MB`;
}

function PrivacyTab() {
  const { refreshBoot, bumpData, toast, go } = useApp();
  const info = useRpc("storageInfo", {});
  const res = useRpc("getSettings", {});
  const { run, busy } = useAction();
  const [confirm, setConfirm] = useState<string | null>(null);
  const d: J = info.data;
  const diag = !!res.data?.settings?.diagnostics_opt_in;
  return (
    <>
      <section className="panel">
        <h2>Your data</h2>
        <p>Ridgeline has no account and no server. Your profile, rides, routes and plans are files on this computer.</p>
        {d && (
          <>
            <table className="table" style={{ maxWidth: 520 }}>
              <tbody>
                {Object.entries(d)
                  .filter(([k]) => k.endsWith("_bytes") && k !== "total_bytes")
                  .map(([k, v]) => (
                    <tr key={k}>
                      <td>{k.replace(/_bytes$/, "").replace(/_/g, " ")}</td>
                      <td className="num">{kb(v as number)}</td>
                    </tr>
                  ))}
                <tr>
                  <td>
                    <b>Total</b>
                  </td>
                  <td className="num">
                    <b>{kb(d.total_bytes)}</b>
                  </td>
                </tr>
              </tbody>
            </table>
            <p className="small">
              Location: <code>{d.data_dir}</code>
            </p>
            <p className="small muted">{d.note}</p>
          </>
        )}
        <div className="actions">
          <Button disabled={!!busy} onClick={() => run("export", async () => toast(`Exported to ${(await rpc("exportAllData", {})).path}`))}>
            Export all my data
          </Button>
        </div>
      </section>
      <section className="panel">
        <h2>Diagnostics</h2>
        <Toggle
          checked={diag}
          onChange={(v) => run("diag", () => rpc("saveSettings", { settings: { ...res.data.settings, diagnostics_opt_in: v } }).then(res.reload))}
          label="Allow diagnostic export"
          hint="Creates a file you can attach to a bug report: versions, device capabilities and the trainer command log. No keys, profile, routes, locations or coach messages. Nothing is uploaded automatically."
        />
        <div className="actions" style={{ marginTop: 10 }}>
          <Button disabled={!diag || !!busy} onClick={() => run("diagx", async () => toast(`Diagnostics saved: ${(await rpc("exportDiagnostics", {})).path}`))}>
            Export diagnostics
          </Button>
        </div>
      </section>
      <section className="panel">
        <h2>Delete everything</h2>
        <p>Removes your profile, rides, routes, plans, coach history, settings and any stored API key from this computer. Files you exported elsewhere are not touched.</p>
        <Button kind="danger" onClick={() => setConfirm("")}>
          Delete all data…
        </Button>
      </section>
      {confirm != null && (
        <Modal title="Delete all Ridgeline data?" onClose={() => setConfirm(null)}>
          <p>
            This can't be undone. Consider <b>Export all my data</b> first. Type <b>DELETE</b> to confirm.
          </p>
          <input type="text" value={confirm} onChange={(e) => setConfirm(e.target.value)} aria-label="Type DELETE to confirm" />
          <div className="actions" style={{ marginTop: 12 }}>
            <Button
              kind="danger"
              disabled={confirm !== "DELETE" || !!busy}
              onClick={() =>
                run("delall", async () => {
                  await rpc("deleteAllData", { confirm: "DELETE" });
                  setConfirm(null);
                  await refreshBoot();
                  bumpData();
                  go("home");
                  toast("All data deleted.");
                })
              }
            >
              Delete everything
            </Button>
            <Button onClick={() => setConfirm(null)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </>
  );
}

// ---------------------------------------------------------------- about

function AboutTab() {
  const about = useRpc("getAbout", {});
  const a: J = about.data;
  if (!a) return <Spinner />;
  return (
    <section className="panel">
      <h2>Ridgeline {a.version}</h2>
      <p className="small muted">
        {a.platform} · coaching policy {a.policy_version}
      </p>
      <p>Free, open-source indoor cycling with a coach that explains itself. Training suggestions are not medical advice.</p>
      <p className="small">{a.updates}</p>
      <h3>Open data and software</h3>
      <ul className="small">
        {(a.attributions as string[]).map((t) => (
          <li key={t}>{t}</li>
        ))}
        <li>Fonts: Barlow and Barlow Condensed (SIL Open Font License 1.1)</li>
        <li>UI: React (MIT)</li>
      </ul>
      <p className="small muted">Full licence texts are in THIRD_PARTY_NOTICES.md in the source repository and installer.</p>
    </section>
  );
}
