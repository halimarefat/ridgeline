// Plan proposal review (accept / reject) and small plan building blocks.
import { useState } from "react";
import { rpc, type J } from "./api";
import { useApp, useRpc } from "./state";
import { Button, ErrorText, Modal, Spinner } from "./ui";
import { WorkoutChart } from "./charts";
import { dateLabel, minutes } from "./format";
import { useAction } from "./hooks";

export function IntensityTag({ v }: { v: string }) {
  const label: Record<string, string> = { easy: "Easy", moderate: "Moderate", hard: "Hard", maximal: "Maximal" };
  return <span className={`tag tag-${v}`}>{label[v] ?? v}</span>;
}

export function SourceTag({ source, provider, model }: { source: string; provider?: string; model?: string }) {
  if (source === "ai") return <span className="tag tag-ai">AI · {[provider, model].filter(Boolean).join(" · ") || "model"}</span>;
  if (source === "offline") return <span className="tag">Offline plan (rules-based, not an AI conversation)</span>;
  if (source === "safety") return <span className="tag" style={{ borderColor: "var(--bad)", color: "var(--bad)" }}>Safety</span>;
  if (source === "rules") return <span className="tag">Coaching rules</span>;
  if (source === "rider") return <span className="tag">Your edit</span>;
  return <span className="tag">{source}</span>;
}

const KIND_LABEL: Record<string, string> = {
  initial: "New plan",
  adaptation: "Adaptation from your feedback",
  readiness: "Today's readiness",
  chat: "Change suggested by the coach",
  edit: "Your change",
};

const ACTION_LABEL: Record<string, string> = { replace: "Replace", move: "Move", shorten: "Shorten", skip: "Skip", restore: "Restore" };

export function ProposalModal({ id, onClose }: { id: string; onClose: (accepted: boolean) => void }) {
  const { data, error, loading } = useRpc("getProposal", { id }, [id]);
  const { toast, bumpData } = useApp();
  const { run, busy } = useAction();
  const [showInput, setShowInput] = useState(false);
  const p: J = data;
  const pending = p?.status === "pending";
  return (
    <Modal
      wide
      title={p ? KIND_LABEL[p.kind] ?? "Proposal" : "Proposal"}
      onClose={() => onClose(false)}
      footer={
        pending ? (
          <>
            <Button
              kind="danger"
              disabled={!!busy}
              onClick={() =>
                run("reject", async () => {
                  await rpc("rejectProposal", { id });
                  bumpData();
                  onClose(false);
                })
              }
            >
              Reject
            </Button>
            <Button
              kind="primary"
              disabled={!!busy}
              onClick={() =>
                run("accept", async () => {
                  await rpc("acceptProposal", { id });
                  bumpData();
                  toast("Plan updated. You can undo this from the Calendar.");
                  onClose(true);
                })
              }
            >
              Accept plan
            </Button>
          </>
        ) : (
          <Button onClick={() => onClose(false)}>Close</Button>
        )
      }
    >
      {loading && <Spinner />}
      <ErrorText>{error}</ErrorText>
      {p && (
        <div className="stack">
          <div className="actions">
            <SourceTag source={p.source} provider={p.provider} model={p.model} />
            {!pending && <span className="tag">{p.status}</span>}
            {p.repaired && <span className="tag">AI output repaired once</span>}
            <span className="tag">Policy {p.policy_version}</span>
          </div>
          {p.fallback_reason && (
            <div className="banner banner-info">
              <p>{p.fallback_reason}</p>
            </div>
          )}
          {p.explanation && <p style={{ whiteSpace: "pre-wrap" }}>{p.explanation}</p>}
          {p.changes?.length > 0 && (
            <section>
              <h3>Changes</h3>
              <table className="table">
                <thead>
                  <tr>
                    <th>Change</th>
                    <th>Session</th>
                    <th>Result</th>
                    <th>Why</th>
                  </tr>
                </thead>
                <tbody>
                  {p.changes.map((c: J, i: number) => (
                    <tr key={i}>
                      <td>{ACTION_LABEL[c.action] ?? c.action}</td>
                      <td>
                        {c.date_before && dateLabel(c.date_before)} · {c.workout_before}
                      </td>
                      <td>
                        {c.action === "move" && c.date && `→ ${dateLabel(c.date)}`}
                        {c.action === "replace" && `→ ${c.workout_after}`}
                        {c.action === "shorten" && `→ ${c.keep_pct}% of the time`}
                        {c.action === "skip" && "→ removed (not stacked onto other days)"}
                        {c.action === "restore" && "→ restored"}
                      </td>
                      <td className="small muted">{c.reason}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}
          {p.plan && <PlanWeeks plan={p.plan} />}
          {p.input_summary && p.source === "ai" && (
            <section>
              <Button kind="quiet" onClick={() => setShowInput((x) => !x)}>
                {showInput ? "Hide" : "Show"} what was sent to the AI
              </Button>
              {showInput && <pre className="code">{JSON.stringify(p.input_summary, null, 2)}</pre>}
            </section>
          )}
        </div>
      )}
    </Modal>
  );
}

export function PlanWeeks({ plan }: { plan: J }) {
  const weeks: J[] = plan.weeks_summary ?? [];
  const sessions: J[] = plan.sessions ?? [];
  return (
    <section>
      <h3>
        {plan.weeks} weeks from {dateLabel(plan.start)} · level: {plan.level}
      </h3>
      <table className="table">
        <thead>
          <tr>
            <th>Week</th>
            <th className="num">Volume</th>
            <th className="num">Rides</th>
            <th className="num">Hard</th>
            <th className="num">Moderate</th>
            <th>Sessions</th>
          </tr>
        </thead>
        <tbody>
          {weeks.map((w) => (
            <tr key={w.index}>
              <td>
                {w.index + 1} · {dateLabel(w.start)}
                {w.recovery_week && <div className="small muted">Recovery week</div>}
              </td>
              <td className="num">{minutes(w.minutes * 60)}</td>
              <td className="num">{w.sessions}</td>
              <td className="num">{w.hard}</td>
              <td className="num">{w.moderate}</td>
              <td>
                {sessions
                  .filter((s) => s.date >= w.start && s.date < addWeek(w.start))
                  .map((s) => (
                    <div key={s.id} className="small" style={{ marginBottom: 4 }}>
                      <b>{dateLabel(s.date)}</b> {s.workout_name ?? s.workout_id} · {minutes(s.duration_s)} <IntensityTag v={s.intensity} />
                      {s.why && <div className="muted">{s.why}</div>}
                    </div>
                  ))}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function addWeek(iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d + 7)).toISOString().slice(0, 10);
}

/** Compact session card used on Home and in the calendar dialog. */
export function SessionSummary({ s }: { s: J }) {
  return (
    <div className="stack">
      <div className="actions">
        <IntensityTag v={s.intensity} />
        <span className="tag">{s.category}</span>
        <span className="tag">{minutes(s.duration_s)}</span>
        {s.keep_pct < 100 && <span className="tag">shortened to {s.keep_pct}%</span>}
      </div>
      {s.profile && <WorkoutChart timeline={s.profile} height={70} compact />}
      {s.purpose && <p>{s.purpose}</p>}
      {s.why && <p className="muted">Why this fits you: {s.why}</p>}
      {s.alternative_names?.length > 0 && <p className="small muted">Alternatives: {s.alternative_names.join(", ")}</p>}
    </div>
  );
}
