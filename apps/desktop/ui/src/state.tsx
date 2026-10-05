import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { rpc, type J } from "./api";
import { localTzInfo, type Units } from "./format";

export type ScreenName = "home" | "ride" | "workouts" | "routes" | "calendar" | "coach" | "history" | "devices" | "settings" | "onboarding";

export interface Screen {
  name: ScreenName;
  params?: Record<string, unknown>;
}

export interface Toast {
  id: number;
  level: "info" | "warn" | "error";
  text: string;
}

interface Ctx {
  boot: J | null;
  bootError: string | null;
  live: J | null;
  units: Units;
  screen: Screen;
  go: (name: ScreenName, params?: Record<string, unknown>) => void;
  refreshBoot: () => Promise<void>;
  toasts: Toast[];
  toast: (text: string, level?: Toast["level"]) => void;
  dismissToast: (id: number) => void;
  /** Bumped when plan/history data changes so screens can refetch. */
  dataVersion: number;
  bumpData: () => void;
}

const AppCtx = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const c = useContext(AppCtx);
  if (!c) throw new Error("AppProvider missing");
  return c;
}

let toastSeq = 0;

export function AppProvider({ children }: { children: ReactNode }) {
  const [boot, setBoot] = useState<J | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [live, setLive] = useState<J | null>(null);
  const [screen, setScreen] = useState<Screen>({ name: "home" });
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [dataVersion, setDataVersion] = useState(0);
  const seenNotices = useRef<Set<number>>(new Set());
  const seenSessionNotices = useRef<Set<string>>(new Set());

  const toast = useCallback((text: string, level: Toast["level"] = "info") => {
    const id = ++toastSeq;
    setToasts((t) => [...t.slice(-4), { id, level, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), level === "error" ? 12000 : 6000);
  }, []);
  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const refreshBoot = useCallback(async () => {
    try {
      const b = await rpc("getBootstrap", localTzInfo());
      setBoot(b);
      setBootError(null);
    } catch (e) {
      setBootError(String((e as Error).message));
    }
  }, []);

  useEffect(() => {
    refreshBoot();
  }, [refreshBoot]);

  // Live state: polled at 4 Hz. The native service keeps time and control on
  // its own clock; this loop only refreshes the display.
  useEffect(() => {
    let stop = false;
    let timer: number | undefined;
    const tick = async () => {
      try {
        const s = await rpc("getState", {});
        if (stop) return;
        setLive(s);
        for (const n of s.notices ?? []) {
          if (!seenNotices.current.has(n.id)) {
            seenNotices.current.add(n.id);
            toast(n.text, n.level === "warn" ? "warn" : n.level === "error" ? "error" : "info");
            rpc("dismissNotice", { id: n.id }).catch(() => {});
          }
        }
        for (const n of s.session?.notices ?? []) {
          const key = `${s.session.id}:${n.id}`;
          if (!seenSessionNotices.current.has(key)) {
            seenSessionNotices.current.add(key);
            toast(n.text, n.level === "warn" ? "warn" : n.level === "error" ? "error" : "info");
          }
        }
      } catch {
        /* transient; keep polling */
      }
      if (!stop) timer = window.setTimeout(tick, 250);
    };
    tick();
    return () => {
      stop = true;
      if (timer) clearTimeout(timer);
    };
  }, [toast]);

  const go = useCallback((name: ScreenName, params?: Record<string, unknown>) => {
    setScreen({ name, params });
    window.scrollTo(0, 0);
  }, []);

  const value = useMemo<Ctx>(
    () => ({
      boot,
      bootError,
      live,
      units: (boot?.settings?.units as Units) ?? "metric",
      screen,
      go,
      refreshBoot,
      toasts,
      toast,
      dismissToast,
      dataVersion,
      bumpData: () => setDataVersion((v) => v + 1),
    }),
    [boot, bootError, live, screen, go, refreshBoot, toasts, toast, dismissToast, dataVersion],
  );
  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>;
}

/** Fetch helper with loading/error state; refetches when deps change. */
export function useRpc<T = J>(method: string, params: Record<string, unknown> = {}, deps: unknown[] = []): { data: T | null; error: string | null; loading: boolean; reload: () => void } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [n, setN] = useState(0);
  const key = JSON.stringify(params);
  useEffect(() => {
    let alive = true;
    setLoading(true);
    rpc<T>(method, JSON.parse(key))
      .then((d) => {
        if (alive) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => alive && setError(String(e.message)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [method, key, n, ...deps]);
  return { data, error, loading, reload: () => setN((x) => x + 1) };
}

/** True when keyboard focus is in a text field (shortcuts must not fire). */
export function typingInField(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}
