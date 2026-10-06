// Display conversions: SI in, rider units out; missing stays missing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { addDays, clock, dist, elev, gradeBucket, mass, massToKg, minutes, pct, speed, weekday, zoneOf } from "../src/format.ts";

test("distance, speed and elevation convert at the edge", () => {
  assert.equal(dist(12345, "metric"), "12.3 km");
  assert.equal(dist(1609.344, "imperial"), "1.0 mi");
  assert.equal(speed(10, "metric"), "36.0 km/h");
  assert.equal(speed(10, "imperial"), "22.4 mph");
  assert.equal(elev(100, "metric"), "100 m");
  assert.equal(elev(100, "imperial"), "328 ft");
});

test("missing and non-finite values are shown as missing, never zero", () => {
  for (const v of [null, undefined, NaN, Infinity]) {
    assert.equal(dist(v as number, "metric"), "—");
    assert.equal(speed(v as number, "metric"), "—");
    assert.equal(elev(v as number, "metric"), "—");
  }
  assert.equal(pct(null), "—");
});

test("mass round-trips between kg and lb", () => {
  assert.equal(mass(75, "metric"), 75);
  assert.equal(mass(75, "imperial"), 165.3);
  assert.ok(Math.abs(massToKg(165.3, "imperial") - 75) < 0.05);
});

test("clock and minutes formatting", () => {
  assert.equal(clock(0), "0:00");
  assert.equal(clock(65), "1:05");
  assert.equal(clock(3725), "1:02:05");
  assert.equal(minutes(5400), "1 h 30 min");
});

test("power zones follow the 7-zone %FTP table", () => {
  assert.deepEqual([40, 55, 74.9, 75, 90, 105, 120, 150, 300].map(zoneOf), [0, 1, 1, 2, 3, 4, 5, 6, 6]);
});

test("grade buckets are signed and ordered", () => {
  assert.deepEqual([-10, -3, 0, 2, 5, 10, 15].map(gradeBucket), [0, 1, 2, 3, 4, 5, 6]);
  assert.equal(gradeBucket(null), -1);
});

test("calendar dates stay in the rider's calendar (no timezone drift)", () => {
  assert.equal(addDays("2026-10-05", 1), "2026-10-06");
  assert.equal(addDays("2026-12-31", 1), "2027-01-01");
  assert.equal(addDays("2026-03-01", -1), "2026-02-28");
  assert.equal(weekday("2026-10-05"), 0); // Monday
  assert.equal(weekday("2026-10-11"), 6); // Sunday
});
