// Coach: plan creation, adaptation and a conversation. The offline coach is
// always available; an AI model (local by default) is used only when the
// rider configured one and consented. Replies are labelled by source.
import { useEffect, useRef, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Button, ErrorText, Field, NumberInput, Spinner, Toggle } from "../ui";
import { ProposalModal, SourceTag } from "../plan";
import { dateTime } from "../format";
import { useAction, useJob } from "../hooks";

const STARTERS = ["Why is this week structured like this?", "I only have 30 minutes on Thursday.", "Make Saturday easier, I'm travelling.", "What does sweet spot mean?"];

export function Coach() {
  const { boot, go, dataVersion, bumpData } = useApp();
  const plan = useRpc("getPlan", {}, [dataVersion]);
  const settings = useRpc("getSettings", {}, [dataVersion]);
  const policy = useRpc("getPolicy", {});
  const hist = useRpc("coachHistory", {}, [dataVersion]);
  const [proposal, setProposal] = useState<string | null>(null);
  const [msg, setMsg] = useState("");
  const [weeks, setWeeks] = useState<number | null>(null);
  const [start, setStart] = useState<string>(boot?.today ?? "");
  const [useAi, setUseAi] = useState(true);
  const [showSummary, setShowSummary] = useState(false);
  const planJob = useJob();
  const chatJob = useJob();
  const { run, busy } = useAction();
  const chatRef = useRef<HTMLDivElement>(null);
  const st: J = settings.data?.settings;
  const ai = st?.ai;
  const consent = boot?.profile?.ai_consent?.enabled;
  const aiReady = ai && ai.provider !== "offline" && (ai.provider === "local" || ai.remote_enabled) && consent;
  const messages: J[] = hist.data ?? [];
  const defWeeks = policy.data?.default_plan_weeks ?? 4;
  const maxWeeks = policy.data?.max_plan_weeks ?? 12;

  useEffect(() => {
    chatRef.current?.scrollTo({ top: chatRef.current.scrollHeight });
  }, [messages.length, chatJob.progress]);

  const send = async (text: string) => {
    const t = text.trim();
    if (!t) return;
    setMsg("");
    const r = await chatJob.start("coachChat", { message: t });
    hist.reload();
    if (r?.proposal_id) bumpData();
  };

  const createPlan = async () => {
    const r = await planJob.start("proposePlan", { weeks: weeks ?? defWeeks, start: start || undefined, use_ai: useAi && !!aiReady });
    if (r?.proposal_id) {
      bumpData();
      setProposal(r.proposal_id);
    }
  };

  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Coach</h1>
          <p className="sub">Plans follow a written coaching policy. Nothing changes in your calendar until you accept a proposal.</p>
        </div>
        <div className="actions">
          <CoachMode ai={ai} consent={consent} />
          <Button kind="quiet" onClick={() => go("settings", { tab: "ai" })}>
            AI settings
          </Button>
        </div>
      </div>

      <div className="grid-2">
        <section className="panel">
          <div className="panel-head">
            <h2>Conversation</h2>
            {messages.length > 0 && (
              <Button kind="quiet" disabled={!!busy} onClick={() => run("clear", () => rpc("clearCoachHistory", {}).then(hist.reload))}>
                Clear
              </Button>
            )}
          </div>
          <div className="chat" ref={chatRef} aria-live="polite">
            {messages.length === 0 && (
              <p className="muted">
                Ask about your plan, or ask for a change. {aiReady ? "Replies come from your configured AI model, checked against the coaching policy." : "The offline coach answers common questions and handles simple changes; connect a local AI model in Settings for open conversation."}
              </p>
            )}
            {messages.map((m, i) => (
              <div key={i} className={`msg ${m.role === "rider" ? "msg-rider" : "msg-coach"} ${m.safety ? "msg-safety" : ""}`}>
                {m.text}
                {m.role !== "rider" && (
                  <div className="msg-meta">
                    <SourceTag source={m.source} />
                    {m.utc && <span>{dateTime(m.utc)}</span>}
                    {m.proposal_id && (
                      <Button kind="quiet" onClick={() => setProposal(m.proposal_id)}>
                        Review the suggested change
                      </Button>
                    )}
                  </div>
                )}
                {m.note && <div className="msg-meta warn-text">{m.note}</div>}
              </div>
            ))}
            {chatJob.progress && (
              <div className="msg msg-coach">
                <Spinner label={chatJob.progress} />{" "}
                <Button kind="quiet" onClick={chatJob.cancel}>
                  Cancel
                </Button>
              </div>
            )}
          </div>
          {messages.length === 0 && (
            <div className="actions" style={{ marginTop: 10 }}>
              {STARTERS.map((s) => (
                <Button key={s} kind="quiet" onClick={() => send(s)} disabled={!!chatJob.progress}>
                  {s}
                </Button>
              ))}
            </div>
          )}
          <form
            className="composer"
            onSubmit={(e) => {
              e.preventDefault();
              send(msg);
            }}
          >
            <textarea
              aria-label="Message to the coach"
              value={msg}
              maxLength={2000}
              placeholder="Ask the coach…"
              onChange={(e) => setMsg(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  send(msg);
                }
              }}
            />
            <Button kind="primary" type="submit" disabled={!msg.trim() || !!chatJob.progress}>
              Send
            </Button>
          </form>
          <p className="small muted">The coach gives training suggestions, not medical advice. If you mention warning symptoms it will tell you to stop and seek care.</p>
        </section>

        <div>
          <section className="panel">
            <h2>{plan.data?.plan ? "Make a new plan" : "Create your plan"}</h2>
            <p className="muted small">
              Built around your available days and recent riding, with a recovery week every few weeks. {plan.data?.plan ? "Accepting a new plan replaces the current one (you can undo)." : ""}
            </p>
            <div className="cols">
              <Field label="Weeks">
                <NumberInput value={weeks ?? defWeeks} min={1} max={maxWeeks} onChange={setWeeks} />
              </Field>
              <Field label="Start">
                <input type="date" value={start} min={boot?.today} onChange={(e) => setStart(e.target.value)} />
              </Field>
            </div>
            {aiReady && (
              <div style={{ marginTop: 10 }}>
                <Toggle checked={useAi} onChange={setUseAi} label={`Ask the AI model (${ai.model}) to personalise it`} hint="Its proposal is validated against the policy; if it fails twice, the offline plan is used and you'll see why." />
              </div>
            )}
            <div className="actions" style={{ marginTop: 12 }}>
              <Button kind="primary" disabled={!!planJob.progress} onClick={createPlan}>
                Propose a plan
              </Button>
              {planJob.progress && (
                <>
                  <Spinner label={planJob.progress} />
                  <Button kind="quiet" onClick={planJob.cancel}>
                    Cancel
                  </Button>
                </>
              )}
            </div>
            {plan.data?.plan && (
              <div className="actions" style={{ marginTop: 12 }}>
                <Button
                  disabled={!!busy}
                  onClick={() =>
                    run("adapt", async () => {
                      const r = await rpc("adaptPlan", {});
                      if (r.proposal_id) {
                        bumpData();
                        setProposal(r.proposal_id);
                      } else hist.reload();
                      return r;
                    }).then((r) => r && !r.proposal_id && setMsg(""))
                  }
                >
                  Adjust from my recent feedback
                </Button>
                <Button kind="quiet" onClick={() => go("calendar")}>
                  Open calendar
                </Button>
              </div>
            )}
            {(plan.data?.pending ?? []).length > 0 && (
              <div className="banner banner-info" style={{ marginTop: 12 }}>
                <p>{plan.data.pending.length} proposal(s) waiting for your decision.</p>
                <Button onClick={() => setProposal(plan.data.pending[0].id)}>Review</Button>
              </div>
            )}
          </section>

          <section className="panel">
            <h3>What the AI can see</h3>
            {!consent && <p className="small">AI sharing is off, so nothing is sent anywhere. The offline coach works entirely on this computer.</p>}
            {consent && (
              <p className="small">
                Only this compact summary is sent to {ai?.provider === "local" ? "the model on this computer" : "your configured service"} — no second-by-second data, locations or route names.
              </p>
            )}
            <Button kind="quiet" onClick={() => setShowSummary((x) => !x)}>
              {showSummary ? "Hide" : "Show"} the summary
            </Button>
            {showSummary && <pre className="code">{JSON.stringify(settings.data?.ai_summary_preview ?? null, null, 2)}</pre>}
          </section>

          <section className="panel">
            <h3>Coaching policy</h3>
            <ErrorText>{policy.error}</ErrorText>
            {policy.data && (
              <>
                <p className="small">
                  Version {policy.data.version}. {policy.data.review_status}
                </p>
                <table className="table">
                  <thead>
                    <tr>
                      <th>Level</th>
                      <th className="num">Hard / week</th>
                      <th className="num">Rest days</th>
                      <th className="num">Weekly growth</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(policy.data.levels as J[]).map((l) => (
                      <tr key={l.level}>
                        <td>{l.level}</td>
                        <td className="num">{l.hard_per_week}</td>
                        <td className="num">≥{l.min_rest_days_per_week}</td>
                        <td className="num">≤{Math.round(l.weekly_growth * 100)}%</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p className="small muted">
                  At least {policy.data.min_days_between_hard} day(s) between hard sessions; a recovery week every {policy.data.recovery_week_every} weeks. Sources and rationale are in the docs
                  (coaching-policy.md).
                </p>
              </>
            )}
          </section>
        </div>
      </div>
      {proposal && <ProposalModal id={proposal} onClose={() => (setProposal(null), bumpData())} />}
    </div>
  );
}

function CoachMode({ ai, consent }: { ai: J; consent: boolean }) {
  if (!ai) return null;
  if (ai.provider === "offline") return <span className="tag">Offline coach</span>;
  if (!consent) return <span className="tag">AI configured · consent off</span>;
  if (ai.provider === "remote" && !ai.remote_enabled) return <span className="tag">Remote AI disabled</span>;
  return (
    <span className="tag tag-ai">
      {ai.provider === "local" ? "Local AI" : "Remote AI"} · {ai.model}
    </span>
  );
}
