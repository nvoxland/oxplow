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
