// Small hooks shared by screens.
import { useCallback, useEffect, useRef, useState } from "react";
import { rpc, waitJob, type J } from "./api";
import { useApp } from "./state";

/** Run an async action with a busy flag; errors become toasts. */
export function useAction() {
  const { toast } = useApp();
  const [busy, setBusy] = useState<string | null>(null);
  const run = useCallback(
    async <T,>(label: string, fn: () => Promise<T>, okMsg?: string): Promise<T | undefined> => {
      setBusy(label);
      try {
        const r = await fn();
        if (okMsg) toast(okMsg);
        return r;
      } catch (e) {
        toast((e as Error).message, "error");
        return undefined;
      } finally {
        setBusy(null);
      }
    },
    [toast],
  );
  return { busy, run };
}

/** Call a command that may start a background job, reporting progress. */
export function useJob() {
  const { toast } = useApp();
  const [progress, setProgress] = useState<string | null>(null);
  const cancelRef = useRef<string | null>(null);
  const start = useCallback(
    async (method: string, params: Record<string, unknown>): Promise<J | undefined> => {
      setProgress("Starting…");
      try {
        const first = await rpc(method, params);
        if (first && typeof first === "object" && first.job_id) {
          cancelRef.current = first.job_id;
          return await waitJob(first.job_id, (m) => setProgress(m));
        }
        return first;
      } catch (e) {
        toast((e as Error).message, "error");
        return undefined;
      } finally {
        cancelRef.current = null;
        setProgress(null);
      }
    },
    [toast],
  );
  const cancel = useCallback(() => {
    if (cancelRef.current) rpc("cancelJob", { id: cancelRef.current }).catch(() => {});
  }, []);
  return { progress, start, cancel };
}

/** Debounced value. */
export function useDebounced<T>(v: T, ms: number): T {
  const [d, setD] = useState(v);
  useEffect(() => {
    const t = setTimeout(() => setD(v), ms);
    return () => clearTimeout(t);
  }, [v, ms]);
  return d;
}

/** Browser file → text (bounded). */
export function readTextFile(f: File, maxBytes: number): Promise<string> {
  if (f.size > maxBytes) return Promise.reject(new Error(`The file is larger than ${Math.round(maxBytes / 1024 / 1024)} MB.`));
  return f.text();
}
