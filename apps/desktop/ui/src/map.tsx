// Route map: MapLibre GL JS with OpenFreeMap vector tiles. If the map style
// or tiles can't load (offline, blocked), a locally drawn route line is shown
// instead — ride control never depends on the map.
import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { useEffect, useRef, useState } from "react";
import { useWidth } from "./charts";

export interface MapProps {
  /** [lon, lat, seg] */
  line: number[][];
  pos?: [number, number] | null;
  styleUrl: string;
  enabled: boolean;
  height?: number;
  waypoints?: [number, number][]; // [lat, lon]
  onMapClick?: (lat: number, lon: number) => void;
  follow?: boolean;
  synthetic?: boolean;
}

function unwrap(line: number[][]): number[][] {
  // Continuous longitudes across the antimeridian for drawing.
  const out: number[][] = [];
  let prev: number | null = null;
  for (const p of line) {
    let lon = p[0];
    if (prev != null) {
      while (lon - prev > 180) lon -= 360;
      while (lon - prev < -180) lon += 360;
    }
    out.push([lon, p[1], p[2] ?? 0]);
    prev = lon;
  }
  return out;
}

function geojson(line: number[][]) {
  const segs: number[][][] = [];
  for (const p of unwrap(line)) {
    const s = p[2] ?? 0;
    if (!segs[s]) segs[s] = [];
    segs[s].push([p[0], p[1]]);
  }
  return {
    type: "FeatureCollection" as const,
    features: segs.filter(Boolean).map((coords) => ({ type: "Feature" as const, properties: {}, geometry: { type: "LineString" as const, coordinates: coords } })),
  };
}

export function MapView(props: MapProps) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<maplibregl.Map | null>(null);
  const marker = useRef<maplibregl.Marker | null>(null);
  const wpMarkers = useRef<maplibregl.Marker[]>([]);
  const [failed, setFailed] = useState<string | null>(props.enabled ? null : "Background map turned off in Settings.");
  const clickRef = useRef(props.onMapClick);
  clickRef.current = props.onMapClick;
  const h = props.height ?? 320;

  useEffect(() => {
    if (!props.enabled || !el.current) return;
    if (typeof navigator !== "undefined" && navigator.onLine === false) {
      setFailed("Offline: showing the route line only.");
      return;
    }
    let m: maplibregl.Map;
    try {
      m = new maplibregl.Map({ container: el.current, style: props.styleUrl, attributionControl: { compact: true }, center: [0, 0], zoom: 1, cooperativeGestures: false });
    } catch (e) {
      setFailed(`Map unavailable (${(e as Error).message}). Showing the route line only.`);
      return;
    }
    map.current = m;
    m.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    let loaded = false;
    const timer = window.setTimeout(() => {
      if (!loaded) setFailed("The map is taking too long to load. Showing the route line only.");
    }, 12000);
    m.on("load", () => {
      loaded = true;
      clearTimeout(timer);
      m.addSource("route", { type: "geojson", data: geojson(props.line) });
      m.addLayer({ id: "route-casing", type: "line", source: "route", paint: { "line-color": "#161c22", "line-width": 8 }, layout: { "line-cap": "round", "line-join": "round" } });
      m.addLayer({ id: "route-line", type: "line", source: "route", paint: { "line-color": "#ffc933", "line-width": 4.5 }, layout: { "line-cap": "round", "line-join": "round" } });
      fit(m, props.line);
    });
    m.on("error", (ev) => {
      if (!loaded) {
        clearTimeout(timer);
        setFailed(`Map tiles unavailable (${ev?.error?.message ?? "network"}). Showing the route line only.`);
      }
    });
    m.on("click", (ev) => clickRef.current?.(ev.lngLat.lat, ev.lngLat.lng));
    return () => {
      clearTimeout(timer);
      m.remove();
      map.current = null;
      marker.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.enabled, props.styleUrl]);

  // Update route geometry.
  useEffect(() => {
    const m = map.current;
    if (!m || !m.isStyleLoaded()) return;
    const src = m.getSource("route") as maplibregl.GeoJSONSource | undefined;
    if (src) {
      src.setData(geojson(props.line));
      fit(m, props.line);
    }
  }, [props.line]);

  // Rider position.
  useEffect(() => {
    const m = map.current;
    if (!m || !props.pos) return;
    if (!marker.current) {
      const dot = document.createElement("div");
      dot.className = "map-rider";
      dot.setAttribute("aria-label", "Your position");
      marker.current = new maplibregl.Marker({ element: dot }).setLngLat(props.pos).addTo(m);
    } else marker.current.setLngLat(props.pos);
    if (props.follow) m.easeTo({ center: props.pos, duration: 600 });
  }, [props.pos, props.follow]);

  // Builder waypoints.
  useEffect(() => {
    const m = map.current;
    if (!m) return;
    wpMarkers.current.forEach((mk) => mk.remove());
    wpMarkers.current = (props.waypoints ?? []).map((w, i, all) => {
      const d = document.createElement("div");
      d.className = `map-wp ${i === 0 ? "start" : i === all.length - 1 ? "end" : ""}`;
      d.textContent = i === 0 ? "S" : i === all.length - 1 ? "F" : String(i);
      return new maplibregl.Marker({ element: d }).setLngLat([w[1], w[0]]).addTo(m);
    });
  }, [props.waypoints]);

  if (failed || !props.enabled) {
    return <FallbackLine line={props.line} pos={props.pos ?? null} height={h} note={failed ?? undefined} waypoints={props.waypoints} onMapClick={props.onMapClick} />;
  }
  return (
    <div className="map-wrap" style={{ height: h }}>
      <div ref={el} className="map" />
      {props.synthetic && <div className="map-note">Synthetic demo route at 0°N 0°E — not a real road.</div>}
    </div>
  );
}

