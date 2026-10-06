// Onboarding interview: short steps with progress, saved as a draft after
// every step so it can be resumed (spec §3, §4).
import { useEffect, useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { useApp } from "../state";
import { Button, ErrorText, Field, NumberInput, Segmented, Toggle } from "../ui";
import { mass, massToKg, massUnit, WEEKDAYS, type Units } from "../format";

export const CATEGORIES: { id: string; label: string }[] = [
  { id: "recovery", label: "Recovery" },
  { id: "endurance", label: "Endurance" },
  { id: "tempo", label: "Tempo" },
  { id: "sweet_spot", label: "Sweet spot" },
  { id: "threshold", label: "Threshold" },
  { id: "over_under", label: "Over-unders" },
  { id: "vo2", label: "VO2-style intervals" },
  { id: "cadence", label: "Cadence drills" },
  { id: "climbing", label: "Climbing preparation" },
  { id: "anaerobic", label: "Anaerobic / sprint" },
  { id: "assessment", label: "Assessments" },
];

export const GOALS = [
  { v: "general_fitness", t: "General fitness", d: "Ride regularly and feel fitter." },
  { v: "endurance", t: "Endurance", d: "Ride longer, more comfortably." },
  { v: "climbing", t: "Climbing", d: "Get better on hills." },
  { v: "event", t: "An event", d: "Prepare for a date on the calendar." },
  { v: "ftp_improvement", t: "Raise my FTP", d: "Increase sustainable power." },
  { v: "returning", t: "Returning to riding", d: "Come back after a break, gently." },
  { v: "maintain", t: "Maintain fitness", d: "Keep what I have with less time." },
];

const STEPS = ["Goal", "History", "Your week", "Fitness", "Body & units", "Preferences", "AI coach", "Review"];
export const MINUTE_OPTIONS = [0, 30, 45, 60, 75, 90, 120, 150, 180, 240];

export function blankProfile(): J {
  return {
    display_name: "",
    goal: "general_fitness",
    event_date: null,
    event_name: "",
    experience: "some",
    recent_weekly_min: 120,
    consistent_weeks: 4,
    weeks_off: 0,
    availability_min: [0, 60, 0, 60, 0, 90, 60],
    long_ride_day: 5,
    competing_activities: "",
    rider_mass_kg: 75,
    bike_mass_kg: 9,
    units: "metric",
    age_range: "",
    max_hr: null,
    threshold_hr: null,
    power_source: "measured",
    preferred_categories: [],
    avoided_categories: [],
    limitations: "",
    ai_consent: { enabled: false, share_activity_summaries: false, share_limitations: false, updated_utc: 0 },
    trainer_difficulty_pct: 100,
  };
}

export function Onboarding() {
  const { boot, refreshBoot, go, toast } = useApp();
  const draft: J = boot?.onboarding_draft;
  const [step, setStep] = useState<number>(draft?.step ?? 0);
  const [p, setP] = useState<J>(draft?.profile ?? blankProfile());
  const [ftp, setFtp] = useState<J>(draft?.ftp ?? { known: false, watts: null, method: "known", effective: boot?.today, confidence: "medium" });
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const u: Units = p.units;
  const set = (k: string, v: unknown) => setP((x: J) => ({ ...x, [k]: v }));

  useEffect(() => {
    // Save/resume: persist the draft whenever the step changes.
    rpc("saveOnboardingDraft", { draft: { step, profile: p, ftp } }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  const check = (): string | null => {
    if (step === 0 && p.goal === "event" && !p.event_date) return "Choose the event date, or pick another goal.";
    if (step === 2) {
      if (p.availability_min.every((m: number) => m === 0)) return "Choose at least one day you can ride.";
      if (p.long_ride_day != null && p.availability_min[p.long_ride_day] === 0) return "Your long-ride day must be a day you can ride.";
    }
    if (step === 3 && ftp.known && (ftp.watts == null || ftp.watts < 40 || ftp.watts > 600)) return "Enter an FTP between 40 and 600 W, or choose “I don't know my FTP”.";
    if (step === 4) {
      if (!(p.rider_mass_kg >= 25 && p.rider_mass_kg <= 250)) return "Rider mass must be between 25 and 250 kg.";
      if (!(p.bike_mass_kg >= 3 && p.bike_mass_kg <= 40)) return "Bike mass must be between 3 and 40 kg.";
    }
    return null;
  };
  const next = () => {
    const e = check();
    setError(e);
    if (!e) setStep((s) => Math.min(STEPS.length - 1, s + 1));
  };
  const finish = async () => {
    setSaving(true);
    setError(null);
    try {
      await rpc("completeOnboarding", { profile: p, ftp: ftp.known ? { watts: ftp.watts, method: ftp.method, effective: ftp.effective, confidence: ftp.confidence } : null });
      await refreshBoot();
      toast("Profile saved. Next: connect your trainer, then ask the coach for a plan.");
      go("devices");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const toggleCat = (key: "preferred_categories" | "avoided_categories", id: string) => {
    const other = key === "preferred_categories" ? "avoided_categories" : "preferred_categories";
    setP((x: J) => {
      const has = x[key].includes(id);
      return { ...x, [key]: has ? x[key].filter((c: string) => c !== id) : [...x[key], id], [other]: x[other].filter((c: string) => c !== id) };
    });
  };

  const weekly = useMemo(() => p.availability_min.reduce((a: number, b: number) => a + b, 0), [p.availability_min]);

  return (
    <main className="content">
      <div className="onb">
        <div className="page-head">
          <div>
            <h1>Set up Ridgeline</h1>
            <p className="sub">
              Step {step + 1} of {STEPS.length}: {STEPS[step]}. Your answers are saved as you go and stay on this computer.
            </p>
          </div>
          <Button kind="quiet" onClick={() => go("home")}>
            Back to start
          </Button>
        </div>
        <div className="progress" aria-hidden="true">
          {STEPS.map((s, i) => (
            <span key={s} className={i <= step ? "done" : ""} />
          ))}
        </div>

        <div className="panel">
          {step === 0 && (
            <>
              <h2>What's your main goal?</h2>
              <div className="choice-grid" role="radiogroup" aria-label="Goal">
                {GOALS.map((g) => (
                  <button key={g.v} className="choice" role="radio" aria-checked={p.goal === g.v} onClick={() => set("goal", g.v)}>
                    <b>{g.t}</b>
                    <span>{g.d}</span>
                  </button>
                ))}
              </div>
              {p.goal === "event" && (
                <div className="cols" style={{ marginTop: 16 }}>
                  <Field label="Event date">
                    <input type="date" value={p.event_date ?? ""} min={boot?.today} onChange={(e) => set("event_date", e.target.value || null)} />
                  </Field>
                  <Field label="Event name (optional)">
                    <input type="text" value={p.event_name} maxLength={120} onChange={(e) => set("event_name", e.target.value)} />
                  </Field>
                </div>
              )}
              <div className="cols" style={{ marginTop: 16 }}>
                <Field label="What should we call you? (optional)">
                  <input type="text" value={p.display_name} maxLength={60} onChange={(e) => set("display_name", e.target.value)} />
                </Field>
              </div>
            </>
          )}

          {step === 1 && (
            <>
              <h2>Your riding so far</h2>
              <div className="cols">
                <Field label="Experience with structured training">
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
                <Field label="Riding per week lately" hint="Average over the last 4 weeks, indoors and outdoors.">
                  <NumberInput value={p.recent_weekly_min} min={0} max={2400} step={15} suffix="minutes" onChange={(v) => set("recent_weekly_min", v ?? 0)} />
                </Field>
                <Field label="Weeks you rode consistently" hint="Out of the last 12 weeks.">
                  <NumberInput value={p.consistent_weeks} min={0} max={12} onChange={(v) => set("consistent_weeks", Math.min(12, v ?? 0))} />
                </Field>
                <Field label="Weeks since you last trained regularly" hint="0 if you're riding now.">
                  <NumberInput value={p.weeks_off} min={0} max={520} onChange={(v) => set("weeks_off", v ?? 0)} />
                </Field>
              </div>
            </>
          )}

          {step === 2 && (
            <>
              <h2>Your week</h2>
              <p className="muted">How much time can you ride on each day? Choose 0 for rest days. The plan never stacks missed rides onto other days.</p>
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
              <p className="small muted" style={{ marginTop: 8 }}>
                Up to {Math.round(weekly / 6) / 10} hours available per week.
              </p>
              <div className="cols" style={{ marginTop: 12 }}>
                <Field label="Preferred long-ride day">
                  <select value={p.long_ride_day ?? ""} onChange={(e) => set("long_ride_day", e.target.value === "" ? null : Number(e.target.value))}>
                    <option value="">No preference</option>
                    {WEEKDAYS.map((d, i) => (
                      <option key={d} value={i} disabled={p.availability_min[i] === 0}>
                        {d}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Other sports or demanding activities (optional)" hint="e.g. running Tuesdays, physical job.">
                  <input type="text" value={p.competing_activities} maxLength={300} onChange={(e) => set("competing_activities", e.target.value)} />
                </Field>
              </div>
            </>
          )}

          {step === 3 && (
            <>
              <h2>Fitness markers</h2>
              <p className="muted">
                FTP (functional threshold power) sets your workout targets. It's fine not to know it: easy riding works without it, and an optional assessment is available later. Ridgeline
                never estimates FTP from age.
              </p>
              <Segmented
                value={ftp.known ? "yes" : "no"}
                onChange={(v) => setFtp({ ...ftp, known: v === "yes" })}
                options={[
                  { value: "yes", label: "I know my FTP" },
                  { value: "no", label: "I don't know my FTP" },
                ]}
              />
              {ftp.known && (
                <div className="cols" style={{ marginTop: 14 }}>
                  <Field label="FTP">
                    <NumberInput value={ftp.watts} min={40} max={600} suffix="W" onChange={(v) => setFtp({ ...ftp, watts: v })} />
                  </Field>
                  <Field label="How was it measured?">
                    <select value={ftp.method} onChange={(e) => setFtp({ ...ftp, method: e.target.value })}>
                      <option value="known">I know it from another app or coach</option>
                      <option value="ramp_test">Ramp test</option>
                      <option value="twenty_minute_test">20-minute test</option>
                      <option value="manual">Rough guess</option>
                    </select>
                  </Field>
                  <Field label="When?">
                    <input type="date" value={ftp.effective ?? ""} max={boot?.today} onChange={(e) => setFtp({ ...ftp, effective: e.target.value })} />
                  </Field>
                  <Field label="How confident are you?">
                    <Segmented
                      value={ftp.confidence}
                      onChange={(v) => setFtp({ ...ftp, confidence: v })}
                      options={[
                        { value: "low", label: "Low" },
                        { value: "medium", label: "Medium" },
                        { value: "high", label: "High" },
                      ]}
                    />
                  </Field>
                </div>
              )}
              <div className="cols" style={{ marginTop: 16 }}>
                <Field label="Max heart rate (optional)" hint="Only if measured; never guessed from age.">
                  <NumberInput value={p.max_hr} min={100} max={230} suffix="bpm" onChange={(v) => set("max_hr", v)} />
                </Field>
                <Field label="Threshold heart rate (optional)">
                  <NumberInput value={p.threshold_hr} min={80} max={220} suffix="bpm" onChange={(v) => set("threshold_hr", v)} />
                </Field>
                <Field label="Power on your setup">
                  <select value={p.power_source} onChange={(e) => set("power_source", e.target.value)}>
                    <option value="measured">Measured (smart trainer or power meter)</option>
                    <option value="estimated">Estimated by the trainer</option>
                    <option value="none">No power data</option>
                  </select>
                </Field>
              </div>
            </>
          )}

          {step === 4 && (
            <>
              <h2>Body, bike and units</h2>
              <p className="muted">Masses are used to simulate climbing on routes. They're stored on this computer only.</p>
              <div className="cols">
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
                <Field label="Age range (optional)">
                  <select value={p.age_range} onChange={(e) => set("age_range", e.target.value)}>
                    <option value="">Prefer not to say</option>
                    {["under 18", "18-29", "30-39", "40-49", "50-59", "60-69", "70+"].map((a) => (
                      <option key={a} value={a}>
                        {a}
                      </option>
                    ))}
                  </select>
                </Field>
              </div>
              {p.age_range === "under 18" && <p className="small muted">Riders under 18 should train with a parent or guardian's knowledge.</p>}
            </>
          )}

          {step === 5 && (
            <>
              <h2>Preferences</h2>
              <p className="muted">Tap once for “more of this”, again for “avoid”, again to clear.</p>
              <div className="choice-grid">
                {CATEGORIES.map((c) => {
                  const pref = p.preferred_categories.includes(c.id);
                  const avoid = p.avoided_categories.includes(c.id);
                  return (
                    <button
                      key={c.id}
                      className="choice"
                      aria-pressed={pref || avoid}
                      onClick={() => (pref ? toggleCat("avoided_categories", c.id) : avoid ? toggleCat("avoided_categories", c.id) : toggleCat("preferred_categories", c.id))}
                    >
                      <b>{c.label}</b>
                      <span>{pref ? "✓ More of this" : avoid ? "✕ Avoid" : "No preference"}</span>
                    </button>
                  );
                })}
              </div>
              <div className="cols" style={{ marginTop: 16 }}>
                <Field label="Injuries or limitations you want the plan to respect (optional)" hint="Private. Shared with an AI service only if you allow it on the next step." wide>
                  <textarea value={p.limitations} maxLength={1000} onChange={(e) => set("limitations", e.target.value)} />
                </Field>
                <Field label="Trainer difficulty on routes" hint="100% feels the road's full gradient. Lower values ease climbs on the trainer only; distance and climbing stay real.">
                  <NumberInput value={p.trainer_difficulty_pct} min={0} max={100} step={5} suffix="%" onChange={(v) => set("trainer_difficulty_pct", Math.max(0, Math.min(100, v ?? 100)))} />
                </Field>
              </div>
            </>
          )}

          {step === 6 && (
            <>
              <h2>The AI coach</h2>
              <p>
                Ridgeline always has an <b>offline coach</b>: a rules-based planner reviewed against a coaching policy. You can also connect a <b>free AI model running on your own computer</b>{" "}
                (for example Ollama) in Settings. The AI only explains and customises within the policy's limits; it can never set trainer resistance.
              </p>
              <Toggle
                checked={p.ai_consent.enabled}
                onChange={(v) => set("ai_consent", { ...p.ai_consent, enabled: v })}
                label="Allow Ridgeline to send a compact summary of my profile and plan to the AI service I configure"
                hint="Off by default. No second-by-second data, no locations, no names of routes."
              />
              <div style={{ marginTop: 12, display: "grid", gap: 12, opacity: p.ai_consent.enabled ? 1 : 0.5 }}>
                <Toggle
                  checked={p.ai_consent.share_activity_summaries}
                  onChange={(v) => set("ai_consent", { ...p.ai_consent, share_activity_summaries: v })}
                  label="Include short summaries of recent rides (duration, average power, how hard it felt)"
                />
                <Toggle checked={p.ai_consent.share_limitations} onChange={(v) => set("ai_consent", { ...p.ai_consent, share_limitations: v })} label="Include the limitations I described" />
              </div>
              <p className="small muted" style={{ marginTop: 12 }}>
                You can change this any time in Settings → AI coach. Remote (cloud) AI services are disabled by default because some charge for use.
              </p>
            </>
          )}

          {step === 7 && (
            <>
              <h2>Review</h2>
              <dl className="review">
                <dt>Goal</dt>
                <dd>
                  {GOALS.find((g) => g.v === p.goal)?.t}
                  {p.goal === "event" && p.event_date && ` on ${p.event_date}`}
                </dd>
                <dt>Experience</dt>
                <dd>
                  {p.experience}, {p.recent_weekly_min} min/week lately, consistent {p.consistent_weeks}/12 weeks
                  {p.weeks_off > 0 && `, ${p.weeks_off} weeks off`}
                </dd>
                <dt>Week</dt>
                <dd>{WEEKDAYS.map((d, i) => (p.availability_min[i] ? `${d} ${p.availability_min[i]}′` : null)).filter(Boolean).join(" · ")}</dd>
                <dt>FTP</dt>
                <dd>{ftp.known ? `${ftp.watts} W (${ftp.confidence} confidence)` : "Not set — easy rides and perceived-effort guidance work without it"}</dd>
                <dt>Mass</dt>
                <dd>
                  {mass(p.rider_mass_kg, u)} + {mass(p.bike_mass_kg, u)} {massUnit(u)}
                </dd>
                <dt>AI</dt>
                <dd>{p.ai_consent.enabled ? "Allowed (summary only)" : "Not allowed — offline coach only"}</dd>
              </dl>
              <p className="small muted">These are coaching rules and suggestions, not medical advice. If you have health concerns, check with a doctor before starting a training plan.</p>
            </>
          )}

          <ErrorText>{error}</ErrorText>
          <div className="form-foot">
            <Button onClick={() => (step === 0 ? go("home") : setStep((s) => s - 1))}>{step === 0 ? "Cancel" : "Back"}</Button>
            {step < STEPS.length - 1 ? (
              <Button kind="primary" onClick={next}>
                Continue
              </Button>
            ) : (
              <Button kind="primary" onClick={finish} disabled={saving}>
                {saving ? "Saving…" : "Save and continue"}
              </Button>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}
