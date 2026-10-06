// Home: today's proposed ride, readiness check, quick starts, device status.
import { useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Button, Empty, ErrorText, Modal, Scale, Spinner, Stat, Status, Toggle } from "../ui";
import { ProposalModal, SessionSummary } from "../plan";
import { dateLabel, dateTime, dist, minutes, round } from "../format";
import { useAction } from "../hooks";

export function Home() {
  const { go, boot, live, dataVersion, units } = useApp();
  const plan = useRpc("getPlan", {}, [dataVersion]);
  const acts = useRpc("listActivities", {}, [dataVersion]);
  const [readiness, setReadiness] = useState<J | null>(null);
  const [proposal, setProposal] = useState<string | null>(null);
  const today: string = plan.data?.today ?? boot?.today;
  const sessions: J[] = plan.data?.plan?.sessions ?? [];
  const todays = sessions.find((s) => s.date === today && s.display_status === "planned");
  const doneToday = sessions.find((s) => s.date === today && s.display_status === "completed");
  const nextUp = useMemo(() => sessions.filter((s) => s.date > today && s.display_status === "planned").sort((a, b) => (a.date < b.date ? -1 : 1))[0], [sessions, today]);
  const pending: J[] = plan.data?.pending ?? [];
  const last: J | undefined = (acts.data ?? []).filter((a: J) => a.meta.status !== "recording")[0];
  const name = boot?.profile?.display_name;
  const hour = new Date().getHours();
  const greet = hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  const devs: J[] = live?.devices?.devices ?? [];
  const trainer = devs.find((d) => d.is_trainer);

  return (
    <div>
      <div className="page-head">
        <div>
          <h1>
            {greet}
            {name ? `, ${name}` : ""}
          </h1>
          <p className="sub">{today && dateLabel(today)}</p>
        </div>
        <div className="actions">
          {live?.ride_active && (
            <Button kind="primary" big onClick={() => go("ride")}>
              Back to the ride
            </Button>
          )}
        </div>
      </div>

      {live?.demo && (
        <div className="banner banner-warn" style={{ marginBottom: 18 }}>
          <p>
            <b>Demo mode.</b> Devices are simulated and rides are labelled as demos. Connect your own trainer on the Devices screen, then turn demo mode off in Settings.
          </p>
          <Button onClick={() => go("devices")}>Devices</Button>
        </div>
      )}

      {pending.length > 0 && (
        <div className="banner banner-info" style={{ marginBottom: 18 }}>
          <p>
            The coach has {pending.length} plan {pending.length === 1 ? "change" : "changes"} for you to review. Nothing changes until you accept.
          </p>
          <Button onClick={() => setProposal(pending[0].id)}>Review</Button>
        </div>
      )}

      <div className="grid-2">
        <section className="panel">
          {plan.loading && <Spinner />}
          {!plan.loading && !plan.data?.plan && (
            <Empty
              title="No training plan yet"
              action={
                <div className="actions">
                  <Button kind="primary" onClick={() => go("coach")}>
                    Create my plan
                  </Button>
                  <Button onClick={() => go("workouts")}>Pick a workout</Button>
                </div>
              }
            >
              The coach builds a four-week plan around the days and time you have. You review it before anything is scheduled.
            </Empty>
          )}
          {plan.data?.plan && todays && (
            <div className="today">
              <div>
                <p className="tag tag-sign">Today</p>
                <h2>{todays.workout_name}</h2>
                <SessionSummary s={todays} />
              </div>
              <div className="actions" style={{ flexDirection: "column", alignItems: "stretch" }}>
                <Button kind="primary" big onClick={() => setReadiness(todays)}>
                  Check in & ride
                </Button>
                <Button onClick={() => go("ride", { mode: "erg", workout_id: todays.workout_id, plan_session_id: todays.id })}>Skip check-in</Button>
                <Button kind="quiet" onClick={() => go("calendar")}>
                  Change in calendar
                </Button>
              </div>
            </div>
          )}
          {plan.data?.plan && !todays && (
            <div>
              <p className="tag">{doneToday ? "Done today" : "Today"}</p>
              <h2>{doneToday ? `Completed: ${doneToday.workout_name}` : "Rest day"}</h2>
              <p className="muted">{doneToday ? "Nice work. Recovery is part of training." : "No ride is planned today. Easy spinning is fine if you feel like it."}</p>
              {nextUp && (
                <p>
                  Next: <b>{nextUp.workout_name}</b> on {dateLabel(nextUp.date)} ({minutes(nextUp.duration_s)}).
                </p>
              )}
              <div className="actions">
                <Button onClick={() => go("calendar")}>Open calendar</Button>
                <Button kind="quiet" onClick={() => go("workouts")}>
                  Ride something else
                </Button>
              </div>
            </div>
          )}
        </section>

        <section className="panel">
          <div className="panel-head">
            <h3>Your setup</h3>
            <Button kind="quiet" onClick={() => go("devices")}>
              Devices
            </Button>
          </div>
          {devs.filter((d) => d.state !== "disconnected" || d.want_connected).length === 0 && <p className="muted">No devices connected. Wake your trainer by pedalling, then scan on the Devices screen.</p>}
          <div className="stack">
            {trainer && (
              <Status kind={trainer.state === "ready" ? "ok" : "warn"}>
                Trainer: {trainer.name} — {trainer.state}
                {trainer.caps && ` (${[trainer.caps.erg && "ERG", trainer.caps.simulation && "road sim", trainer.caps.resistance && "level"].filter(Boolean).join(", ")})`}
              </Status>
            )}
            {devs
              .filter((d) => !d.is_trainer && (d.state === "ready" || d.want_connected))
              .map((d) => (
                <Status key={d.key} kind={d.state === "ready" ? "ok" : "warn"}>
                  {d.name} — {d.state}
                  {d.battery != null && ` · battery ${d.battery}%`}
                </Status>
              ))}
          </div>
          <div className="launch" style={{ marginTop: 16 }}>
            <Button kind="primary" onClick={() => go("workouts")}>
              Start a workout
            </Button>
            <Button onClick={() => go("routes")}>Free ride a route</Button>
          </div>
        </section>
      </div>

      <section className="panel">
        <div className="panel-head">
          <h3>Last ride</h3>
          <Button kind="quiet" onClick={() => go("history")}>
            History
          </Button>
        </div>
        {!last && <p className="muted">No rides yet.</p>}
        {last && (
          <div>
            <p>
              <b>{last.meta.title}</b> · {dateTime(last.meta.start_utc)}
            </p>
            <div className="metric-row">
              <Stat label="Time" value={minutes(last.summary?.timer_s)} />
              <Stat label="Distance" value={dist(last.summary?.distance_m, units)} />
              <Stat label="Avg power" value={round(last.summary?.power?.avg)} unit="W" />
              <Stat label="Avg HR" value={round(last.summary?.hr?.avg)} unit="bpm" />
            </div>
          </div>
        )}
      </section>

      {readiness && <ReadinessModal session={readiness} onClose={() => setReadiness(null)} onProposal={(id) => setProposal(id)} />}
      {proposal && <ProposalModal id={proposal} onClose={() => setProposal(null)} />}
    </div>
  );
}

