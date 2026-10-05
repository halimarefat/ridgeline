// App shell: first-launch welcome, onboarding gate, navigation, device bar,
// toasts, interrupted-ride recovery and the close-during-ride guard.
import { useEffect, useState, type ComponentType } from "react";
import { exitApp, onDesktopEvent, rpc, type J } from "./api";
import { useApp, type ScreenName } from "./state";
import { Button, Empty, ErrorText, Modal, Spinner, Status } from "./ui";
import { dateTime, round } from "./format";
import { Onboarding } from "./screens/onboarding";
import { Home } from "./screens/home";
import { Devices } from "./screens/devices";
import { Ride } from "./screens/ride";
import { Workouts } from "./screens/workouts";
import { Routes } from "./screens/routes";
import { Calendar } from "./screens/calendar";
import { Coach } from "./screens/coach";
import { History } from "./screens/history";
import { Settings } from "./screens/settings";
import { useAction } from "./hooks";

const NAV: { name: ScreenName; label: string; icon: string }[] = [
  { name: "home", label: "Home", icon: "⌂" },
  { name: "ride", label: "Ride", icon: "▶" },
  { name: "calendar", label: "Calendar", icon: "▦" },
  { name: "workouts", label: "Workouts", icon: "▤" },
  { name: "routes", label: "Routes", icon: "⛰" },
  { name: "coach", label: "Coach", icon: "✎" },
  { name: "history", label: "History", icon: "↺" },
];
const NAV2: { name: ScreenName; label: string; icon: string }[] = [
  { name: "devices", label: "Devices", icon: "⌁" },
  { name: "settings", label: "Settings", icon: "⚙" },
];

export function Root() {
  const { boot, bootError, refreshBoot, screen } = useApp();
  useTheme();
  if (bootError) {
    return (
      <div className="boot">
        <div className="panel" style={{ maxWidth: 560 }}>
          <h2>Ridgeline could not start</h2>
          <ErrorText>{bootError}</ErrorText>
          <p className="muted">If another Ridgeline window is open, close it first: only one window can control the trainer at a time.</p>
          <Button kind="primary" onClick={refreshBoot}>
            Try again
          </Button>
        </div>
      </div>
    );
  }
  if (!boot) return <div className="boot">Starting Ridgeline…</div>;
  if (!boot.onboarded) return screen.name === "onboarding" ? <Onboarding /> : <Welcome />;
  return <Shell />;
}

function useTheme() {
  const { boot } = useApp();
  const theme: string = boot?.settings?.theme ?? "dark";
  useEffect(() => {
    const apply = () => {
      const t = theme === "system" ? (window.matchMedia?.("(prefers-color-scheme: light)").matches ? "light" : "dark") : theme;
      document.documentElement.dataset.theme = t;
    };
    apply();
    const mq = window.matchMedia?.("(prefers-color-scheme: light)");
    mq?.addEventListener?.("change", apply);
    return () => mq?.removeEventListener?.("change", apply);
  }, [theme]);
}

