// The coach on the Ride screen: ride cues and coach replies, one-tap prompts,
// suggestion buttons and optional speech. Suggestions only ever call the same
// commands as the ride controls, and only when the rider presses them.
import { useEffect, useRef, useState } from "react";
import { rpc, type J } from "./api";
import { useApp } from "./state";
import { Button, Spinner } from "./ui";
import { actionUsable, itemsToSpeak, QUICK_PROMPTS, sourceLabel, speechText, type FeedItem } from "./coachfeed";
import { clock } from "./format";

export function speechAvailable(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window && typeof SpeechSynthesisUtterance !== "undefined";
}

function speak(text: string) {
  if (!speechAvailable()) return;
  const u = new SpeechSynthesisUtterance(speechText(text));
  u.rate = 1.05;
  window.speechSynthesis.speak(u);
}

/** Send a quick prompt or message to the ride coach. */
export async function askCoach(trigger: string, text?: string): Promise<void> {
  await rpc("rideCoach", text ? { trigger, text } : { trigger });
}

export function CoachPanel({ s, onStop }: { s: J; onStop: () => void }) {
  const { live, boot, refreshBoot, toast } = useApp();
  const feed: FeedItem[] = s.coach ?? [];
  const busy = !!live?.ride_coach_busy;
  const settings = boot?.settings;
  const aiOn = settings?.ai?.provider && settings.ai.provider !== "offline";
  const model: string | undefined = aiOn ? settings?.ai?.model : undefined;
  const voice = !!settings?.ride_coach?.voice;
  const [text, setText] = useState("");
  const [applied, setApplied] = useState<Set<number>>(new Set());
  const [sending, setSending] = useState(false);
  const lastSpoken = useRef<number | null>(null);
  const latestId = feed.length ? feed[feed.length - 1].id : 0;

  // Speak new items when voice is on (never the backlog from before the panel opened).
  useEffect(() => {
    if (lastSpoken.current == null) {
      lastSpoken.current = latestId;
      return;
    }
    const fresh = itemsToSpeak(feed, lastSpoken.current);
    lastSpoken.current = Math.max(lastSpoken.current, latestId);
    if (voice) fresh.forEach((f) => speak(f.text));
  }, [latestId, voice]); // eslint-disable-line react-hooks/exhaustive-deps

  const ask = async (trigger: string, msg?: string) => {
    setSending(true);
    try {
      await askCoach(trigger, msg);
      if (msg) setText("");
    } catch (e) {
      toast(String((e as Error).message), "warn");
    } finally {
      setSending(false);
    }
  };

  const toggleVoice = async () => {
    try {
      const cur = await rpc("getSettings", {});
      const next = { ...cur.settings, ride_coach: { ...cur.settings.ride_coach, voice: !voice } };
      await rpc("saveSettings", { settings: next });
      await refreshBoot();
      if (!voice && speechAvailable()) speak("Voice coaching on.");
      if (voice && speechAvailable()) window.speechSynthesis.cancel();
    } catch (e) {
      toast(String((e as Error).message), "warn");
    }
  };

  const doAction = async (item: FeedItem) => {
    if (!item.action) return;
    if (item.action.kind === "stop") {
      onStop();
      return;
    }
    try {
      await rpc("adjustIntensity", { delta: item.action.delta, via: "coach" });
      setApplied((a) => new Set(a).add(item.id));
    } catch (e) {
      toast(String((e as Error).message), "warn");
    }
  };

  const shown = feed.slice(-6);
  const latest = [...shown].reverse().find((f) => f.from !== "rider" && f.from !== "note");
  const earlier = shown.filter((f) => f !== latest).reverse();
  const hasWorkout = !!s.workout;

  const itemView = (f: FeedItem, big: boolean) => (
    <div key={f.id} className={`coach-item coach-${f.from} ${big ? "coach-latest" : ""}`}>
      <span className="coach-meta">
        <span className={`coach-src coach-src-${f.from}`}>{sourceLabel(f.from, model)}</span>
        <span className="muted"> · {clock(f.at_s)}</span>
      </span>
      <span className="coach-text">{f.text}</span>
      {f.action && (
        <span className="coach-action">
          {applied.has(f.id) ? (
            <span className="tag tag-sign">Applied ✓</span>
          ) : actionUsable(f, latestId, s.state, hasWorkout, applied) ? (
            <Button kind={f.action.kind === "stop" ? "danger" : "primary"} onClick={() => doAction(f)}>
              {f.action.label}
            </Button>
          ) : null}
        </span>
      )}
    </div>
  );

  return (
    <section className="panel coach-panel" aria-label="Coach">
      <div className="panel-head">
        <h3>Coach</h3>
        <div className="actions">
          <span className={`tag ${aiOn ? "tag-ai" : ""}`}>{aiOn ? `AI · ${model}` : "Offline coach"}</span>
          {speechAvailable() && (
            <Button kind="quiet" onClick={toggleVoice} title="Read the coach aloud with your computer's voice">
              {voice ? "🔊 Voice on" : "🔈 Voice off"}
            </Button>
          )}
        </div>
      </div>
      <div className="coach-grid">
        <div className="coach-feed" aria-live="polite" aria-relevant="additions">
          {!latest && <p className="muted">Cues for the next interval, cadence and climbs appear here. Ask the coach anything with the buttons.</p>}
          {latest && itemView(latest, true)}
          {earlier.length > 0 && <div className="coach-earlier">{earlier.map((f) => itemView(f, false))}</div>}
        </div>
        <div className="coach-ask">
          <div className="coach-prompts" role="group" aria-label="Ask the coach">
            {QUICK_PROMPTS.map((q) => (
              <Button key={q.trigger} onClick={() => ask(q.trigger)} disabled={busy || sending} kbd={q.key}>
                {q.label}
              </Button>
            ))}
          </div>
          <form
            className="coach-input"
            onSubmit={(e) => {
              e.preventDefault();
              if (text.trim()) ask("message", text.trim());
            }}
          >
            <input
              id="coach-input"
              type="text"
              value={text}
              maxLength={300}
              placeholder={aiOn ? "Ask the coach… (C)" : "Ask the offline coach… (C)"}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && (e.currentTarget as HTMLInputElement).blur()}
              aria-label="Message to the coach"
            />
            <Button type="submit" disabled={busy || sending || !text.trim()}>
              Send
            </Button>
          </form>
          {busy && <Spinner label="The coach is thinking…" />}
          {!busy && <p className="small muted">Ride shortcuts are off while typing. Esc returns to the ride.</p>}
        </div>
      </div>
    </section>
  );
}
