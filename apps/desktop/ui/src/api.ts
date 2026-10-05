// Narrow command interface to the native service. In the desktop app this
// goes through Tauri IPC; in the developer server it is a local HTTP POST.
// The UI never talks to Bluetooth, storage or AI services directly.

/* eslint-disable @typescript-eslint/no-explicit-any */
export type J = any;

interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event?: { listen: (event: string, cb: (e: unknown) => void) => Promise<() => void> };
}

declare global {
  interface Window {
    __TAURI__?: TauriGlobal;
  }
}

export class RpcError extends Error {}

export const isDesktop = (): boolean => typeof window !== "undefined" && !!window.__TAURI__?.core;

export async function rpc<T = J>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  let text: string;
  if (isDesktop()) {
    text = (await window.__TAURI__!.core.invoke("rpc", { method, params: JSON.stringify(params) })) as string;
  } else {
    const r = await fetch("/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ method, params }),
    });
    text = await r.text();
  }
  let v: J;
  try {
    v = JSON.parse(text);
  } catch {
    throw new RpcError("The app sent an unreadable reply.");
  }
  if (!v.ok) throw new RpcError(String(v.error ?? "Something went wrong."));
  return v.result as T;
}

/** Poll a background job until it finishes; returns its result. */
export async function waitJob(id: string, onProgress?: (msg: string, done?: number, total?: number) => void): Promise<J> {
  for (;;) {
    const j = await rpc("getJob", { id });
    if (!j) throw new RpcError("The task disappeared.");
    if (j.state === "running") {
      onProgress?.(j.message || "Working…", j.progress?.[0], j.progress?.[1]);
      await new Promise((r) => setTimeout(r, 500));
      continue;
    }
    if (j.state === "done") return j.result;
    if (j.state === "cancelled") throw new RpcError("Cancelled.");
    throw new RpcError(j.error || "The task failed.");
  }
}

/** Run a command that may return {job_id}; waits for the job if so. */
export async function rpcMaybeJob(method: string, params: Record<string, unknown>, onProgress?: (msg: string) => void): Promise<J> {
  const r = await rpc(method, params);
  if (r && typeof r === "object" && "job_id" in r && r.job_id) return waitJob(r.job_id, onProgress);
  return r;
}

export async function onDesktopEvent(name: string, cb: () => void): Promise<() => void> {
  const ev = window.__TAURI__?.event;
  if (!ev) return () => {};
  return ev.listen(name, () => cb());
}

export async function exitApp(): Promise<void> {
  if (isDesktop()) await window.__TAURI__!.core.invoke("exit_app");
}
