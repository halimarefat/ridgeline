// Routes: list, GPX import, map route builder (routing provider), profile
// preview with grade distribution, elevation quality and corrections.
import { useRef, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Bars, ElevationChart } from "../charts";
import { MapView } from "../map";
import { Button, Empty, ErrorText, Field, Modal, NumberInput, Spinner, Stat, Toggle } from "../ui";
import { dist, distUnit, distValue, elev, GRADE_LABELS, KM_PER_MI, pct } from "../format";
import { readTextFile, useAction, useJob } from "../hooks";

const SOURCE_LABEL: Record<string, string> = {
  imported: "Elevation from the GPX file",
  dem: "Elevation estimated from a terrain model",
  synthetic: "Synthetic demo profile",
  none: "No elevation yet",
};

export function Routes() {
  const { dataVersion, bumpData, screen, toast } = useApp();
  const list = useRpc("listRoutes", {}, [dataVersion]);
  const [sel, setSel] = useState<string | null>(null);
  const [building, setBuilding] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const { run, busy } = useAction();
  const routes: J[] = list.data ?? [];
  const current = sel ?? routes[0]?.id ?? null;
  const pickFor = (screen.params as J)?.pick_for_workout as string | undefined;
  const importFile = (f: File) =>
    run("import", async () => {
      const text = await readTextFile(f, 20 * 1024 * 1024);
      const r = await rpc("importGpx", { text, name: f.name.replace(/\.gpx$/i, "") });
      bumpData();
      setSel(r.id);
      const notes = [];
      if (r.dropped_points) notes.push(`${r.dropped_points} invalid point(s) skipped`);
      if (r.dropped_elevations) notes.push(`${r.dropped_elevations} implausible elevation value(s) ignored`);
      toast(`Route imported${notes.length ? ` (${notes.join(", ")})` : ""}.${r.needs_elevation ? " It needs elevation data before a free ride." : ""}`);
    });
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>Routes</h1>
          <p className="sub">Import a GPX file or build a route on cycling roads. The trainer follows the processed elevation profile.</p>
        </div>
        <div className="actions">
          <input ref={fileRef} type="file" accept=".gpx,application/gpx+xml,application/xml,text/xml" hidden onChange={(e) => e.target.files?.[0] && importFile(e.target.files[0])} />
          <Button onClick={() => fileRef.current?.click()} disabled={!!busy}>
            Import GPX
          </Button>
          <Button kind="primary" onClick={() => setBuilding(true)}>
            Build a route
          </Button>
        </div>
      </div>
      {pickFor && (
        <div className="banner banner-info" style={{ marginBottom: 16 }}>
          <p>Choose a route to ride your workout on. The map advances with you; the workout keeps control of the trainer.</p>
        </div>
      )}
      {list.loading && <Spinner />}
      <div className="grid-2" style={{ gridTemplateColumns: "minmax(0, 1fr) minmax(0, 2fr)" }}>
        <section className="panel" style={{ padding: "8px 12px" }}>
          {routes.length === 0 && !list.loading && <Empty title="No routes yet" />}
          <div className="list" role="listbox" aria-label="Routes">
            {routes.map((r) => (
              <div key={r.id} className="list-item" role="option" aria-selected={current === r.id} tabIndex={0} onClick={() => setSel(r.id)} onKeyDown={(e) => e.key === "Enter" && setSel(r.id)}>
                <div>
                  <div className="li-title">{r.name}</div>
                  <div className="li-meta">
                    <RouteMeta r={r} />
                  </div>
                </div>
                {r.summary?.simulation_ready ? <span className="tag tag-easy">ready</span> : <span className="tag tag-maximal">needs elevation</span>}
              </div>
            ))}
          </div>
        </section>
        {current && <RouteDetail id={current} version={dataVersion} pickFor={pickFor} onChanged={(id) => (bumpData(), id !== undefined && setSel(id))} />}
      </div>
      {building && (
        <RouteBuilder
          onClose={(id) => {
            setBuilding(false);
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

function RouteMeta({ r }: { r: J }) {
  const { units } = useApp();
  const s = r.summary;
  return (
    <>
      <span>{dist(s?.total_m, units)}</span>
      <span>{elev(s?.ascent_m, units)} up</span>
      <span>{r.bundled ? "demo route" : r.kind === "built" ? "built" : r.kind === "reversed" ? "reversed" : "GPX"}</span>
    </>
  );
}

function RouteDetail({ id, version, pickFor, onChanged }: { id: string; version: number; pickFor?: string; onChanged: (newId?: string | null) => void }) {
  const { units, go, toast } = useApp();
  const { data, loading, error, reload } = useRpc("getRoute", { id, max_points: 2500 }, [id, version]);
  const settings = useRpc("getSettings", {});
  const workouts = useRpc("listWorkouts", {});
  const { run, busy } = useAction();
  const job = useJob();
  const [corr, setCorr] = useState<{ from: number | null; to: number | null; note: string } | null>(null);
  const [rename, setRename] = useState<string | null>(null);
  const [withWorkout, setWithWorkout] = useState(pickFor ?? "");
  const [confirmDel, setConfirmDel] = useState(false);
  const r: J = data;
  if (loading && !r) return <Spinner />;
  if (error) return <ErrorText>{error}</ErrorText>;
  if (!r) return null;
  const s = r.summary;
  const synthetic = r.elevation?.kind === "synthetic";
  const hist: J[] = s.histogram ?? [];
  const total = hist.reduce((a, h) => a + h[1], 0) || 1;
  const after = () => (reload(), onChanged(undefined));
  return (
    <section className="panel">
      <div className="panel-head">
        <h2>{r.name}</h2>
        <div className="actions">
          {r.bundled && <span className="tag tag-sign">demo</span>}
          <span className="tag">{SOURCE_LABEL[r.elevation?.kind] ?? r.elevation?.kind}</span>
        </div>
      </div>
      <div className="metric-row">
        <Stat label="Distance" value={dist(s.total_m, units)} />
        <Stat label="Climbing" value={elev(s.ascent_m, units)} note={`descent ${elev(s.descent_m, units)}`} />
        <Stat label="Steepest" value={pct(s.max_grade)} note={`down ${pct(s.min_grade)}`} />
        <Stat label="Elevation" value={s.ele_min != null ? `${elev(s.ele_min, units)}–${elev(s.ele_max, units)}` : "—"} />
        <Stat label="Data" value={`${Math.round(s.coverage_pct)}%`} note="points with elevation" />
      </div>
      {!s.simulation_ready && (
        <div className="banner banner-warn">
          <p>{s.blocked_reason}</p>
          {!r.bundled && settings.data?.settings?.providers?.elevation_enabled && (
            <Button kind="primary" disabled={!!job.progress} onClick={async () => (await job.start("fetchElevation", { id })) && after()}>
              Fetch elevation
            </Button>
          )}
          <Button onClick={() => run("flat", () => rpc("setFlatFallback", { id, on: true }).then(after))}>Use flat fallback</Button>
        </div>
      )}
      {job.progress && (
        <p>
          <Spinner label={job.progress} />{" "}
          <Button kind="quiet" onClick={job.cancel}>
            Cancel
          </Button>
        </p>
      )}
      <div className="grid-2" style={{ gridTemplateColumns: "1fr 1fr" }}>
        <MapView line={r.display.line} styleUrl={settings.data?.settings?.map?.style_url ?? ""} enabled={!!settings.data?.settings?.map?.enabled && !synthetic} synthetic={synthetic} height={260} />
        <div>
          <ElevationChart chart={r.display.chart} units={units} height={190} flags={s.flags} />
          <div className="grade-key" aria-label="Gradient colours">
            {GRADE_LABELS.map((l, i) => (
              <span key={l}>
                <i className={`grade-${i}`} style={{ background: `var(--g${i})` }} />
                {l}
              </span>
            ))}
          </div>
        </div>
      </div>
      <div className="grid-2" style={{ marginTop: 16, gridTemplateColumns: "1fr 1fr" }}>
        <div>
          <h3>Grade distribution</h3>
          <Bars items={hist.map((h, i) => ({ label: h[0], value: h[1], cls: `bar-g${Math.min(6, Math.max(0, i - 1))}`, text: `${Math.round((100 * h[1]) / total)}%` }))} />
        </div>
        <div>
          <h3>Elevation quality</h3>
          <p className="small">
            {SOURCE_LABEL[r.elevation?.kind]}
            {r.elevation?.dataset && ` — ${r.elevation.dataset}`}
            {r.elevation?.resolution_m ? ` (~${Math.round(r.elevation.resolution_m)} m resolution)` : ""}. Profile resampled every 10 m and smoothed; {s.spikes_removed} spike(s) and {s.duplicates_removed}{" "}
            duplicate point(s) removed; {s.segments} segment(s).
          </p>
          {(s.flags ?? []).length > 0 && (
            <ul className="flags">
              {(s.flags as J[]).slice(0, 12).map((f, i) => (
                <li key={i} className="small">
                  <b>{f.kind}</b>
                  {f.to_m > f.from_m && ` at ${distValue(f.from_m, units).toFixed(2)}–${distValue(f.to_m, units).toFixed(2)} ${distUnit(units)}`}: {f.message}
                </li>
              ))}
            </ul>
          )}
          {(r.corrections ?? []).length > 0 && <p className="small">{r.corrections.length} curated correction(s) applied.</p>}
          {r.elevation?.attribution && <p className="small muted">{r.elevation.attribution}</p>}
          {r.attribution && <p className="small muted">{r.attribution}</p>}
        </div>
      </div>
      <div className="actions" style={{ marginTop: 16 }}>
        <Button kind="primary" disabled={!s.simulation_ready} onClick={() => go("ride", { mode: "free_ride", route_id: id })}>
          Free ride this route
        </Button>
        <select value={withWorkout} onChange={(e) => setWithWorkout(e.target.value)} aria-label="Workout to ride on this route" style={{ width: "auto" }}>
          <option value="">Ride a workout on it…</option>
          {(workouts.data?.workouts ?? []).map((w: J) => (
            <option key={w.id} value={w.id}>
              {w.name}
            </option>
          ))}
        </select>
        {withWorkout && (
          <Button kind={pickFor ? "primary" : "secondary"} onClick={() => go("ride", { mode: "erg", workout_id: withWorkout, route_id: id })}>
            Ride workout on this route
          </Button>
        )}
      </div>
      <div className="actions" style={{ marginTop: 10 }}>
        {!r.bundled && <Button onClick={() => setCorr({ from: null, to: null, note: "" })}>Add correction</Button>}
        {!r.bundled && (r.corrections ?? []).length > 0 && <Button onClick={() => run("clear", () => rpc("clearCorrections", { id }).then(after))}>Clear corrections</Button>}
        {!r.bundled && s.flat_fallback && <Button onClick={() => run("flat", () => rpc("setFlatFallback", { id, on: false }).then(after))}>Turn off flat fallback</Button>}
        <Button
          onClick={() =>
            run("rev", async () => {
              const x = await rpc("reverseRoute", { id });
              onChanged(x.id);
              toast("Reversed copy created: the gradient is recomputed in the new direction.");
            })
          }
        >
          Reverse
        </Button>
        {!r.bundled && <Button onClick={() => setRename(r.name)}>Rename</Button>}
        <Button onClick={() => run("gpx", async () => toast(`GPX saved: ${(await rpc("exportRouteGpx", { id })).path}`))}>Export GPX</Button>
        {!r.bundled && (
          <Button kind="danger" onClick={() => setConfirmDel(true)}>
            Delete
          </Button>
        )}
      </div>
      {corr && (
        <Modal title="Curated elevation correction" onClose={() => setCorr(null)}>
          <p className="muted">
            Terrain models often show false dips at bridges or humps over tunnels. A correction replaces the elevation between two points with a straight line. Look at the chart to find the
            section.
          </p>
          <div className="cols">
            <Field label={`From (${distUnit(units)})`}>
              <NumberInput value={corr.from} step={0.01} min={0} onChange={(v) => setCorr({ ...corr, from: v })} />
            </Field>
            <Field label={`To (${distUnit(units)})`}>
              <NumberInput value={corr.to} step={0.01} min={0} onChange={(v) => setCorr({ ...corr, to: v })} />
            </Field>
            <Field label="Note" wide>
              <input type="text" value={corr.note} maxLength={200} placeholder="e.g. bridge over the river" onChange={(e) => setCorr({ ...corr, note: e.target.value })} />
            </Field>
          </div>
          <div className="actions">
            <Button
              kind="primary"
              disabled={corr.from == null || corr.to == null}
              onClick={() =>
                run("corr", async () => {
                  const f = units === "imperial" ? KM_PER_MI * 1000 : 1000;
                  await rpc("addCorrection", { id, from_m: corr.from! * f, to_m: corr.to! * f, note: corr.note });
                  setCorr(null);
                  after();
                })
              }
            >
              Apply
            </Button>
            <Button onClick={() => setCorr(null)}>Cancel</Button>
          </div>
        </Modal>
      )}
      {rename != null && (
        <Modal title="Rename route" onClose={() => setRename(null)}>
          <Field label="Name">
            <input type="text" value={rename} maxLength={100} onChange={(e) => setRename(e.target.value)} />
          </Field>
          <div className="actions" style={{ marginTop: 12 }}>
            <Button kind="primary" disabled={!!busy} onClick={() => run("rename", () => rpc("renameRoute", { id, name: rename }).then(() => (setRename(null), after())))}>
              Save
            </Button>
          </div>
        </Modal>
      )}
      {confirmDel && (
        <Modal title="Delete route?" onClose={() => setConfirmDel(false)}>
          <p>“{r.name}” and its cached profile will be removed. Rides you already did on it are kept.</p>
          <div className="actions">
            <Button kind="danger" onClick={() => run("del", () => rpc("deleteRoute", { id }).then(() => (setConfirmDel(false), onChanged(null))))}>
              Delete
            </Button>
            <Button onClick={() => setConfirmDel(false)}>Cancel</Button>
          </div>
        </Modal>
      )}
    </section>
  );
}

function RouteBuilder({ onClose }: { onClose: (id?: string) => void }) {
  const settings = useRpc("getSettings", {});
  const [wps, setWps] = useState<[number, number][]>([]);
  const [name, setName] = useState("My route");
  const [fetchEle, setFetchEle] = useState(true);
  const [result, setResult] = useState<J | null>(null);
  const job = useJob();
  const st = settings.data?.settings;
  const disabled = st && (!st.providers?.routing_enabled || !st.map?.enabled);
  return (
    <Modal wide title="Build a route" onClose={() => onClose(result?.id)}>
      {disabled && <ErrorText>The route builder needs the background map and the routing service, which are turned off in Settings.</ErrorText>}
      <p className="muted small">
        Click the map to place the start, up to 23 points in between, and the finish. The routing service finds a path on roads suitable for cycling between them — not straight lines.
        Your points are sent to the routing service ({st?.providers?.routing_url}); nothing else is.
      </p>
      <MapView line={[]} waypoints={wps} styleUrl={st?.map?.style_url ?? ""} enabled={!!st?.map?.enabled} height={380} onMapClick={(lat, lon) => wps.length < 25 && setWps([...wps, [lat, lon]])} />
      <div className="cols" style={{ marginTop: 12 }}>
        <Field label="Route name">
          <input type="text" value={name} maxLength={100} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Toggle checked={fetchEle} onChange={setFetchEle} label="Fetch elevation for the route" hint="From Open-Meteo (Copernicus 90 m terrain model). Estimated, not surveyed." />
      </div>
      <p className="small">
        {wps.length} point(s).{" "}
        <Button kind="quiet" onClick={() => setWps(wps.slice(0, -1))} disabled={!wps.length}>
          Undo last point
        </Button>
        <Button kind="quiet" onClick={() => setWps([])} disabled={!wps.length}>
          Clear
        </Button>
      </p>
      {job.progress && (
        <p>
          <Spinner label={job.progress} />{" "}
          <Button kind="quiet" onClick={job.cancel}>
            Cancel
          </Button>
        </p>
      )}
      {result && (
        <div className="banner banner-info">
          <p>
            Route saved ({(result.distance_m / 1000).toFixed(1)} km). {result.warning}
          </p>
          <Button kind="primary" onClick={() => onClose(result.id)}>
            Open route
          </Button>
        </div>
      )}
      <div className="actions" style={{ marginTop: 12 }}>
        <Button
          kind="primary"
          disabled={wps.length < 2 || !!job.progress || !!disabled}
          onClick={async () => {
            const r = await job.start("buildRoute", { waypoints: wps, name, fetch_elevation: fetchEle });
            if (r) setResult(r);
          }}
        >
          Find route
        </Button>
        <Button onClick={() => onClose(result?.id)}>Close</Button>
      </div>
    </Modal>
  );
}
