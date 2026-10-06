// Workout library (search/filter/detail) and the structured workout editor.
import { useEffect, useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Button, Empty, ErrorText, Field, Modal, NumberInput, Segmented, Spinner } from "../ui";
import { WorkoutChart } from "../charts";
import { clock, minutes } from "../format";
import { useAction, useDebounced } from "../hooks";
import { IntensityTag } from "../plan";

export function Workouts() {
  const { go, dataVersion, bumpData, toast } = useApp();
  const list = useRpc("listWorkouts", { include_tests: true }, [dataVersion]);
  const [q, setQ] = useState("");
  const [cat, setCat] = useState("");
  const [dur, setDur] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const [editing, setEditing] = useState<J | null>(null);
  const { run } = useAction();
  const all: J[] = list.data?.workouts ?? [];
  const shown = useMemo(
    () =>
      all.filter((w) => {
        if (cat ? w.category !== cat : w.category === "test_fixture") return false;
        if (q && !`${w.name} ${w.description} ${w.category_label}`.toLowerCase().includes(q.toLowerCase())) return false;
        const m = w.duration_s / 60;
        if (dur === "short" && m > 45) return false;
        if (dur === "medium" && (m <= 45 || m > 75)) return false;
        if (dur === "long" && m <= 75) return false;
        return true;
      }),
    [all, q, cat, dur],
  );
  const current = sel ?? shown[0]?.id ?? null;
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Workouts</h1>
          <p className="sub">
            {all.filter((w) => w.category !== "test_fixture").length} workouts. Targets scale to your FTP; riders without reliable power can ride any workout by perceived effort.
          </p>
        </div>
        <Button kind="primary" onClick={() => setEditing(newWorkout())}>
          New workout
        </Button>
      </div>
      <div className="filters">
        <input type="text" placeholder="Search workouts" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search workouts" />
        <select value={cat} onChange={(e) => setCat(e.target.value)} aria-label="Category">
          <option value="">All categories</option>
          {(list.data?.categories ?? []).map((c: J) => (
            <option key={c.id} value={c.id}>
              {c.label}
            </option>
          ))}
        </select>
        <Segmented
          label="Duration"
          value={dur}
          onChange={setDur}
          options={[
            { value: "", label: "Any length" },
            { value: "short", label: "≤45 min" },
            { value: "medium", label: "46–75" },
            { value: "long", label: "75+" },
          ]}
        />
      </div>
      {list.loading && <Spinner />}
      <div className="grid-2">
        <section className="panel" style={{ padding: "8px 12px" }}>
          {shown.length === 0 && !list.loading && <Empty title="No workouts match" />}
          <div className="list" role="listbox" aria-label="Workouts">
            {shown.map((w) => (
              <div key={w.id} className="list-item" role="option" aria-selected={current === w.id} tabIndex={0} onClick={() => setSel(w.id)} onKeyDown={(e) => e.key === "Enter" && setSel(w.id)}>
                <div>
                  <div className="li-title">{w.name}</div>
                  <div className="li-meta">
                    <span>{w.category_label}</span>
                    <span>{minutes(w.duration_s)}</span>
                    <span>difficulty {w.difficulty}/5</span>
                    {!w.builtin && <span className="tag">custom</span>}
                    {!w.eligible && <span className="tag">not in plans at your level</span>}
                  </div>
                  <WorkoutChart timeline={w.profile} height={36} compact />
                </div>
                <IntensityTag v={w.intensity} />
              </div>
            ))}
          </div>
        </section>
        {current && (
          <WorkoutDetail
            id={current}
            onRide={() => go("ride", { mode: "erg", workout_id: current })}
            onRideMap={() => go("routes", { pick_for_workout: current })}
            onEdit={(w) => setEditing(w)}
            onDuplicate={() =>
              run("dup", async () => {
                const r = await rpc("duplicateWorkout", { id: current });
                bumpData();
                setSel(r.id);
                toast("Copy created. You can edit it now.");
              })
            }
            onDelete={() =>
              run("del", async () => {
                await rpc("deleteWorkout", { id: current });
                setSel(null);
                bumpData();
              })
            }
            version={dataVersion}
          />
        )}
      </div>
      {editing && (
        <WorkoutEditor
          initial={editing}
          onClose={(id) => {
            setEditing(null);
            if (id) {
              bumpData();
              setSel(id);
            }
          }}
        />
      )}
    </div>
  );
}

