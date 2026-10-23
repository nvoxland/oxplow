import type { SeriesPoint } from "../api.js";
import type { TimeRange } from "../kit/rangePresets.js";

// Pure helpers behind the per-kind Metric detail view (tsk232). Kept out of the
// component so they're unit-testable without a DOM — same split as
// the metric detail page's chart. Operates over the engine's
// per-capture `SeriesPoint`s (epic tsk12); `ChartPoint` is the reduced `{t,v}`
// shape the trend chart plots.

export type ChartPoint = { t: number; v: number };

/** Series points whose `captured_at` falls inside the window (inclusive). */
export function filterByRange(samples: SeriesPoint[], range: TimeRange): SeriesPoint[] {
  return samples.filter((s) => {
    const t = Date.parse(String(s.captured_at));
    return !Number.isNaN(t) && t >= range.from && t <= range.to;
  });
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

/** Y-axis scaling. `auto` fits the data (+ target) with padding — the same
 *  data-relative scaling the Metrics sparkline uses, so a metric whose
 *  variation is small relative to its value (avg complexity ~1.96, coverage
 *  ~98%) still shows its trend instead of a flat line pinned near the top.
 *  `zero` forces the axis through 0 (honest about absolute magnitude). */
export type ChartScale = "auto" | "zero";

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
