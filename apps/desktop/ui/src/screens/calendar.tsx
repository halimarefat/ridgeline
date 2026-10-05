// Calendar: the accepted plan by week, with per-session edits. Every edit is
// a proposal that is validated against the coaching policy and only applied
// after the rider accepts it; accepted changes can be undone.
import { useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Button, Empty, ErrorText, Field, Modal, NumberInput, Spinner } from "../ui";
import { IntensityTag, ProposalModal, SessionSummary, SourceTag } from "../plan";
import { addDays, dateLabel, dateTime, minutes, WEEKDAYS, weekday } from "../format";
import { useAction } from "../hooks";

const STATUS_LABEL: Record<string, string> = { planned: "Planned", completed: "Done", skipped: "Skipped", missed: "Missed" };

export function Calendar() {
  const { go, dataVersion, bumpData, toast } = useApp();
  const { data, loading, error } = useRpc("getPlan", {}, [dataVersion]);
  const [open, setOpen] = useState<J | null>(null);
  const [proposal, setProposal] = useState<string | null>(null);
  const [confirmUndo, setConfirmUndo] = useState(false);
  const { run, busy } = useAction();
  const plan: J = data?.plan;
  const today: string = data?.today ?? "";
  const pending: J[] = data?.pending ?? [];

  const weeks = useMemo(() => {
    if (!plan) return [] as { start: string; days: string[]; summary: J }[];
    // Rows start on the Monday on or before the plan start so weekdays line up.
    const first = addDays(plan.start, -weekday(plan.start));
    const end = addDays(plan.start, 7 * plan.weeks);
    const rows: { start: string; days: string[]; summary: J }[] = [];
    for (let d = first; d < end; d = addDays(d, 7)) {
      const days = Array.from({ length: 7 }, (_, i) => addDays(d, i));
      const summary = (plan.weeks_summary ?? []).find((w: J) => w.start >= d && w.start < addDays(d, 7)) ?? (plan.weeks_summary ?? []).find((w: J) => days.includes(w.start));
      rows.push({ start: d, days, summary });
    }
    return rows;
  }, [plan]);

  const byDate = useMemo(() => {
    const m = new Map<string, J[]>();
    for (const s of plan?.sessions ?? []) m.set(s.date, [...(m.get(s.date) ?? []), s]);
    return m;
  }, [plan]);

  const inPlan = (d: string) => plan && d >= plan.start && d < addDays(plan.start, 7 * plan.weeks);

  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Calendar</h1>
          <p className="sub">{plan ? `${plan.weeks}-week plan from ${dateLabel(plan.start)} · version ${plan.version}` : "Your training plan"}</p>
        </div>
        <div className="actions">
          {data?.can_undo && (
            <Button onClick={() => setConfirmUndo(true)} disabled={!!busy}>
              Undo last change
            </Button>
          )}
          {plan && (
            <Button
              disabled={!!busy}
              onClick={() =>
                run("adapt", async () => {
                  const r = await rpc("adaptPlan", {});
                  bumpData();
                  if (r.proposal_id) setProposal(r.proposal_id);
                  else toast(r.message);
                })
              }
            >
              Check for adjustments
            </Button>
          )}
          <Button kind="primary" onClick={() => go("coach")}>
            {plan ? "New plan" : "Create a plan"}
          </Button>
        </div>
      </div>
      {loading && !data && <Spinner />}
      <ErrorText>{error}</ErrorText>

      {pending.length > 0 && (
        <section className="panel">
          <h3>Waiting for your decision</h3>
          <div className="list">
            {pending.map((p) => (
              <div key={p.id} className="list-item" role="button" tabIndex={0} onClick={() => setProposal(p.id)} onKeyDown={(e) => e.key === "Enter" && setProposal(p.id)}>
                <div>
                  <div className="li-title">{p.explanation?.slice(0, 140) || "Plan change"}</div>
                  <div className="li-meta">
                    <span>{dateTime(p.created_utc)}</span>
                    <span>{(p.changes ?? []).length || "new"} change(s)</span>
                  </div>
                </div>
                <SourceTag source={p.source} provider={p.provider} model={p.model} />
              </div>
            ))}
          </div>
        </section>
      )}

      {data && !plan && (
        <section className="panel">
          <Empty title="No plan yet" action={<Button kind="primary" onClick={() => go("coach")}>Ask the coach for a plan</Button>}>
            Your plan appears here once you accept a proposal. You can still ride any workout or route without a plan.
          </Empty>
        </section>
      )}

      {plan && (
        <section className="panel">
          <div className="panel-head">
            <div className="actions">
              <SourceTag source={plan.source} provider={plan.provider} model={plan.model} />
              <span className="tag">level: {plan.level}</span>
            </div>
          </div>
          {plan.rationale && <p className="muted">{plan.rationale}</p>}
          <div className="cal-scroll">
            <div className="cal-head" aria-hidden="true">
              <span />
              {WEEKDAYS.map((d) => (
                <span key={d}>{d}</span>
              ))}
            </div>
            {weeks.map((w, wi) => (
              <div className="week" key={w.start}>
                <div className="week-label">
                  <b>Week {wi + 1}</b>
                  {w.summary && (
                    <>
                      {minutes(w.summary.minutes * 60)} · {w.summary.sessions} ride(s)
                      {w.summary.recovery_week && <div className="small">Recovery week</div>}
                    </>
                  )}
                </div>
                {w.days.map((d) => {
                  const ss = byDate.get(d) ?? [];
                  const cls = ["cal-day", ss.length ? "" : "rest", d === today ? "today" : "", inPlan(d) ? "" : "outside"].join(" ");
                  return (
                    <div className={cls} key={d}>
                      <span className="cal-date">
                        {d.slice(8)}
                        {d === today && " · today"}
                      </span>
                      {ss.map((s) => (
                        <button key={s.id} className={`cal-session ${s.display_status}`} onClick={() => setOpen(s)} aria-label={`${s.workout_name}, ${dateLabel(s.date)}, ${STATUS_LABEL[s.display_status]}`}>
                          <b>{s.workout_name ?? s.workout_id}</b>
                          <span className="small muted">
                            {minutes((s.duration_s * (s.keep_pct ?? 100)) / 100)} · {STATUS_LABEL[s.display_status]}
                          </span>
                          <IntensityTag v={s.intensity} />
                        </button>
                      ))}
                      {!ss.length && inPlan(d) && <span className="small">Rest</span>}
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </section>
      )}

      {(data?.log ?? []).length > 0 && (
        <section className="panel">
          <h3>Plan history</h3>
          <div className="log">
            {(data.log as J[]).map((e, i) => (
              <div key={i}>
                {dateTime(e.utc)} — {e.action === "accept" ? "Accepted" : e.action === "reject" ? "Rejected" : e.action === "undo" ? "Undid a change" : e.action}
                {e.explanation ? `: ${String(e.explanation).slice(0, 160)}` : ""}
              </div>
            ))}
          </div>
        </section>
      )}

      {open && (
        <SessionDialog
          s={open}
          today={today}
          plan={plan}
          onClose={() => setOpen(null)}
          onProposal={(id) => {
            setOpen(null);
            setProposal(id);
          }}
        />
      )}
      {proposal && <ProposalModal id={proposal} onClose={() => (setProposal(null), bumpData())} />}
      {confirmUndo && (
        <Modal title="Undo the last accepted change?" onClose={() => setConfirmUndo(false)}>
          <p>Your plan goes back to the version before the last change you accepted. Completed rides are not affected.</p>
          <div className="actions">
            <Button
              kind="primary"
              onClick={() =>
                run(
                  "undo",
                  async () => {
                    await rpc("undoPlan", {});
                    setConfirmUndo(false);
                    bumpData();
                  },
                  "Change undone.",
                )
              }
            >
              Undo
            </Button>
            <Button onClick={() => setConfirmUndo(false)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </div>
  );
}

type Edit = "move" | "replace" | "shorten" | null;

function SessionDialog({ s, today, plan, onClose, onProposal }: { s: J; today: string; plan: J; onClose: () => void; onProposal: (id: string) => void }) {
  const { go } = useApp();
  const workouts = useRpc("listWorkouts", {});
  const { run, busy } = useAction();
  const [edit, setEdit] = useState<Edit>(null);
  const [date, setDate] = useState<string>(s.date < today ? today : s.date);
  const [wid, setWid] = useState<string>(s.alternatives?.[0] ?? "");
  const [keep, setKeep] = useState<number | null>(70);
  const [reason, setReason] = useState("");
  const planned = s.display_status === "planned";
  const editable = planned || s.display_status === "missed";
  const end = addDays(plan.start, 7 * plan.weeks - 1);
  const propose = (change: J) =>
    run("preview", async () => {
      const r = await rpc("previewChange", { changes: [{ session_id: s.id, reason, ...change }], reason: reason || undefined });
      if (r.proposal_id) onProposal(r.proposal_id);
    });
  const alts: string[] = s.alternatives ?? [];
  const list: J[] = workouts.data?.workouts ?? [];
  return (
    <Modal title={`${dateLabel(s.date)} · ${s.workout_name ?? s.workout_id}`} onClose={onClose} wide>
      <SessionSummary s={s} />
      <p className="small muted">Status: {STATUS_LABEL[s.display_status]}</p>
      <div className="actions" style={{ marginTop: 12 }}>
        {s.display_status === "completed" && s.completed_activity && (
          <Button kind="primary" onClick={() => (onClose(), go("history", { id: s.completed_activity }))}>
            Open the ride
          </Button>
        )}
        {planned && (
          <Button kind="primary" onClick={() => (onClose(), go("ride", { mode: "erg", workout_id: s.workout_id, plan_session_id: s.id }))}>
            Ride this now
          </Button>
        )}
        {planned && <Button onClick={() => (onClose(), go("routes", { pick_for_workout: s.workout_id }))}>Ride it on a map</Button>}
        {editable && (
          <>
            <Button kind={edit === "move" ? "secondary" : "quiet"} onClick={() => setEdit("move")}>
              Move
            </Button>
            <Button kind={edit === "replace" ? "secondary" : "quiet"} onClick={() => setEdit("replace")}>
              Swap workout
            </Button>
            <Button kind={edit === "shorten" ? "secondary" : "quiet"} onClick={() => setEdit("shorten")}>
              Shorten
            </Button>
            <Button kind="quiet" disabled={!!busy} onClick={() => propose({ action: "skip" })}>
              Skip
            </Button>
          </>
        )}
        {s.display_status === "skipped" && (
          <Button disabled={!!busy} onClick={() => propose({ action: "restore" })}>
            Restore
          </Button>
        )}
      </div>
      {edit && (
        <div className="panel" style={{ marginTop: 14 }}>
          {edit === "move" && (
            <Field label="New date" hint="Within this plan. The coach checks rest days and spacing between hard sessions.">
              <input type="date" value={date} min={today > plan.start ? today : plan.start} max={end} onChange={(e) => setDate(e.target.value)} />
            </Field>
          )}
          {edit === "replace" && (
            <Field label="Workout" hint="Suggested alternatives first; anything outside your current level is rejected with a reason.">
              <select value={wid} onChange={(e) => setWid(e.target.value)}>
                <option value="">Choose…</option>
                {alts.length > 0 && (
                  <optgroup label="Suggested">
                    {alts.map((a, i) => (
                      <option key={a} value={a}>
                        {s.alternative_names?.[i] ?? a}
                      </option>
                    ))}
                  </optgroup>
                )}
                <optgroup label="All workouts">
                  {list
                    .filter((w) => w.id !== s.workout_id)
                    .map((w) => (
                      <option key={w.id} value={w.id}>
                        {w.name} · {minutes(w.duration_s)}
                      </option>
                    ))}
                </optgroup>
              </select>
            </Field>
          )}
          {edit === "shorten" && (
            <Field label="Keep this much of the workout" hint="50–90%. The warm-up and cool-down stay; the main set is trimmed.">
              <NumberInput value={keep} min={50} max={90} step={5} suffix="%" onChange={setKeep} />
            </Field>
          )}
          <Field label="Reason (optional)">
            <input type="text" value={reason} maxLength={200} onChange={(e) => setReason(e.target.value)} placeholder="e.g. travelling on Thursday" />
          </Field>
          <div className="actions" style={{ marginTop: 10 }}>
            <Button
              kind="primary"
              disabled={!!busy || (edit === "replace" && !wid) || (edit === "move" && (!date || date === s.date))}
              onClick={() =>
                propose(edit === "move" ? { action: "move", date } : edit === "replace" ? { action: "replace", workout_id: wid } : { action: "shorten", keep_pct: Math.max(50, Math.min(90, keep ?? 70)) })
              }
            >
              Preview change
            </Button>
            <Button kind="quiet" onClick={() => setEdit(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