function WorkoutDetail(props: { id: string; version: number; onRide: () => void; onRideMap: () => void; onEdit: (w: J) => void; onDuplicate: () => void; onDelete: () => void }) {
  const { data, loading, error } = useRpc("getWorkout", { id: props.id }, [props.version]);
  const [confirmDel, setConfirmDel] = useState(false);
  const w: J = data;
  if (loading && !w) return <Spinner />;
  if (error) return <ErrorText>{error}</ErrorText>;
  if (!w) return null;
  const tl: number[][] = (w.timeline ?? []).map((s: J) => [s.start_s, s.dur_s, s.pct_start ?? 0, s.pct_end ?? 0]);
  return (
    <section className="panel">
      <div className="panel-head">
        <h2>{w.name}</h2>
        <div className="actions">
          <IntensityTag v={w.summary.intensity} />
          <span className="tag">{w.summary.category_label}</span>
          <span className="tag">{minutes(w.summary.duration_s)}</span>
        </div>
      </div>
      <WorkoutChart timeline={tl} height={150} />
      <p>{w.description}</p>
      <p className="muted">{w.purpose}</p>
      {!w.ftp && w.summary.uses_power && <p className="small muted">No FTP set: targets are shown as % FTP. Set an FTP in Settings, or ride by perceived effort.</p>}
      <div className="actions" style={{ margin: "12px 0" }}>
        <Button kind="primary" onClick={props.onRide}>
          Ride this workout
        </Button>
        <Button onClick={props.onRideMap}>Ride it on a map</Button>
        <Button onClick={props.onDuplicate}>Duplicate</Button>
        {!w.builtin && <Button onClick={() => props.onEdit(w)}>Edit</Button>}
        {!w.builtin && (
          <Button kind="danger" onClick={() => setConfirmDel(true)}>
            Delete
          </Button>
        )}
      </div>
      <table className="table">
        <thead>
          <tr>
            <th>Step</th>
            <th>Time</th>
            <th className="num">Duration</th>
            <th className="num">Target</th>
            <th>Cue</th>
          </tr>
        </thead>
        <tbody>
          {(w.timeline ?? []).map((s: J) => (
            <tr key={s.index}>
              <td>{s.label}</td>
              <td>{clock(s.start_s)}</td>
              <td className="num">{clock(s.dur_s)}</td>
              <td className="num">{targetText(s)}</td>
              <td className="small muted">
                {s.cadence && `${s.cadence.lo}–${s.cadence.hi} rpm. `}
                {s.text}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {confirmDel && (
        <Modal title="Delete workout?" onClose={() => setConfirmDel(false)}>
          <p>This removes “{w.name}” from your library. Past rides keep their copy.</p>
          <div className="actions">
            <Button
              kind="danger"
              onClick={() => {
                setConfirmDel(false);
                props.onDelete();
              }}
            >
              Delete
            </Button>
            <Button onClick={() => setConfirmDel(false)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </section>
  );
}

function targetText(s: J): string {
  const t = s.target;
  if (t.basis === "rpe") return `RPE ${t.value}`;
  const w = s.watts_start != null ? (t.end != null ? `${Math.round(s.watts_start)}→${Math.round(s.watts_end)} W` : `${Math.round(s.watts_start)} W`) : "";
  const p = t.basis === "ftp_pct" ? (t.end != null ? `${t.value}→${t.end}%` : `${t.value}%`) : "";
  return [w, p].filter(Boolean).join(" · ");
}

// ------------------------------------------------------------ editor

function newWorkout(): J {
  return {
    id: "new",
    name: "My workout",
    category: "endurance",
    difficulty: 2,
    description: "",
    purpose: "",
    blocks: [
      { count: 1, steps: [{ kind: "warmup", dur_s: 600, target: { basis: "ftp_pct", value: 45, end: 65 }, cadence: null, text: null }] },
      { count: 3, steps: [
        { kind: "work", dur_s: 300, target: { basis: "ftp_pct", value: 85, end: null }, cadence: null, text: null },
        { kind: "rest", dur_s: 120, target: { basis: "ftp_pct", value: 55, end: null }, cadence: null, text: null },
      ] },
      { count: 1, steps: [{ kind: "cooldown", dur_s: 300, target: { basis: "ftp_pct", value: 60, end: 45 }, cadence: null, text: null }] },
    ],
  };
}

const KINDS = [
  { v: "warmup", t: "Warm-up" },
  { v: "steady", t: "Steady" },
  { v: "work", t: "Interval" },
  { v: "rest", t: "Recovery" },
  { v: "cooldown", t: "Cool-down" },
];

function parseClock(s: string): number | null {
  const parts = s.trim().split(":").map((x) => Number(x));
  if (parts.some((x) => !isFinite(x) || x < 0)) return null;
  if (parts.length === 1) return Math.round(parts[0] * 60);
  if (parts.length === 2) return Math.round(parts[0] * 60 + parts[1]);
  if (parts.length === 3) return Math.round(parts[0] * 3600 + parts[1] * 60 + parts[2]);
  return null;
}

function WorkoutEditor({ initial, onClose }: { initial: J; onClose: (savedId?: string) => void }) {
  const [w, setW] = useState<J>(() => JSON.parse(JSON.stringify(initial)));
  const [check, setCheck] = useState<J | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const { run, busy } = useAction();
  const dw = useDebounced(w, 300);
  useEffect(() => {
    rpc("validateWorkout", { workout: dw })
      .then((r) => (setCheck(r), setErr(null)))
      .catch((e) => (setCheck(null), setErr(e.message)));
  }, [dw]);
  const set = (k: string, v: unknown) => setW((x: J) => ({ ...x, [k]: v }));
  const setBlock = (bi: number, b: J) => setW((x: J) => ({ ...x, blocks: x.blocks.map((o: J, i: number) => (i === bi ? b : o)) }));
  const setStep = (bi: number, si: number, st: J) => setBlock(bi, { ...w.blocks[bi], steps: w.blocks[bi].steps.map((o: J, i: number) => (i === si ? st : o)) });
  const issues: J[] = check?.issues ?? [];
  const preview = useMemo(() => {
    const tl: number[][] = [];
    let t = 0;
    for (const b of w.blocks)
      for (let r = 0; r < Math.min(b.count || 0, 30); r++)
        for (const s of b.steps) {
          const pctOf = (v: number) => (s.target.basis === "rpe" ? [40, 40, 50, 62, 72, 82, 90, 98, 108, 115, 150][Math.round(v)] ?? 60 : s.target.basis === "watts" ? 0 : v);
          tl.push([t, s.dur_s || 0, pctOf(s.target.value), pctOf(s.target.end ?? s.target.value)]);
          t += s.dur_s || 0;
        }
    return tl;
  }, [w.blocks]);
  return (
    <Modal
      wide
      title={initial.id === "new" ? "New workout" : `Edit: ${initial.name}`}
      onClose={() => onClose()}
      footer={
        <>
          <span className="muted small" style={{ marginRight: "auto" }}>
            {check ? `${minutes(check.duration_s)} · ${check.intensity}` : ""} {issues.length ? `· ${issues.length} problem(s)` : check ? "· valid" : ""}
          </span>
          <Button onClick={() => onClose()}>Cancel</Button>
          <Button
            kind="primary"
            disabled={!!busy || issues.length > 0}
            onClick={() =>
              run("save", async () => {
                const r = await rpc("saveWorkout", { workout: w });
                onClose(r.id);
              })
            }
          >
            Save
          </Button>
        </>
      }
    >
      <div className="cols">
        <Field label="Name">
          <input type="text" value={w.name} maxLength={80} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label="Category">
          <select value={w.category} onChange={(e) => set("category", e.target.value)}>
            {["recovery", "endurance", "tempo", "sweet_spot", "threshold", "over_under", "vo2", "anaerobic", "cadence", "climbing", "assessment"].map((c) => (
              <option key={c} value={c}>
                {c.replace("_", " ")}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Difficulty (1–5)">
          <NumberInput value={w.difficulty} min={1} max={5} onChange={(v) => set("difficulty", v ?? 1)} />
        </Field>
        <Field label="Description" wide>
          <textarea value={w.description} maxLength={800} onChange={(e) => set("description", e.target.value)} />
        </Field>
        <Field label="Purpose" wide>
          <input type="text" value={w.purpose} maxLength={400} onChange={(e) => set("purpose", e.target.value)} />
        </Field>
      </div>
      <div style={{ margin: "14px 0" }}>
        <WorkoutChart timeline={preview} height={90} />
      </div>
      <div className="blocks">
        {w.blocks.map((b: J, bi: number) => (
          <div className="block" key={bi}>
            <div className="block-head">
              <b>Block {bi + 1}</b>
              <span className="muted">repeat</span>
              <NumberInput value={b.count} min={1} max={30} suffix="×" onChange={(v) => setBlock(bi, { ...b, count: v ?? 1 })} />
              <span style={{ flex: 1 }} />
              <Button kind="quiet" onClick={() => setBlock(bi, { ...b, steps: [...b.steps, { kind: "steady", dur_s: 300, target: { basis: "ftp_pct", value: 65, end: null }, cadence: null, text: null }] })}>
                + Step
              </Button>
              <Button kind="quiet" disabled={w.blocks.length <= 1} onClick={() => set("blocks", w.blocks.filter((_: J, i: number) => i !== bi))}>
                Remove block
              </Button>
            </div>
            {b.steps.map((s: J, si: number) => (
              <div className="step-row" key={si}>
                <select aria-label="Step type" value={s.kind} onChange={(e) => setStep(bi, si, { ...s, kind: e.target.value })}>
                  {KINDS.map((k) => (
                    <option key={k.v} value={k.v}>
                      {k.t}
                    </option>
                  ))}
                </select>
                <input
                  type="text"
                  aria-label="Duration (m:ss)"
                  defaultValue={clock(s.dur_s)}
                  onBlur={(e) => {
                    const v = parseClock(e.target.value);
                    if (v != null) setStep(bi, si, { ...s, dur_s: v });
                    e.target.value = clock(v ?? s.dur_s);
                  }}
                />
                <select
                  aria-label="Target type"
                  value={s.target.basis}
                  onChange={(e) => {
                    const basis = e.target.value;
                    const value = basis === "rpe" ? 5 : basis === "watts" ? 150 : 65;
                    setStep(bi, si, { ...s, target: { basis, value, end: null } });
                  }}
                >
                  <option value="ftp_pct">% FTP</option>
                  <option value="watts">Watts</option>
                  <option value="rpe">Effort (RPE)</option>
                </select>
                <input type="number" aria-label="Target" value={s.target.value} onChange={(e) => setStep(bi, si, { ...s, target: { ...s.target, value: Number(e.target.value) } })} />
                <input
                  type="number"
                  aria-label="Ramp to"
                  placeholder="ramp to"
                  disabled={s.target.basis === "rpe"}
                  value={s.target.end ?? ""}
                  onChange={(e) => setStep(bi, si, { ...s, target: { ...s.target, end: e.target.value === "" ? null : Number(e.target.value) } })}
                />
                <input
                  type="text"
                  aria-label="Cadence cue (e.g. 85-95)"
                  placeholder="rpm cue"
                  defaultValue={s.cadence ? `${s.cadence.lo}-${s.cadence.hi}` : ""}
                  onBlur={(e) => {
                    const m = e.target.value.match(/^\s*(\d+)\s*[-–]\s*(\d+)\s*$/);
                    setStep(bi, si, { ...s, cadence: m ? { lo: Number(m[1]), hi: Number(m[2]) } : null });
                  }}
                />
                <input type="text" aria-label="Instruction" placeholder="instruction (optional)" maxLength={200} value={s.text ?? ""} onChange={(e) => setStep(bi, si, { ...s, text: e.target.value || null })} />
                <Button kind="quiet" disabled={b.steps.length <= 1} onClick={() => setBlock(bi, { ...b, steps: b.steps.filter((_: J, i: number) => i !== si) })}>
                  ✕
                </Button>
              </div>
            ))}
          </div>
        ))}
      </div>
      <div className="actions" style={{ marginTop: 10 }}>
        <Button onClick={() => set("blocks", [...w.blocks, { count: 1, steps: [{ kind: "steady", dur_s: 600, target: { basis: "ftp_pct", value: 65, end: null }, cadence: null, text: null }] }])}>+ Block</Button>
      </div>
      <ErrorText>{err}</ErrorText>
      {issues.length > 0 && (
        <ul className="issues">
          {issues.map((i, k) => (
            <li key={k}>
              <span className="error-text">{i.message}</span> <span className="small muted">({i.path})</span>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
