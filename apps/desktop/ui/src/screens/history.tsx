// History: saved rides with summary, charts, laps, zones, events, feedback,
// FTP estimates from assessments, and FIT / CSV / GPX export.
import { useMemo, useState } from "react";
import { rpc, type J } from "../api";
import { useApp, useRpc } from "../state";
import { Bars, SeriesChart } from "../charts";
import { MapView } from "../map";
import { Button, Empty, ErrorText, Field, Modal, Spinner, Stat } from "../ui";
import { clock, dateTime, dist, minutes, round, ZONE_NAMES } from "../format";
import { useAction } from "../hooks";
import { ProposalModal } from "../plan";
import { FeedbackForm, SummaryStats } from "./ride";

const MODE_LABEL: Record<string, string> = { erg: "Workout (ERG)", free_ride: "Free ride", manual: "Manual", read_only: "Recording only" };

export function History() {
  const { screen, dataVersion, bumpData, units } = useApp();
  const list = useRpc("listActivities", {}, [dataVersion]);
  const [sel, setSel] = useState<string | null>((screen.params?.id as string) ?? null);
  const [q, setQ] = useState("");
  const [hideDemo, setHideDemo] = useState(false);
  const rows: J[] = useMemo(
    () =>
      (list.data ?? []).filter((a: J) => (!hideDemo || !a.meta.demo) && (!q || a.meta.title.toLowerCase().includes(q.toLowerCase()) || (a.meta.workout_name ?? "").toLowerCase().includes(q.toLowerCase()))),
    [list.data, q, hideDemo],
  );
  const current = sel ?? rows[0]?.meta.id ?? null;
  const totals = useMemo(() => {
    const last28 = Date.now() - 28 * 86400e3;
    const r = (list.data ?? []).filter((a: J) => !a.meta.demo && a.meta.start_utc >= last28 && a.summary);
    return { rides: r.length, time: r.reduce((s: number, a: J) => s + (a.summary.timer_s ?? 0), 0), kj: r.reduce((s: number, a: J) => s + (a.summary.work_kj ?? 0), 0) };
  }, [list.data]);
  return (
    <div>
      <div className="page-head">
        <div>
          <h1>History</h1>
          <p className="sub">
            Last 28 days: {totals.rides} ride(s), {minutes(totals.time)}, {Math.round(totals.kj)} kJ (demo rides excluded)
          </p>
        </div>
      </div>
      {list.loading && !list.data && <Spinner />}
      <ErrorText>{list.error}</ErrorText>
      {list.data && list.data.length === 0 && (
        <section className="panel">
          <Empty title="No rides yet">Rides are saved here automatically, every second, so even an interrupted ride can be recovered.</Empty>
        </section>
      )}
      {list.data && list.data.length > 0 && (
        <div className="grid-2" style={{ gridTemplateColumns: "minmax(0, 1fr) minmax(0, 2fr)" }}>
          <section className="panel" style={{ padding: "8px 12px" }}>
            <div className="filters">
              <input type="text" placeholder="Search" aria-label="Search rides" value={q} onChange={(e) => setQ(e.target.value)} />
              <label className="small">
                <input type="checkbox" checked={hideDemo} onChange={(e) => setHideDemo(e.target.checked)} /> Hide demo rides
              </label>
            </div>
            <div className="list" role="listbox" aria-label="Rides">
              {rows.map((a) => (
                <div
                  key={a.meta.id}
                  className="list-item"
                  role="option"
                  aria-selected={current === a.meta.id}
                  tabIndex={0}
                  onClick={() => setSel(a.meta.id)}
                  onKeyDown={(e) => e.key === "Enter" && setSel(a.meta.id)}
                >
                  <div>
                    <div className="li-title">
                      {a.meta.demo && <span className="demo-flag">DEMO</span>} {a.meta.title.replace(/^\[Demo\]\s*/, "")}
                    </div>
                    <div className="li-meta">
                      <span>{dateTime(a.meta.start_utc)}</span>
                      <span>{minutes(a.summary?.timer_s)}</span>
                      {a.summary?.distance_m != null && <span>{dist(a.summary.distance_m, units)}</span>}
                      {a.summary?.power?.avg != null && <span>{round(a.summary.power.avg)} W</span>}
                    </div>
                  </div>
                  {a.meta.status === "recording" ? <span className="tag tag-maximal">interrupted</span> : a.meta.status === "recovered" ? <span className="tag">recovered</span> : null}
                </div>
              ))}
            </div>
          </section>
          {current && <ActivityDetail id={current} version={dataVersion} onChanged={(gone) => (gone && setSel(null), bumpData())} />}
        </div>
      )}
    </div>
  );
}

