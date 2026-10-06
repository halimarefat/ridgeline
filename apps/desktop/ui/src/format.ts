// Display formatting. Everything is stored in SI units; conversion happens
// only here, at the edge.

export type Units = "metric" | "imperial";

export const KM_PER_MI = 1.609344;
export const LB_PER_KG = 2.2046226218;
export const FT_PER_M = 3.280839895;

export function dist(m: number | null | undefined, u: Units, digits = 1): string {
  if (m == null || !isFinite(m)) return "—";
  return u === "imperial" ? `${(m / 1000 / KM_PER_MI).toFixed(digits)} mi` : `${(m / 1000).toFixed(digits)} km`;
}

export function distValue(m: number, u: Units): number {
  return u === "imperial" ? m / 1000 / KM_PER_MI : m / 1000;
}
export function distUnit(u: Units): string {
  return u === "imperial" ? "mi" : "km";
}

export function speed(mps: number | null | undefined, u: Units): string {
  if (mps == null || !isFinite(mps)) return "—";
  return u === "imperial" ? `${((mps * 3.6) / KM_PER_MI).toFixed(1)} mph` : `${(mps * 3.6).toFixed(1)} km/h`;
}

export function speedValue(mps: number, u: Units): number {
  return u === "imperial" ? (mps * 3.6) / KM_PER_MI : mps * 3.6;
}
export function speedUnit(u: Units): string {
  return u === "imperial" ? "mph" : "km/h";
}

export function elev(m: number | null | undefined, u: Units): string {
  if (m == null || !isFinite(m)) return "—";
  return u === "imperial" ? `${Math.round(m * FT_PER_M)} ft` : `${Math.round(m)} m`;
}

export function mass(kg: number, u: Units): number {
  return u === "imperial" ? Math.round(kg * LB_PER_KG * 10) / 10 : Math.round(kg * 10) / 10;
}
export function massToKg(v: number, u: Units): number {
  return u === "imperial" ? v / LB_PER_KG : v;
}
export function massUnit(u: Units): string {
  return u === "imperial" ? "lb" : "kg";
}

export function clock(s: number | null | undefined): string {
  if (s == null || !isFinite(s)) return "—";
  const t = Math.max(0, Math.floor(s));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = t % 60;
  const mm = String(m).padStart(h > 0 ? 2 : 1, "0");
  return h > 0 ? `${h}:${mm}:${String(sec).padStart(2, "0")}` : `${mm}:${String(sec).padStart(2, "0")}`;
}

export function minutes(s: number | null | undefined): string {
  if (s == null || !isFinite(s)) return "—";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const r = m % 60;
  return r ? `${h} h ${r} min` : `${h} h`;
}

export function pct(v: number | null | undefined, digits = 1): string {
  if (v == null || !isFinite(v)) return "—";
  const s = v.toFixed(digits);
  return `${v > 0 ? "+" : ""}${s}%`;
}

export function round(v: number | null | undefined): string {
  if (v == null || !isFinite(v)) return "—";
  return String(Math.round(v));
}

export function dateLabel(iso: string): string {
  // iso = YYYY-MM-DD in the rider's calendar
  const [y, m, d] = iso.split("-").map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d));
  return dt.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric", timeZone: "UTC" });
}

export function dateTime(utcMs: number): string {
  return new Date(utcMs).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function addDays(iso: string, n: number): string {
  const [y, m, d] = iso.split("-").map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d + n));
  return dt.toISOString().slice(0, 10);
}

export function weekday(iso: string): number {
  // 0 = Monday … 6 = Sunday
  const [y, m, d] = iso.split("-").map(Number);
  return (new Date(Date.UTC(y, m - 1, d)).getUTCDay() + 6) % 7;
}

export const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Power zone index (0..6) for a %FTP value — mirrors the Rust zone table. */
export function zoneOf(pctFtp: number): number {
  const edges = [55, 75, 90, 105, 120, 150];
  let i = 0;
  while (i < edges.length && pctFtp >= edges[i]) i++;
  return i;
}

export const ZONE_NAMES = ["Z1 Recovery", "Z2 Endurance", "Z3 Tempo", "Z4 Threshold", "Z5 VO2", "Z6 Anaerobic", "Z7 Sprint"];

/** Grade bucket (0..6) for colouring elevation profiles. */
export function gradeBucket(g: number | null | undefined): number {
  if (g == null || !isFinite(g)) return -1;
  if (g < -4) return 0;
  if (g < -1) return 1;
  if (g < 1) return 2;
  if (g < 4) return 3;
  if (g < 8) return 4;
  if (g < 12) return 5;
  return 6;
}

export const GRADE_LABELS = ["Steep descent", "Descent", "Flat", "Rolling climb", "Climb", "Steep climb", "Very steep"];

export function localTzInfo(): { tz_name: string; tz_offset_min: number } {
  return { tz_name: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC", tz_offset_min: -new Date().getTimezoneOffset() };
}