export function ReadinessModal({ session, onClose, onProposal }: { session: J; onClose: () => void; onProposal: (id: string) => void }) {
  const { go } = useApp();
  const { run, busy } = useAction();
  const [r, setR] = useState({ sleep: 3, fatigue: 2, soreness: 1, stress: 2, illness: false, pain_or_injury: false, warning_symptoms: false });
  const [result, setResult] = useState<J | null>(null);
  const set = (k: string, v: unknown) => setR((x) => ({ ...x, [k]: v }));
  const start = () => {
    onClose();
    go("ride", { mode: "erg", workout_id: session.workout_id, plan_session_id: session.id });
  };
  return (
    <Modal title="Quick check-in" onClose={onClose}>
      {!result && (
        <div className="stack">
          <p className="muted">How are you today? This takes ten seconds and can suggest a lighter ride.</p>
          <div className="field">
            <span className="field-label">Sleep last night</span>
            <Scale name="Sleep" value={r.sleep} min={1} max={5} onChange={(v) => set("sleep", v)} labels={["poor", "great"]} />
          </div>
          <div className="field">
            <span className="field-label">Fatigue</span>
            <Scale name="Fatigue" value={r.fatigue} min={1} max={5} onChange={(v) => set("fatigue", v)} labels={["fresh", "exhausted"]} />
          </div>
          <div className="field">
            <span className="field-label">Muscle soreness</span>
            <Scale name="Soreness" value={r.soreness} min={1} max={5} onChange={(v) => set("soreness", v)} labels={["none", "very sore"]} />
          </div>
          <div className="field">
            <span className="field-label">Stress</span>
            <Scale name="Stress" value={r.stress} min={1} max={5} onChange={(v) => set("stress", v)} labels={["calm", "very stressed"]} />
          </div>
          <Toggle checked={r.illness} onChange={(v) => set("illness", v)} label="I feel unwell (cold, fever, stomach bug)" />
          <Toggle checked={r.pain_or_injury} onChange={(v) => set("pain_or_injury", v)} label="I have pain or an injury" />
          <Toggle
            checked={r.warning_symptoms}
            onChange={(v) => set("warning_symptoms", v)}
            label="Recently: chest pain or pressure, fainting, unusual breathlessness, palpitations or dizziness"
          />
          <div className="actions">
            <Button
              kind="primary"
              disabled={!!busy}
              onClick={() =>
                run("check", async () => {
                  setResult(await rpc("readinessCheck", { readiness: r }));
                })
              }
            >
              Check
            </Button>
            <Button kind="quiet" onClick={start}>
              Skip
            </Button>
          </div>
        </div>
      )}
      {result && (
        <div className="stack">
          <div className={`banner ${result.safety ? "banner-bad" : result.advice === "proceed" ? "banner-info" : "banner-warn"}`}>
            <p>{result.message}</p>
          </div>
          <ErrorText>{null}</ErrorText>
          <div className="actions">
            {result.proposal_id && (
              <Button
                kind="primary"
                onClick={() => {
                  onClose();
                  onProposal(result.proposal_id);
                }}
              >
                Review suggested change
              </Button>
            )}
            {!result.safety && (
              <Button kind={result.proposal_id ? "secondary" : "primary"} onClick={start}>
                {result.advice === "proceed" ? "Start the ride" : "Ride as planned anyway"}
              </Button>
            )}
            <Button kind="quiet" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
