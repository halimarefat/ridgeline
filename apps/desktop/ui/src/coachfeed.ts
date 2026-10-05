// Pure helpers for the in-ride coach feed (no React, unit-tested).

export interface FeedItem {
  id: number;
  at_s: number;
  from: "cue" | "ai" | "coach" | "rider" | "safety" | "note";
  kind: string;
  text: string;
  action: { kind: "intensity"; delta: number; label: string } | { kind: "stop"; label: string } | null;
  speak: boolean;
}

export const QUICK_PROMPTS: { trigger: string; label: string; key: string }[] = [
  { trigger: "how", label: "How am I doing?", key: "1" },
  { trigger: "too_hard", label: "Too hard", key: "2" },
  { trigger: "too_easy", label: "Too easy", key: "3" },
  { trigger: "motivate", label: "Motivate me", key: "4" },
];

export function sourceLabel(from: FeedItem["from"], model?: string): string {
  switch (from) {
    case "cue":
      return "Ride cue";
    case "ai":
      return model ? `AI coach · ${model}` : "AI coach";
    case "coach":
      return "Offline coach";
    case "safety":
      return "Safety";
    case "rider":
      return "You";
    default:
      return "Note";
  }
}

/** New items that should be read aloud, oldest first. */
export function itemsToSpeak(feed: FeedItem[], lastSpokenId: number): FeedItem[] {
  return feed.filter((f) => f.id > lastSpokenId && f.speak && f.from !== "rider" && f.from !== "note");
}

/** Make cue text sound natural when spoken ("400 W" → "400 watts"). */
export function speechText(text: string): string {
  return text
    .replace(/(\d)\s*→\s*(\d)/g, "$1 to $2")
    .replace(/(\d+):(\d{2}) min\b/g, (_m, a: string, b: string) => `${Number(a)} minutes ${Number(b)} seconds`)
    .replace(/(\d)\s*W\b/g, "$1 watts")
    .replace(/(\d)\s*rpm\b/g, "$1 R P M")
    .replace(/(\d)\s*bpm\b/g, "$1 beats per minute")
    .replace(/(\d)\s*min\b/g, "$1 minutes")
    .replace(/\b1 minutes\b/g, "1 minute")
    .replace(/(\d)\s*s\b/g, "$1 seconds")
    .replace(/(\d)\s*km\b/g, "$1 kilometres")
    .replace(/(\d)\s*mi\b/g, "$1 miles")
    .replace(/(\d)\s*ft\b/g, "$1 feet")
    .replace(/(\d)\s*m\b/g, "$1 metres")
    .replace(/(\d)\s*–\s*(\d)/g, "$1 to $2")
    .replace(/[−-](\d)/g, "minus $1")
    .replace(/\+(\d)/g, "plus $1")
    .replace(/%/g, " percent")
    .replace(/\s{2,}/g, " ")
    .trim();
}

/** Whether a suggestion button can still be pressed. */
export function actionUsable(item: FeedItem, latestId: number, state: string, hasWorkout: boolean, applied: Set<number>): boolean {
  if (!item.action || applied.has(item.id)) return false;
  if (state !== "running" && state !== "paused") return false;
  if (item.action.kind === "intensity" && !hasWorkout) return false;
  // Old suggestions go stale: only the five most recent items stay actionable.
  return latestId - item.id < 5;
}
