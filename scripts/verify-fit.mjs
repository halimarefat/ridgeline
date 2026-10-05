// Independent FIT verification with the Garmin FIT JavaScript SDK.
// Usage: node scripts/verify-fit.mjs <fit-sdk-src-dir> <file.fit> <expect.json>
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { join, resolve } from "node:path";

const [sdkDir, fitPath, expectPath] = process.argv.slice(2);
const { Decoder, Stream } = await import(pathToFileURL(join(resolve(sdkDir), "index.js")).href);
const expect = JSON.parse(readFileSync(expectPath, "utf8"));
const bytes = readFileSync(fitPath);
const stream = Stream.fromByteArray(Array.from(bytes));
const decoder = new Decoder(stream);
const fail = (m) => { console.error("FAIL:", m); process.exit(1); };
if (!Decoder.isFIT(stream)) fail("not a FIT file");
if (!decoder.checkIntegrity()) fail("CRC / integrity check failed");
const { messages, errors } = decoder.read({ convertDateTimesToDates: false, includeUnknownData: true });
if (errors.length) fail("decoder errors: " + errors.map(String).join("; "));
const rec = messages.recordMesgs ?? [];
const laps = messages.lapMesgs ?? [];
const sess = (messages.sessionMesgs ?? [])[0];
const act = (messages.activityMesgs ?? [])[0];
const ev = messages.eventMesgs ?? [];
const fid = (messages.fileIdMesgs ?? [])[0];
const checks = [
  ["file type activity", fid?.type === "activity"],
  ["record count", rec.length === expect.records],
  ["lap count", laps.length === expect.laps && sess?.numLaps === expect.laps],
  ["session timer time", Math.abs(sess.totalTimerTime - expect.timer_s) < 0.01],
  ["session elapsed time", Math.abs(sess.totalElapsedTime - expect.elapsed_s) < 0.01],
  ["avg power", Math.abs(sess.avgPower - expect.avg_power) <= 1],
  ["sport cycling", sess.sport === "cycling"],
  ["sub sport", sess.subSport === expect.sub_sport],
  ["activity message", act?.numSessions === 1],
  ["missing HR stays missing", rec.filter((r) => r.heartRate === undefined).length === expect.missing_hr_records],
  ["missing power stays missing", rec.filter((r) => r.power === undefined).length === 5],
  ["timer events (start, pause, resume, stop)", ev.filter((e) => e.event === "timer").length === 4],
  ["monotonic timestamps", rec.every((r, i) => i === 0 || r.timestamp > rec[i - 1].timestamp)],
  ["lap timer sum equals session", Math.abs(laps.reduce((a, l) => a + l.totalTimerTime, 0) - sess.totalTimerTime) < 0.01],
];
let ok = true;
for (const [name, pass] of checks) { console.log(`${pass ? "ok  " : "FAIL"} ${name}`); ok &&= pass; }
if (!ok) process.exit(1);
console.log(`verified ${fitPath}: ${rec.length} records, ${laps.length} laps, timer ${sess.totalTimerTime}s`);