/** One-screen explanation with the two entry points (spec §3). */
function Welcome() {
  const { go, refreshBoot, toast } = useApp();
  const [busy, setBusy] = useState(false);
  const demo = async () => {
    setBusy(true);
    try {
      await rpc("startDemo", {});
      await refreshBoot();
      go("home");
      toast("Demo mode: simulated trainer and sensors. Demo rides are labelled and never count as real rides.", "info");
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="welcome">
      <div className="welcome-art" aria-hidden="true">
        <WelcomeArt />
      </div>
      <div className="welcome-copy">
        <p className="tag tag-sign" style={{ alignSelf: "flex-start" }}>
          Indoor cycling · local-first
        </p>
        <h1>Ride real roads. Train with a plan.</h1>
        <p className="muted" style={{ fontSize: "1.1rem" }}>
          Ridgeline connects to your Bluetooth smart trainer and sensors, builds a training plan around your week, runs structured ERG workouts, and lets you ride mapped routes where the
          trainer follows the road's gradient.
        </p>
        <ul>
          <li>Works offline once routes and workouts are on your computer. No account needed.</li>
          <li>The coach runs on rules, or on a free AI model on your own computer if you choose.</li>
          <li>Pause and Stop are always on screen. If the trainer disconnects, Ridgeline never silently restores a hard effort.</li>
        </ul>
        <div className="actions" style={{ marginTop: 8 }}>
          <Button kind="primary" big onClick={() => go("onboarding")}>
            Set up my bike
          </Button>
          <Button big onClick={demo} disabled={busy}>
            {busy ? "Starting demo…" : "Try a demo"}
          </Button>
        </div>
        <p className="small muted">The demo uses a simulated trainer, heart-rate strap and cadence sensor. You can set up your real equipment any time from Devices.</p>
      </div>
    </div>
  );
}

function WelcomeArt() {
  // A stylised stage profile: original artwork.
  const pts = [0, 40, 38, 52, 47, 70, 96, 88, 120, 150, 132, 118, 160, 205, 190, 178, 230, 262, 240, 210, 196, 170, 150, 120, 104, 80, 74, 60];
  const w = 1000;
  const h = 520;
  const step = w / (pts.length - 1);
  const y = (v: number) => h - 60 - v * 1.5;
  let d = `M0,${h} `;
  pts.forEach((v, i) => (d += `L${(i * step).toFixed(1)},${y(v).toFixed(1)} `));
  d += `L${w},${h} Z`;
  let road = "";
  pts.forEach((v, i) => (road += `${i ? "L" : "M"}${(i * step).toFixed(1)},${y(v).toFixed(1)} `));
  return (
    <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id="hill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--g4)" stopOpacity="0.85" />
          <stop offset="0.55" stopColor="var(--g3)" stopOpacity="0.45" />
          <stop offset="1" stopColor="var(--tarmac)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={d} fill="url(#hill)" />
      <path d={road} fill="none" stroke="var(--sign)" strokeWidth="5" strokeDasharray="22 14" />
      <circle cx={step * 13} cy={y(pts[13])} r="13" fill="var(--chalk)" stroke="var(--asphalt)" strokeWidth="5" />
    </svg>
  );
}

function Shell() {
  const { screen, go, live, boot } = useApp();
  const rideActive = !!live?.ride_active;
  const S = screens[screen.name] ?? Home;
  return (
    <div className="shell">
      <nav className="nav" aria-label="Main">
        <div className="brand">
          <img src="icon.png" alt="" />
          <span>Ridgeline</span>
        </div>
        {NAV.map((n) => (
          <button key={n.name} aria-current={screen.name === n.name ? "page" : undefined} onClick={() => go(n.name)} title={n.label}>
            <span className="nav-icon" aria-hidden="true">
              {n.icon}
            </span>
            <span className={`nav-text ${n.name === "ride" && rideActive ? "ride-live" : ""}`}>{n.name === "ride" && rideActive ? "Ride · live" : n.label}</span>
          </button>
        ))}
        <div className="nav-sep" />
        {NAV2.map((n) => (
          <button key={n.name} aria-current={screen.name === n.name ? "page" : undefined} onClick={() => go(n.name)} title={n.label}>
            <span className="nav-icon" aria-hidden="true">
              {n.icon}
            </span>
            <span className="nav-text">{n.label}</span>
          </button>
        ))}
        <div className="nav-foot">
          v{boot?.version} · {boot?.platform}
          <br />
          Policy {boot?.policy_version}
        </div>
      </nav>
      <main className="content" id="main">
        <S />
      </main>
      <DevBar />
      <Toasts />
      <RecoveryPrompt />
      <CloseGuard />
    </div>
  );
}

const screens: Record<string, ComponentType> = {
  home: Home,
  ride: Ride,
  calendar: Calendar,
  workouts: Workouts,
  routes: Routes,
  coach: Coach,
  history: History,
  devices: Devices,
  settings: Settings,
};

/** Bottom status strip: devices and live values, always visible. */
function DevBar() {
  const { live, go } = useApp();
  const devs: J[] = live?.devices?.devices ?? [];
  const trainer = devs.find((d) => d.is_trainer);
  const ctl = live?.devices?.control?.state;
  const r = (x: J) => (x?.freshness === "fresh" ? round(x.value) : x?.freshness === "no_source" ? "—" : "stale");
  const others = devs.filter((d) => !d.is_trainer && d.state === "ready");
  return (
    <div className="devbar" role="status" aria-live="off">
      {live?.demo && <span className="demo-flag">DEMO</span>}
      <button className="icon-btn" onClick={() => go("devices")} title="Open Devices">
        {trainer ? (
          <Status kind={trainer.state === "ready" ? "ok" : trainer.state === "reconnecting" || trainer.state === "connecting" ? "busy" : "bad"}>
            Trainer: {trainer.name} ({trainer.state}
            {ctl === "controlled" ? ", controlled" : ""})
          </Status>
        ) : (
          <Status kind="idle">No trainer selected</Status>
        )}
      </button>
      {others.length > 0 && <span>{others.map((d) => d.name).join(" · ")}</span>}
      <span>
        Power <b>{r(live?.live?.power)}</b> W
      </span>
      <span>
        Cadence <b>{r(live?.live?.cadence)}</b> rpm
      </span>
      <span>
        HR <b>{r(live?.live?.heart_rate)}</b> bpm
      </span>
    </div>
  );
}

function Toasts() {
  const { toasts, dismissToast } = useApp();
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.level}`} role={t.level === "error" ? "alert" : "status"}>
          <p>{t.text}</p>
          <button className="icon-btn" onClick={() => dismissToast(t.id)} aria-label="Dismiss">
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}

/** Offer recovery of rides interrupted by a crash or power loss (A13). */
function RecoveryPrompt() {
  const { boot, refreshBoot, toast, bumpData } = useApp();
  const { run, busy } = useAction();
  const [hidden, setHidden] = useState(false);
  const items: J[] = boot?.recovery ?? [];
  if (!items.length || hidden) return null;
  return (
    <Modal title="Interrupted ride found" onClose={() => setHidden(true)}>
      <p>Ridgeline found a ride that was not saved properly (the app or computer stopped during the ride). Its data was journaled every few seconds.</p>
      <p className="muted small">Recovering does not start or control the trainer.</p>
      {items.map((a) => (
        <div key={a.id} className="list-item" style={{ cursor: "default" }}>
          <div>
            <div className="li-title">
              {a.title} {a.demo && <span className="tag">demo</span>}
            </div>
            <div className="li-meta">Started {dateTime(a.start_utc)}</div>
          </div>
          <div className="actions">
            <Button
              kind="primary"
              disabled={!!busy}
              onClick={() =>
                run("recover", async () => {
                  const r = await rpc("recoverActivity", { id: a.id });
                  toast(`Recovered ${Math.round(r.summary?.timer_s / 60)} min of riding${r.corrupt_lines ? ` (${r.corrupt_lines} damaged record(s) skipped)` : ""}.`);
                  await refreshBoot();
                  bumpData();
                })
              }
            >
              Recover
            </Button>
            <Button
              kind="danger"
              disabled={!!busy}
              onClick={() =>
                run("discard", async () => {
                  await rpc("discardRecovery", { id: a.id });
                  await refreshBoot();
                })
              }
            >
              Discard
            </Button>
          </div>
        </div>
      ))}
    </Modal>
  );
}

/** Closing the window during a ride: stop and save first (A17). */
function CloseGuard() {
  const { live } = useApp();
  const [asked, setAsked] = useState(false);
  const [stopping, setStopping] = useState(false);
  useEffect(() => {
    let un: (() => void) | undefined;
    onDesktopEvent("ridgeline://close-requested", () => setAsked(true)).then((u) => (un = u));
    return () => un?.();
  }, []);
  useEffect(() => {
    if (stopping && live && !live.ride_active) {
      exitApp();
    }
  }, [stopping, live]);
  if (!asked) return null;
  return (
    <Modal title="A ride is in progress" onClose={() => !stopping && setAsked(false)}>
      {stopping ? (
        <p>
          <Spinner label="Stopping the ride, easing the trainer and saving…" />
        </p>
      ) : (
        <>
          <p>Ridgeline will stop the ride, ask the trainer to ease off, save your data and then close.</p>
          <p className="muted small">If the trainer doesn't confirm the stop it may keep its last resistance — stop pedalling if needed.</p>
          <div className="actions">
            <Button
              kind="primary"
              onClick={async () => {
                setStopping(true);
                await rpc("stopSession", { save: true }).catch(() => {});
              }}
            >
              Stop, save and close
            </Button>
            <Button onClick={() => setAsked(false)}>Keep riding</Button>
          </div>
        </>
      )}
    </Modal>
  );
}

export function NotOnboarded() {
  const { go } = useApp();
  return <Empty title="Set up your profile first" action={<Button onClick={() => go("onboarding")}>Start setup</Button>} />;
}
