// Small shared UI building blocks.
import { useEffect, useRef, type ReactNode } from "react";

export function Button(props: {
  children: ReactNode;
  onClick?: () => void;
  kind?: "primary" | "secondary" | "danger" | "quiet";
  disabled?: boolean;
  title?: string;
  big?: boolean;
  type?: "button" | "submit";
  kbd?: string;
}) {
  const cls = ["btn", `btn-${props.kind ?? "secondary"}`, props.big ? "btn-big" : ""].join(" ");
  return (
    <button type={props.type ?? "button"} className={cls} onClick={props.onClick} disabled={props.disabled} title={props.title}>
      {props.children}
      {props.kbd && <kbd>{props.kbd}</kbd>}
    </button>
  );
}

export function Field(props: { label: string; hint?: string; children: ReactNode; wide?: boolean }) {
  return (
    <label className={`field ${props.wide ? "field-wide" : ""}`}>
      <span className="field-label">{props.label}</span>
      {props.children}
      {props.hint && <span className="field-hint">{props.hint}</span>}
    </label>
  );
}

export function NumberInput(props: { value: number | null | undefined; onChange: (v: number | null) => void; min?: number; max?: number; step?: number; suffix?: string; placeholder?: string }) {
  return (
    <span className="num-input">
      <input
        type="number"
        value={props.value ?? ""}
        min={props.min}
        max={props.max}
        step={props.step ?? 1}
        placeholder={props.placeholder}
        onChange={(e) => {
          const v = e.target.value === "" ? null : Number(e.target.value);
          props.onChange(v == null || isNaN(v) ? null : v);
        }}
      />
      {props.suffix && <span className="suffix">{props.suffix}</span>}
    </span>
  );
}

export function Segmented<T extends string>(props: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void; label?: string }) {
  return (
    <div className="segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map((o) => (
        <button key={o.value} type="button" role="radio" aria-checked={props.value === o.value} className={props.value === o.value ? "on" : ""} onClick={() => props.onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Toggle(props: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode; hint?: string }) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={props.checked} onChange={(e) => props.onChange(e.target.checked)} />
      <span className="toggle-track" aria-hidden="true" />
      <span className="toggle-text">
        {props.label}
        {props.hint && <span className="field-hint">{props.hint}</span>}
      </span>
    </label>
  );
}

export function Modal(props: { title: string; onClose: () => void; children: ReactNode; wide?: boolean; footer?: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      prev?.focus?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className={`modal ${props.wide ? "modal-wide" : ""}`} role="dialog" aria-modal="true" aria-label={props.title} tabIndex={-1} ref={ref}>
        <header className="modal-head">
          <h2>{props.title}</h2>
          <button className="icon-btn" onClick={props.onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="modal-body">{props.children}</div>
        {props.footer && <footer className="modal-foot">{props.footer}</footer>}
      </div>
    </div>
  );
}

/** Status with an icon and text: never colour alone. */
export function Status(props: { kind: "ok" | "warn" | "bad" | "idle" | "busy"; children: ReactNode }) {
  const icon = { ok: "●", warn: "▲", bad: "■", idle: "○", busy: "◌" }[props.kind];
  return (
    <span className={`status status-${props.kind}`}>
      <span aria-hidden="true" className="status-icon">
        {icon}
      </span>
      {props.children}
    </span>
  );
}

export function Empty(props: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty">
      <h3>{props.title}</h3>
      {props.children && <p>{props.children}</p>}
      {props.action}
    </div>
  );
}

export function Spinner(props: { label?: string }) {
  return (
    <span className="spinner" role="status">
      <span className="spinner-dot" aria-hidden="true" />
      {props.label ?? "Loading…"}
    </span>
  );
}

export function ErrorText(props: { children: ReactNode }) {
  if (!props.children) return null;
  return (
    <p className="error-text" role="alert">
      {props.children}
    </p>
  );
}

export function Stat(props: { label: string; value: ReactNode; unit?: string; note?: ReactNode }) {
  return (
    <div className="stat">
      <span className="stat-label">{props.label}</span>
      <span className="stat-value">
        {props.value}
        {props.unit && <span className="stat-unit">{props.unit}</span>}
      </span>
      {props.note && <span className="stat-note">{props.note}</span>}
    </div>
  );
}

export function Scale(props: { value: number; min: number; max: number; onChange: (v: number) => void; labels?: [string, string]; name: string }) {
  const opts = [];
  for (let i = props.min; i <= props.max; i++) opts.push(i);
  return (
    <div className="scale" role="radiogroup" aria-label={props.name}>
      {props.labels && <span className="scale-end">{props.labels[0]}</span>}
      {opts.map((i) => (
        <button key={i} type="button" role="radio" aria-checked={props.value === i} className={props.value === i ? "on" : ""} onClick={() => props.onChange(i)}>
          {i}
        </button>
      ))}
      {props.labels && <span className="scale-end">{props.labels[1]}</span>}
    </div>
  );
}

export function useKeys(handler: (e: KeyboardEvent) => void, deps: unknown[]) {
  useEffect(() => {
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
