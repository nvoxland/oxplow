import type { MetricFinding, MetricSample } from "../api.js";

// Pure helpers behind the per-kind Metric detail view (tsk232). Kept out of the
// component so they're unit-testable without a DOM — same split as
// `buildExplorerSeries` in MetricsExplorer.

export type SeriesPoint = { t: number; v: number };

/** An inclusive epoch-ms time window the detail page is scoped to. */
export type TimeRange = { from: number; to: number };

const DAY_MS = 24 * 60 * 60 * 1000;

/** The preset windows offered in the range dropdown (label + lookback span). */
export const RANGE_PRESETS: Array<{ key: string; label: string; ms: number }> = [
  { key: "1d", label: "Last day", ms: DAY_MS },
  { key: "2d", label: "Last 2 days", ms: 2 * DAY_MS },
  { key: "3d", label: "Last 3 days", ms: 3 * DAY_MS },
  { key: "7d", label: "Last 7 days", ms: 7 * DAY_MS },
  { key: "30d", label: "Last month", ms: 30 * DAY_MS },
];

export const DEFAULT_RANGE_KEY = "7d";

/** Build a `[now - preset.ms, now]` window. */
export function rangeFromPreset(key: string, now: number): TimeRange {
  const preset = RANGE_PRESETS.find((p) => p.key === key) ?? RANGE_PRESETS[3]!;
  return { from: now - preset.ms, to: now };
}

/** Which preset (if any) a range corresponds to: its `to` is ~now (within a
 *  minute) and its span matches a preset. Otherwise `"custom"`. */
export function matchPresetKey(range: TimeRange, now: number): string {
  if (Math.abs(range.to - now) > 60_000) return "custom";
  const span = range.to - range.from;
  const hit = RANGE_PRESETS.find((p) => Math.abs(p.ms - span) < 60_000);
  return hit?.key ?? "custom";
}

/** Samples whose `captured_at` falls inside the window (inclusive). */
export function filterByRange(samples: MetricSample[], range: TimeRange): MetricSample[] {
  return samples.filter((s) => {
    const t = Date.parse(String(s.captured_at));
    return !Number.isNaN(t) && t >= range.from && t <= range.to;
  });
}

/** Epoch ms → a `YYYY-MM-DDTHH:mm` string for a `datetime-local` input (local
 *  time). */