function ActivityDetail({ id, version, onChanged }: { id: string; version: number; onChanged: (gone?: boolean) => void }) {
  const { units, toast, refreshBoot, boot } = useApp();
  const { data, loading, error, reload } = useRpc("getActivity", { id }, [id, version]);
  const { run, busy } = useAction();
  const [rename, setRename] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState(false);
  const [proposal, setProposal] = useState<string | null>(null);
  const [editFb, setEditFb] = useState(false);
  const a: J = data;
  if (loading && !a) return <Spinner />;
  if (error) return <ErrorText>{error}</ErrorText>;
  if (!a) return null;
  const m = a.meta;
  const s = a.summary;
  const recording = m.status === "recording";
  const zones: number[] = s?.time_in_zones_s ?? [];
  const zoneTotal = zones.reduce((x, y) => x + y, 0) || 1;
  const hasTarget = (a.series as J[]).some((r) => r[6] != null);
  const hasGrade = (a.series as J[]).some((r) => r[7] != null);
  const markers = (a.laps as J[]).slice(1).map((l) => l.start_active_s);
  const exp = (format: string) =>
    run(format, async () => {
      const r = await rpc("exportActivity", { id, format });
      toast(`${format.toUpperCase()} saved: ${r.path}`);
    });
  return (
    <section className="panel">
      <div className="panel-head">
        <div>
          <h2>
            {m.demo && <span className="demo-flag">DEMO</span>} {m.title.replace(/^\[Demo\]\s*/, "")}
          </h2>
          <p className="small muted">
            {dateTime(m.start_utc)} · {MODE_LABEL[m.mode] ?? m.mode}
            {m.workout_name && ` · ${m.workout_name}`}
            {m.route_name && ` · ${m.route_name}`}
            {m.ftp_w && ` · FTP ${Math.round(m.ftp_w)} W`}
          </p>
        </div>
      </div>
      {recording && (
        <div className="banner banner-warn">
          <p>This ride was interrupted (the app closed or crashed while recording). Recover it to keep the data saved so far.</p>
          <Button kind="primary" onClick={() => run("recover", () => rpc("recoverActivity", { id }).then(() => (onChanged(), refreshBoot())), "Ride recovered.")}>
            Recover
          </Button>
        </div>
      )}
      {a.corrupt_lines > 0 && <p className="small warn-text">{a.corrupt_lines} damaged record line(s) were skipped when loading this ride.</p>}
      {s && <SummaryStats sum={s} units={units} />}
      {s && (s.gaps > 0 || s.stale_power_s > 0 || s.control_lost_s > 0) && (
        <p className="small muted">
          Data quality: {s.gaps} gap(s) in recording, {s.stale_power_s} s without fresh power, {s.control_lost_s} s without trainer control.
        </p>
      )}
      {a.ftp_estimate && (
        <div className="banner banner-info">
          <p>
            This assessment suggests an FTP of about <b>{Math.round(a.ftp_estimate.watts)} W</b> ({a.ftp_estimate.method === "ramp_test" ? "75% of your best minute" : "95% of your 20-minute average"}). It's an estimate; accept it
            only if the test went well.
          </p>
          {!m.demo && (
            <Button kind="primary" onClick={() => run("ftp", () => rpc("acceptFtpEstimate", { activity_id: id }).then(() => refreshBoot()), "FTP updated (provisional). Future targets use it.")}>
              Use as my FTP
            </Button>
          )}
        </div>
      )}
      <h3>Power, heart rate and cadence</h3>
      <SeriesChart
        rows={a.series}
        height={170}
        markers={markers}
        series={[
          { index: 1, label: "Power", cls: "s-power", scale: "w" },
          ...(hasTarget ? [{ index: 6, label: "Target", cls: "s-target", scale: "w" }] : []),
          { index: 2, label: "Heart rate", cls: "s-hr" },
          { index: 3, label: "Cadence", cls: "s-cad" },
        ]}
      />
      {(hasGrade || (a.line ?? []).length > 1) && (
        <>
          <h3>Route</h3>
          <div className="grid-2" style={{ gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)" }}>
            <SeriesChart
              rows={a.series}
              height={150}
              series={[
                { index: 5, label: "Elevation", cls: "s-ele" },
                { index: 4, label: "Speed", cls: "s-speed" },
              ]}
            />
            {(a.line ?? []).length > 1 && <MapView line={a.line} styleUrl={boot?.settings?.map?.style_url ?? ""} enabled={!!boot?.settings?.map?.enabled && !m.virtual_route} synthetic={!!m.virtual_route} height={200} />}
          </div>
        </>
      )}
      <div className="grid-2" style={{ gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)", marginTop: 12 }}>
        <div>
          <h3>Time in power zones</h3>
          {zones.length ? (
            <Bars items={zones.map((z, i) => ({ label: s.zone_names?.[i] ?? ZONE_NAMES[i], value: z, cls: `bar-z${i}`, text: `${Math.round((100 * z) / zoneTotal)}%` }))} />
          ) : (
            <p className="small muted">Zones need an FTP and power data.</p>
          )}
        </div>
        <div>
          <h3>How it felt</h3>
          {a.feedback && !editFb ? (
            <div>
              <div className="metric-row">
                <Stat label="Effort (RPE)" value={a.feedback.rpe} unit="/10" />
                <Stat label="Fatigue" value={a.feedback.fatigue} unit="/5" />
                <Stat label="Enjoyment" value={a.feedback.enjoyment} unit="/5" />
              </div>
              {a.feedback.notes && <p className="small">{a.feedback.notes}</p>}
              <Button kind="quiet" onClick={() => setEditFb(true)}>
                Edit feedback
              </Button>
            </div>
          ) : recording ? (
            <p className="small muted">Recover the ride first.</p>
          ) : (
            <FeedbackForm activityId={id} initial={a.feedback} onProposal={setProposal} />
          )}
        </div>
      </div>
      {(s?.laps ?? []).length > 1 && (
        <>
          <h3>Laps and intervals</h3>
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Label</th>
                  <th className="num">Time</th>
                  <th className="num">Avg W</th>
                  <th className="num">Target</th>
                  <th className="num">Avg HR</th>
                  <th className="num">Cadence</th>
                </tr>
              </thead>
              <tbody>
                {(s.laps as J[]).map((l) => (
                  <tr key={l.index}>
                    <td>{l.index + 1}</td>
                    <td>{l.label}</td>
                    <td className="num">{clock(l.duration_s)}</td>
                    <td className="num">{round(l.power?.avg)}</td>
                    <td className="num">
                      {round(l.target_w)}
                      {l.target_error != null && <span className="small muted"> ({Math.round(l.target_error * 100)}% off)</span>}
                    </td>
                    <td className="num">{round(l.hr?.avg)}</td>
                    <td className="num">{round(l.cadence?.avg)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
      {(a.events ?? []).length > 0 && (
        <details style={{ marginTop: 12 }}>
          <summary>Ride events ({a.events.length})</summary>
          <div className="log">
            {(a.events as J[]).map((e, i) => (
              <div key={i}>
                {new Date(e.u).toLocaleTimeString()} — {e.kind}
                {e.detail ? `: ${e.detail}` : ""}
              </div>
            ))}
          </div>
        </details>
      )}
      <div className="actions" style={{ marginTop: 16 }}>
        <Button kind="primary" disabled={recording || !!busy} onClick={() => exp("fit")}>
          Export FIT
        </Button>
        <Button disabled={recording || !!busy} onClick={() => exp("csv")}>
          Export CSV
        </Button>
        {(a.line ?? []).length > 1 && (
          <Button disabled={recording || !!busy} onClick={() => exp("gpx")}>
            Export GPX
          </Button>
        )}
        <Button onClick={() => setRename(m.title.replace(/^\[Demo\]\s*/, ""))}>Rename</Button>
        <Button kind="danger" onClick={() => setConfirmDel(true)}>
          Delete
        </Button>
      </div>
      <p className="small muted">FIT files work with Garmin Connect, Strava, intervals.icu and most training logs. Distance is virtual (from the trainer or route simulation).</p>
      {rename != null && (
        <Modal title="Rename ride" onClose={() => setRename(null)}>
          <Field label="Title">
            <input type="text" value={rename} maxLength={120} onChange={(e) => setRename(e.target.value)} />
          </Field>
          <div className="actions" style={{ marginTop: 12 }}>
            <Button kind="primary" disabled={!rename.trim()} onClick={() => run("rename", () => rpc("renameActivity", { id, title: rename.trim() }).then(() => (setRename(null), reload(), onChanged())))}>
              Save
            </Button>
          </div>
        </Modal>
      )}
      {confirmDel && (
        <Modal title="Delete this ride?" onClose={() => setConfirmDel(false)}>
          <p>“{m.title}” and its recording will be permanently removed from this computer. Files you already exported are not affected.</p>
          <div className="actions">
            <Button kind="danger" onClick={() => run("del", () => rpc("deleteActivity", { id }).then(() => (setConfirmDel(false), onChanged(true))), "Ride deleted.")}>
              Delete
            </Button>
            <Button onClick={() => setConfirmDel(false)}>Cancel</Button>
          </div>
        </Modal>
      )}
      {proposal && <ProposalModal id={proposal} onClose={() => (setProposal(null), onChanged())} />}
    </section>
  );
}
