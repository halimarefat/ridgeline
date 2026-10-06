// SVG charts: workout power profiles (zone-coloured blocks), stage-style
// elevation profiles (grade-coloured), time series and histograms.
import { useCallback, useEffect, useRef, useState } from "react";
import { traceRuns } from "./chartdata";
import { clock, distValue, distUnit, gradeBucket, zoneOf, type Units } from "./format";

export function useWidth<T extends HTMLElement>(): [(el: T | null) => void, number] {
  // Callback ref: works even when the measured element mounts after the
  // first render (e.g. a chart that first shows an empty state).
  const [w, setW] = useState(600);
  const ro = useRef<ResizeObserver | null>(null);
  const ref = useCallback((el: T | null) => {
    ro.current?.disconnect();
    ro.current = null;
    if (!el) return;
    setW(Math.max(120, Math.floor(el.getBoundingClientRect().width)));
    const obs = new ResizeObserver((es) => {
      for (const e of es) setW(Math.max(120, Math.floor(e.contentRect.width)));
    });
    obs.observe(el);
    ro.current = obs;
  }, []);
  useEffect(() => () => ro.current?.disconnect(), []);
  return [ref, w];
}

/** timeline: [start_s, dur_s, pct_start, pct_end] */
export function WorkoutChart(props: { timeline: number[][]; height?: number; pos?: number | null; ftp?: number | null; compact?: boolean; label?: string; trace?: number[][] }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const h = props.height ?? 140;
  const tl = props.timeline ?? [];
  const total = tl.length ? tl[tl.length - 1][0] + tl[tl.length - 1][1] : 1;
  // The rider's power as % FTP (same scale as the profile), when FTP is known.
  const tracePct = props.ftp && props.trace?.length ? props.trace.map(([p, wts]) => [p, (wts / props.ftp!) * 100]) : [];
  const maxPct = Math.max(120, ...tl.map((s) => Math.max(s[2] || 0, s[3] || 0)), ...tracePct.map((t) => t[1])) * 1.05;
  const x = (t: number) => (t / total) * w;
  const y = (p: number) => h - (p / maxPct) * (h - 4);
  const ftpLine = y(100);
  return (
    <div className="chart" ref={ref} aria-label={props.label ?? "Workout profile"} role="img">
      <svg width={w} height={h}>
        {!props.compact && <line x1={0} x2={w} y1={ftpLine} y2={ftpLine} className="chart-ref" />}
        {tl.map((s, i) => {
          const [st, d, a, b] = s;
          const z = zoneOf(((a || 0) + (b || 0)) / 2);
          const pts = `${x(st)},${h} ${x(st)},${y(a || 0)} ${x(st + d)},${y(b || 0)} ${x(st + d)},${h}`;
          return <polygon key={i} points={pts} className={`zone-${z}`} />;
        })}
        {props.pos != null && (
          <>
            <rect x={0} y={0} width={x(props.pos)} height={h} className="chart-done" />
            <line x1={x(props.pos)} x2={x(props.pos)} y1={0} y2={h} className="chart-cursor" />
          </>
        )}
        {traceRuns(tracePct).map((run, i) => {
          const pts = run.map(([p, v]) => `${x(p).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
          return (
            <g key={`t${i}`}>
              <polyline points={pts} className="chart-power-halo" />
              <polyline points={pts} className="chart-power" />
            </g>
          );
        })}
      </svg>
      {tracePct.length > 0 && !props.compact && (
        <span className="chart-trace-legend">
          <span className="chart-trace-swatch" /> Your power
        </span>
      )}
      {!props.compact && (
        <span className="chart-ftp-label" style={{ top: ftpLine - 16 }}>
          FTP
        </span>
      )}
      {!props.compact && (
        <div className="chart-axis">
          <span>0:00</span>
          <span>{clock(total)}</span>
        </div>
      )}
    </div>
  );
}

/** chart rows: [s, ele, grade, seg] */
export function ElevationChart(props: { chart: (number | null)[][]; height?: number; pos?: number | null; units: Units; compact?: boolean; flags?: { from_m: number; to_m: number; kind: string }[] }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const h = props.height ?? 160;
  const rows = props.chart ?? [];
  if (rows.length < 2) return <div className="chart chart-empty">No elevation profile.</div>;
  const total = (rows[rows.length - 1][0] as number) || 1;
  const eles = rows.map((r) => r[1]).filter((e): e is number => e != null);
  const lo = eles.length ? Math.min(...eles) : 0;
  const hi = eles.length ? Math.max(...eles) : 1;
  const span = Math.max(hi - lo, 30);
  const pad = props.compact ? 2 : 14;
  const x = (s: number) => (s / total) * w;
  const y = (e: number) => h - pad - ((e - lo) / span) * (h - pad * 2 - 6);
  const polys = [];
  for (let i = 0; i + 1 < rows.length; i++) {
    const [s0, e0, g0, seg0] = rows[i];
    const [s1, e1, , seg1] = rows[i + 1];
    if (seg0 !== seg1) continue;
    if (e0 == null || e1 == null) {
      polys.push(<rect key={i} x={x(s0 as number)} y={pad} width={Math.max(1, x(s1 as number) - x(s0 as number))} height={h - pad * 2} className="grade-missing" />);
      continue;
    }
    const b = gradeBucket(g0);
    polys.push(<polygon key={i} points={`${x(s0 as number)},${h} ${x(s0 as number)},${y(e0)} ${x(s1 as number)},${y(e1)} ${x(s1 as number)},${h}`} className={`grade-${b}`} />);
  }
  const ticks = [];
  const km = distValue(total, props.units);
  const step = km > 80 ? 20 : km > 40 ? 10 : km > 15 ? 5 : km > 5 ? 2 : 1;
  for (let k = step; k < km; k += step) ticks.push(k);
  const perUnit = total / km;
  const posPt = props.pos != null ? rows.reduce((best, r) => (Math.abs((r[0] as number) - props.pos!) < Math.abs((best[0] as number) - props.pos!) ? r : best), rows[0]) : null;
  return (
    <div className="chart" ref={ref} role="img" aria-label="Elevation profile coloured by gradient">
      <svg width={w} height={h}>
        {polys}
        {(props.flags ?? [])
          .filter((f) => f.kind === "steep" || f.kind === "abrupt" || f.kind === "missing")
          .map((f, i) => (
            <rect key={`f${i}`} x={x(f.from_m)} y={0} width={Math.max(2, x(f.to_m) - x(f.from_m))} height={4} className={`flag-${f.kind}`} />
          ))}
        {!props.compact &&
          ticks.map((k) => (
            <g key={k}>
              <line x1={x(k * perUnit)} x2={x(k * perUnit)} y1={h - 6} y2={h} className="chart-tick" />
              <text x={x(k * perUnit)} y={h - 8} className="chart-text" textAnchor="middle">
                {k}
              </text>
            </g>
          ))}
        {posPt && posPt[1] != null && (
          <g>
            <line x1={x(props.pos!)} x2={x(props.pos!)} y1={0} y2={h} className="chart-cursor" />
            <circle cx={x(props.pos!)} cy={y(posPt[1] as number)} r={7} className="rider-dot" />
          </g>
        )}
      </svg>
      {!props.compact && (
        <div className="chart-axis">
          <span>
            {Math.round(lo)}–{Math.round(hi)} m
          </span>
          <span>{distUnit(props.units)}</span>
        </div>
      )}
    </div>
  );
}

/** Generic time series: rows [t, v1, v2, …]; series index → class & label. */
export function SeriesChart(props: { rows: (number | null)[][]; series: { index: number; label: string; cls: string; scale?: string }[]; height?: number; markers?: number[] }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const h = props.height ?? 160;
  const rows = props.rows ?? [];
  if (rows.length < 2) return <div className="chart chart-empty">Not enough data yet.</div>;
  const t0 = rows[0][0] as number;
  const t1 = (rows[rows.length - 1][0] as number) || t0 + 1;
  const x = (t: number) => ((t - t0) / Math.max(1, t1 - t0)) * w;
  return (
    <div className="chart" ref={ref}>
      <svg width={w} height={h} role="img" aria-label={props.series.map((s) => s.label).join(", ")}>
        {props.series.map((s) => {
          const vals = rows.map((r) => r[s.index]).filter((v): v is number => v != null);
          if (!vals.length) return null;
          // Series sharing a scale key (e.g. power and target) share one axis.
          const peers = props.series.filter((o) => (o.scale ?? o.index) === (s.scale ?? s.index));
          let top = 0;
          for (const o of peers) for (const r of rows) if (r[o.index] != null) top = Math.max(top, r[o.index] as number);
          const hi = top * 1.08 || 1;
          const y = (v: number) => h - 4 - (v / hi) * (h - 10);
          // Break the line at missing values (gaps stay visible).
          const paths: string[] = [];
          let cur = "";
          for (const r of rows) {
            const v = r[s.index];
            if (v == null) {
              if (cur) paths.push(cur);
              cur = "";
            } else {
              cur += `${cur ? "L" : "M"}${x(r[0] as number).toFixed(1)},${y(v).toFixed(1)}`;
            }
          }
          if (cur) paths.push(cur);
          return paths.map((d, i) => <path key={`${s.index}-${i}`} d={d} className={`series ${s.cls}`} />);
        })}
        {(props.markers ?? []).map((m, i) => (
          <line key={i} x1={x(m)} x2={x(m)} y1={0} y2={h} className="chart-marker" />
        ))}
      </svg>
      <div className="chart-legend">
        {props.series.map((s) => (
          <span key={s.index} className={`legend ${s.cls}`}>
            <span className="legend-swatch" aria-hidden="true" />
            {s.label}
          </span>
        ))}
        <span className="legend-time">{clock(t1 - t0)}</span>
      </div>
    </div>
  );
}

export function Bars(props: { items: { label: string; value: number; cls?: string; text?: string }[] }) {
  const max = Math.max(1, ...props.items.map((i) => i.value));
  return (
    <div className="bars">
      {props.items.map((i) => (
        <div className="bar-row" key={i.label}>
          <span className="bar-label">{i.label}</span>
          <span className="bar-track">
            <span className={`bar-fill ${i.cls ?? ""}`} style={{ width: `${(100 * i.value) / max}%` }} />
          </span>
          <span className="bar-value">{i.text ?? Math.round(i.value)}</span>
        </div>
      ))}
    </div>
  );
}
