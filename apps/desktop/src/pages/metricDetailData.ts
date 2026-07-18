import type { MetricSpec, SeriesPoint } from "../api.js";

// Pure helpers behind the per-kind Metric detail view (tsk232). Kept out of the
// component so they're unit-testable without a DOM — same split as
// `buildExplorerSeries` in MetricsExplorer. Operates over the engine's
// per-capture `SeriesPoint`s (epic tsk12); `ChartPoint` is the reduced `{t,v}`
// shape the trend chart plots.

export type ChartPoint = { t: number; v: number };

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

/** Series points whose `captured_at` falls inside the window (inclusive). */
export function filterByRange(samples: SeriesPoint[], range: TimeRange): SeriesPoint[] {
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

/** Series points (any order) → time-ascending `{t,v}` chart points, dropping
 *  unparseable timestamps. The shared input to every kind's trend chart. */
export function seriesPoints(samples: SeriesPoint[]): ChartPoint[] {
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

/** Y-axis scaling. `auto` fits the data (+ target) with padding — the same
 *  data-relative scaling the Recorded Metrics sparkline uses, so a metric whose
 *  variation is small relative to its value (avg complexity ~1.96, coverage
 *  ~98%) still shows its trend instead of a flat line pinned near the top.
 *  `zero` forces the axis through 0 (honest about absolute magnitude). */
export type ChartScale = "auto" | "zero";

export const CHART_SCALES: Array<{ key: ChartScale; label: string }> = [
  { key: "auto", label: "Auto" },
  { key: "zero", label: "From zero" },
];

export const DEFAULT_CHART_SCALE: ChartScale = "auto";

/** The chart's Y-axis `[min, max]` for a set of values (+ an optional target
 *  line that always stays in view). `zero` forces the axis through 0; `auto`
 *  fits the data with ~8% padding — for a flat series (span 0) it pads by a
 *  fraction of the value so the line renders mid-chart rather than on an edge.
 *  Pure + exported so the scaling is unit-tested independent of the SVG. */
export function yDomain(
  values: number[],
  target: number | null | undefined,
  scale: ChartScale,
): { min: number; max: number } {
  const lo = Math.min(...values, target ?? Infinity);
  const hi = Math.max(...values, target ?? -Infinity);
  if (scale === "zero") {
    return { min: Math.min(0, lo), max: Math.max(0, hi) };
  }
  const span = hi - lo;
  const pad = span > 0 ? span * 0.08 : Math.abs(hi) * 0.08 || 1;
  return { min: lo - pad, max: hi + pad };
}

const AVG_WINDOW = 5;

/** The chart mode that best matches the spec's `aggregation`: a `sum`/per-event
 *  metric (tokens, nudges) reads as a running total → `cumulative`; an `avg`
 *  metric → `avg`; a level gauge (`last`/`count`) → the raw `value`. The page
 *  seeds the chart with this until the user picks a mode. */
export function defaultChartMode(aggregation: string): ChartMode {
  if (aggregation === "sum") return "cumulative";
  if (aggregation === "avg") return "avg";
  return "value";
}

/** Re-shape a time-ascending series for the chosen visualization:
 *  - `value` — the recorded value (identity);
 *  - `cumulative` — running sum;
 *  - `change` — delta vs the previous point (drops the first point);
 *  - `avg` — trailing moving average over the last {@link AVG_WINDOW} points. */
export function transformSeries(points: ChartPoint[], mode: ChartMode): ChartPoint[] {
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

/**
 * The dimensions a per-file metric can be broken down by: always `package`
 * (the file's directory), plus any per-file `dims_json` key the metric declares
 * (e.g. `language`). Run/time dims that aren't a per-file grain (`git_version`,
 * `branch`) are excluded.
 *
 * Lives here rather than inside `MetricDetail.tsx` (where it started) because
 * more than one surface asks the question: the detail page's breakdown card and
 * the dashboard's breakout picker, which needs to know whether a given tile can
 * participate in a chosen dimension (tsk150). One rule, so the two can't
 * disagree about what a metric is sliceable by.
 */
export function breakdownDimensions(def: MetricSpec): string[] {
  const out = ["package"];
  if (def.sliceable_dims_json) {
    try {
      for (const d of JSON.parse(def.sliceable_dims_json) as string[]) {
        if (d !== "git_version" && d !== "branch" && !out.includes(d)) out.push(d);
      }
    } catch {
      /* ignore */
    }
  }
  return out;
}

/** Distinct non-null branches present in the series points, sorted. */
export function branchOptions(samples: SeriesPoint[]): string[] {
  const set = new Set<string>();
  for (const s of samples) if (s.branch) set.add(s.branch);
  return [...set].sort();
}

/** Series points on `branch`, or all when `branch` is null (the "All branches"
 *  option). */
export function filterByBranch(samples: SeriesPoint[], branch: string | null): SeriesPoint[] {
  if (branch == null) return samples;
  return samples.filter((s) => s.branch === branch);
}

/** Latest value minus the earliest (the effort/window delta). `null` when there
 *  are fewer than two points. */
export function deltaVsFirst(samples: SeriesPoint[]): number | null {
  const pts = seriesPoints(samples);
  if (pts.length < 2) return null;
  return pts[pts.length - 1]!.v - pts[0]!.v;
}

/** The headline "in range" stat, computed the way the spec AGGREGATES rather
 *  than always last−first — so a `sum`/per-event metric like tokens shows its
 *  window total, not a meaningless endpoint diff:
 *  - `sum` → "Total in range" (Σ of in-range values);
 *  - `avg` → "Avg in range" (mean);
 *  - anything else (`last`/`count` level gauges) → "Δ in range" (last − first),
 *    the signed change. `null` when there's nothing to show. */
export function inRangeStat(
  samples: SeriesPoint[],
  aggregation: string,
): { label: string; value: number; signed: boolean } | null {
  const pts = seriesPoints(samples);
  if (pts.length === 0) return null;
  if (aggregation === "sum") {
    return { label: "Total in range", value: pts.reduce((a, p) => a + p.v, 0), signed: false };
  }
  if (aggregation === "avg") {
    return { label: "Avg in range", value: pts.reduce((a, p) => a + p.v, 0) / pts.length, signed: false };
  }
  if (pts.length < 2) return null;
  return { label: "Δ in range", value: pts[pts.length - 1]!.v - pts[0]!.v, signed: true };
}
