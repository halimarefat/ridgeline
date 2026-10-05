// In-ride coach feed helpers: what is spoken, how it sounds, and when a
// suggestion button may still be pressed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { actionUsable, itemsToSpeak, sourceLabel, speechText, type FeedItem } from "../src/coachfeed.ts";

const item = (id: number, from: FeedItem["from"], extra: Partial<FeedItem> = {}): FeedItem => ({ id, at_s: id * 10, from, kind: "x", text: `t${id}`, action: null, speak: true, ...extra });

test("only new coach lines are spoken, never the rider's own words or notes", () => {
  const feed = [item(1, "cue"), item(2, "rider"), item(3, "ai"), item(4, "note"), item(5, "coach", { speak: false }), item(6, "safety")];
  assert.deepEqual(itemsToSpeak(feed, 0).map((f) => f.id), [1, 3, 6]);
  assert.deepEqual(itemsToSpeak(feed, 3).map((f) => f.id), [6]);
});

test("cue text reads naturally", () => {
  assert.equal(speechText("In 15 s: Work 1/4 — 5 min at 400 W."), "In 15 seconds: Work 1/4 — 5 minutes at 400 watts.");
  assert.equal(speechText("Cadence 72 rpm — spin up to 85–95."), "Cadence 72 R P M — spin up to 85 to 95.");
  assert.equal(speechText("Climb in 400 m: 1.2 km at 5.0% average."), "Climb in 400 metres: 1.2 kilometres at 5.0 percent average.");
  assert.equal(speechText("Warm-up: 12 min at 180→320 W."), "Warm-up: 12 minutes at 180 to 320 watts.");
  assert.equal(speechText("Halfway — 2:30 min to go."), "Halfway — 2 minutes 30 seconds to go.");
  assert.equal(speechText("Intensity is now -5%."), "Intensity is now minus 5 percent.");
});

test("suggestions are pressable only while riding, once, and while recent", () => {
  const ease = item(10, "coach", { action: { kind: "intensity", delta: -5, label: "Easier 5%" } });
  const stop = item(11, "safety", { action: { kind: "stop", label: "Stop the ride" } });
  assert.equal(actionUsable(ease, 10, "running", true, new Set()), true);
  assert.equal(actionUsable(ease, 10, "finished", true, new Set()), false);
  assert.equal(actionUsable(ease, 10, "running", false, new Set()), false, "no intensity control without a workout");
  assert.equal(actionUsable(ease, 10, "running", true, new Set([10])), false, "applied once");
  assert.equal(actionUsable(ease, 15, "running", true, new Set()), false, "stale after newer lines");
  assert.equal(actionUsable(stop, 11, "paused", false, new Set()), true);
  assert.equal(sourceLabel("ai", "llama3.2"), "AI coach · llama3.2");
  assert.equal(sourceLabel("cue"), "Ride cue");
});
