/// The quick picks of a `date_range` param, by key. A lens names the ones
/// it offers (`presets`); an unknown key shows as itself.
const LABELS: Record<string, string> = {
  "1d": "Last day",
  "2d": "Last 2 days",
  "3d": "Last 3 days",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
  "90d": "Last 90 days",
  all: "All time",
};

/** What a `date_range` offers when its lens names no presets. */
export const DEFAULT_RANGE_PRESETS = ["1d", "7d", "30d", "all"];

export function rangePresetLabel(key: string): string {
  return LABELS[key] ?? key;
}

/** `a..b` split into its two ends (either may be empty); null for a preset. */
export function splitRange(value: string): [string, string] | null {
  const at = value.indexOf("..");
  if (at === -1) return null;
  return [value.slice(0, at), value.slice(at + 2)];
}

/** An inclusive epoch-ms time window. */
export type TimeRange = { from: number; to: number };

const DAY_MS = 24 * 60 * 60 * 1000;
const SPAN_DAYS: Record<string, number> = { "1d": 1, "2d": 2, "3d": 3, "7d": 7, "30d": 30, "90d": 90 };

/** What a range is when its value names none. */
export const DEFAULT_RANGE_KEY = "7d";

/** `[now - span, now]` for a preset key; an unknown one is the default week. */
export function rangeFromPreset(key: string, now: number): TimeRange {
  const days = SPAN_DAYS[key] ?? SPAN_DAYS[DEFAULT_RANGE_KEY]!;
  return { from: now - days * DAY_MS, to: now };
}

/** An end of `a..b` as epoch ms: a date as the end takes in its whole day. */
function endMs(text: string, isTo: boolean): number | null {
  const ms = Date.parse(text);
  if (Number.isNaN(ms)) return null;
  return isTo && /^\d{4}-\d{2}-\d{2}$/.test(text) ? ms + DAY_MS - 1 : ms;
}

/** The window a range value stands for — a preset key, `all`, or `a..b`
 *  (dates or date-times, either end open) — or null for all time. */
export function resolveRange(value: string, now: number): TimeRange | null {
  if (value === "all") return null;
  const ends = splitRange(value);
  if (!ends) return rangeFromPreset(value, now);
  return { from: endMs(ends[0], false) ?? 0, to: endMs(ends[1], true) ?? now };
}

/** The preset a window is — its `to` within a minute of now and its span a
 *  preset's — else `"custom"`. */
export function matchPresetKey(range: TimeRange, now: number): string {
  if (Math.abs(range.to - now) > 60_000) return "custom";
  const span = range.to - range.from;
  return Object.entries(SPAN_DAYS).find(([, days]) => Math.abs(days * DAY_MS - span) < 60_000)?.[0] ?? "custom";
}

/** `range` as a range value: its preset key, else `a..b`. */
export function rangeValue(range: TimeRange, now: number): string {
  const key = matchPresetKey(range, now);
  return key !== "custom" ? key : `${new Date(range.from).toISOString()}..${new Date(range.to).toISOString()}`;
}