function fit(m: maplibregl.Map, line: number[][]) {
  const pts = unwrap(line);
  if (pts.length < 2) return;
  const b = new maplibregl.LngLatBounds([pts[0][0], pts[0][1]], [pts[0][0], pts[0][1]]);
  for (const p of pts) b.extend([p[0], p[1]]);
  m.fitBounds(b, { padding: 40, duration: 0, maxZoom: 15 });
}

/** Locally rendered route line (no network). */
function FallbackLine(props: { line: number[][]; pos: [number, number] | null; height: number; note?: string; waypoints?: [number, number][]; onMapClick?: (lat: number, lon: number) => void }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const h = props.height;
  const pts = unwrap(props.line);
  const lat0 = pts.length ? pts.reduce((a, p) => a + p[1], 0) / pts.length : 0;
  const k = Math.cos((lat0 * Math.PI) / 180);
  const xs = pts.map((p) => p[0] * k);
  const ys = pts.map((p) => p[1]);
  const [minX, maxX, minY, maxY] = xs.length ? [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)] : [0, 1, 0, 1];
  const span = Math.max(maxX - minX, maxY - minY, 1e-6);
  const sc = (Math.min(w, h) - 40) / span;
  const ox = (w - (maxX - minX) * sc) / 2;
  const oy = (h - (maxY - minY) * sc) / 2;
  const X = (lon: number) => ox + (lon * k - minX) * sc;
  const Y = (lat: number) => h - (oy + (lat - minY) * sc);
  let d = "";
  let prevSeg = -1;
  for (const p of pts) {
    d += `${p[2] !== prevSeg ? "M" : "L"}${X(p[0]).toFixed(1)},${Y(p[1]).toFixed(1)}`;
    prevSeg = p[2];
  }
  return (
    <div className="map-wrap map-fallback" ref={ref} style={{ height: h }}>
      <svg width={w} height={h} role="img" aria-label="Route line">
        <path d={d} className="fallback-casing" />
        <path d={d} className="fallback-line" />
        {props.pos && <circle cx={X(props.pos[0])} cy={Y(props.pos[1])} r={8} className="rider-dot" />}
      </svg>
      {props.note && <div className="map-note">{props.note}</div>}
    </div>
  );
}