export function toLocalInput(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** A `datetime-local` value → epoch ms, or `null` if unparseable/empty. */
export function fromLocalInput(value: string): number | null {
  if (!value) return null;
  const ms = new Date(value).getTime();
  return Number.isNaN(ms) ? null : ms;
}

/** Samples (any order) → time-ascending `{t,v}` points, dropping unparseable
 *  timestamps. The shared input to every kind's trend chart. */
export function seriesPoints(samples: MetricSample[]): SeriesPoint[] {
  return samples
    .map((s) => ({ t: Date.parse(String(s.captured_at)), v: s.value }))
    .filter((p) => !Number.isNaN(p.t))
    .sort((a, b) => a.t - b.t);
}

/** How the trend chart visualizes the series. */
export type ChartMode = "value" | "cumulative" | "change" | "avg";

export const CHART_MODES: Array<{ key: ChartMode; label: string }> = [
  { key: "value", label: "Value" },
  { key: "cumulative", label: "Cumulative" },
  { key: "change", label: "Change" },
  { key: "avg", label: "Moving avg" },
];

const AVG_WINDOW = 5;

/** The chart mode that best matches how a metric rolls up: a `sum`/per-event
 *  metric (tokens, nudges) reads as a running total → `cumulative`; an `avg`
 *  metric → `avg`; a level gauge (`last`) → the raw `value`. The page seeds the
 *  chart with this until the user picks a mode. */
export function defaultChartMode(defaultAgg: string): ChartMode {
  if (defaultAgg === "sum") return "cumulative";
  if (defaultAgg === "avg") return "avg";
  return "value";
}

/** Re-shape a time-ascending series for the chosen visualization:
 *  - `value` — the recorded value (identity);
 *  - `cumulative` — running sum;
 *  - `change` — delta vs the previous point (drops the first point);
 *  - `avg` — trailing moving average over the last {@link AVG_WINDOW} points. */
export function transformSeries(points: SeriesPoint[], mode: ChartMode): SeriesPoint[] {
  switch (mode) {
    case "cumulative": {
      let acc = 0;
      return points.map((p) => ({ t: p.t, v: (acc += p.v) }));
    }
    case "change":
      return points.slice(1).map((p, i) => ({ t: p.t, v: p.v - points[i]!.v }));
    case "avg":
      return points.map((p, i) => {
        const win = points.slice(Math.max(0, i - AVG_WINDOW + 1), i + 1);
        const sum = win.reduce((a, b) => a + b.v, 0);
        return { t: p.t, v: sum / win.length };
      });
    default:
      return points;
  }
}

/** Distinct non-null branches present in the samples, sorted. */
export function branchOptions(samples: MetricSample[]): string[] {
  const set = new Set<string>();
  for (const s of samples) if (s.branch) set.add(s.branch);
  return [...set].sort();
}

/** Samples on `branch`, or all when `branch` is null (the "All branches"
 *  option). */
export function filterByBranch(samples: MetricSample[], branch: string | null): MetricSample[] {
  if (branch == null) return samples;
  return samples.filter((s) => s.branch === branch);
}

/** Latest value minus the earliest (the effort/window delta). `null` when there
 *  are fewer than two points. */
export function deltaVsFirst(samples: MetricSample[]): number | null {
  const pts = seriesPoints(samples);
  if (pts.length < 2) return null;
  return pts[pts.length - 1]!.v - pts[0]!.v;
}

/** The headline "in range" stat, computed the way the metric ROLLS UP
 *  (`default_agg`) rather than always last−first — so a `sum`/per-event metric
 *  like tokens shows its window total, not a meaningless endpoint diff:
 *  - `sum` → "Total in range" (Σ of in-range values);
 *  - `avg` → "Avg in range" (mean);
 *  - anything else (`last`/level gauges) → "Δ in range" (last − first), the
 *    signed change. `null` when there's nothing to show. */
export function inRangeStat(
  samples: MetricSample[],
  defaultAgg: string,
): { label: string; value: number; signed: boolean } | null {
  const pts = seriesPoints(samples);
  if (pts.length === 0) return null;
  if (defaultAgg === "sum") {
    return { label: "Total in range", value: pts.reduce((a, p) => a + p.v, 0), signed: false };
  }
  if (defaultAgg === "avg") {
    return { label: "Avg in range", value: pts.reduce((a, p) => a + p.v, 0) / pts.length, signed: false };
  }
  if (pts.length < 2) return null;
  return { label: "Δ in range", value: pts[pts.length - 1]!.v - pts[0]!.v, signed: true };
}

/** Top-N subjects by summed value — the `event`/`findings` "where is it
 *  concentrated" breakdown. Groups by `subject_ref` (falling back to
 *  `subject_kind`); samples with no subject are bucketed under "—". */
export function topSubjects(
  samples: MetricSample[],
  n: number,
): { subject: string; value: number }[] {
  const sums = new Map<string, number>();
  for (const s of samples) {
    const key = s.subject_ref ?? s.subject_kind ?? "—";
    sums.set(key, (sums.get(key) ?? 0) + s.value);
  }
  return [...sums.entries()]
    .map(([subject, value]) => ({ subject, value }))
    .sort((a, b) => b.value - a.value)
    .slice(0, n);
}

/** The real findings rows (lint hits, complexity, …) for the findings-kind
 *  table — excludes the verbatim `*-detail` payload findings the producers
 *  attach for the effort panel. */
export function findingRows(findings: MetricFinding[]): MetricFinding[] {
  return findings.filter((f) => !f.kind.endsWith("-detail"));
}

/** Parse a `*-detail` finding's `extra_json` payload (the test suite/case tree
 *  or coverage file/line map kept verbatim by the producers). `null` when
 *  absent or unparseable. */
export function parseDetailPayload(
  findings: MetricFinding[],
  detailKind: string,
): unknown | null {
  const f = findings.find((x) => x.kind === detailKind);
  if (!f?.extra_json) return null;
  try {
    return JSON.parse(f.extra_json) as unknown;
  } catch {
    return null;
  }
}
