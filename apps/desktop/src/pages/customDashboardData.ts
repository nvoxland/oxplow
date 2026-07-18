import type { Dashboard, MetricCatalogEntry, MetricSpec, SeriesPoint } from "../api.js";
import type { MenuItem } from "../menu.js";
import {
  type ChartMode,
  type ChartScale,
  type TimeRange,
  breakdownDimensions,
  rangeFromPreset,
  seriesPoints,
} from "./metricDetailData.js";

// Pure helpers behind the custom-dashboard page + tiles (tsk141, epic tsk138).
// Kept React-free so the tile-option parsing and the add-metric menu assembly
// are unit-testable without mounting the page — same split as
// `metricDetailData.ts`.

/** How one metric tile is rendered. Persisted as the tile's opaque
 *  `options_json` blob (so the shape grows with no migration). Phase 3 ships
 *  `line` + `number`; later phases add `sparkline` / `bar` / sizing / a
 *  per-tile range+branch override. Everything is optional — an absent field
 *  falls back to the tile default. */
export interface TileOptions {
  /** `line` (trend chart) | `number` (big headline value) | `sparkline` (bare
   *  trend line) | `bar` (breakdown bars by `dim`). Default `line`. */
  viz?: "line" | "number" | "sparkline" | "bar";
  /** Chart transform for a `line` tile (value / cumulative / change / avg). */
  mode?: ChartMode;
  /** Y-axis scaling for a `line` tile (auto / from-zero). */
  scale?: ChartScale;
  /** Title override — else the metric's own title. */
  title?: string;
  /** Grid footprint: `wide` spans 2 columns, `tall` spans 2 rows, `full` spans
   *  the whole grid width (the heading-band size text tiles default to).
   *  Default `small` (1×1). */
  size?: "small" | "wide" | "tall" | "full";
  /** Breakdown dimension for a `bar` tile (e.g. `package`, `language`). */
  dim?: string;
  /** Heading text for a `text` tile. Plain text, not markdown — see `TextTile`. */
  text?: string;
  /** Per-tile time-range override: a {@link RANGE_PRESETS} key, or `all` for no
   *  window. Absent → inherit the dashboard's filter. */
  range?: string;
  /** Per-tile branch override. Absent → inherit the dashboard's filter. */
  branch?: string;
  /** Highlight the tile when the metric is missing its target. Absent → the
   *  tile's default (on): it only shows for a metric that *has* a target and is
   *  missing it, so it's never noise for target-less metrics. */
  alertOffTarget?: boolean;
}

const VIZ = new Set<TileOptions["viz"]>(["line", "number", "sparkline", "bar"]);
const SIZES = new Set<TileOptions["size"]>(["small", "wide", "tall", "full"]);
const MODES = new Set<ChartMode>(["value", "cumulative", "change", "avg"]);
const SCALES = new Set<ChartScale>(["auto", "zero"]);

/** Parse a tile's opaque `options_json` blob into a {@link TileOptions},
 *  tolerating null / blank / malformed JSON (→ `{}`) and silently dropping any
 *  field whose value isn't one this version understands. A tile with a bad blob
 *  renders with defaults rather than crashing the whole grid. */
export function parseTileOptions(json: string | null | undefined): TileOptions {
  if (!json) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (typeof raw !== "object" || raw === null) return {};
  const obj = raw as Record<string, unknown>;
  const out: TileOptions = {};
  if (typeof obj.viz === "string" && VIZ.has(obj.viz as TileOptions["viz"])) {
    out.viz = obj.viz as TileOptions["viz"];
  }
  if (typeof obj.mode === "string" && MODES.has(obj.mode as ChartMode)) {
    out.mode = obj.mode as ChartMode;
  }
  if (typeof obj.scale === "string" && SCALES.has(obj.scale as ChartScale)) {
    out.scale = obj.scale as ChartScale;
  }
  if (typeof obj.title === "string") out.title = obj.title;
  if (typeof obj.size === "string" && SIZES.has(obj.size as TileOptions["size"])) {
    out.size = obj.size as TileOptions["size"];
  }
  if (typeof obj.dim === "string") out.dim = obj.dim;
  if (typeof obj.text === "string") out.text = obj.text;
  if (typeof obj.range === "string") out.range = obj.range;
  if (typeof obj.branch === "string") out.branch = obj.branch;
  if (typeof obj.alertOffTarget === "boolean") out.alertOffTarget = obj.alertOffTarget;
  return out;
}

/** Grid footprint for a tile size — `full` spans every column (a heading band),
 *  `wide` two columns, `tall` two rows, anything else stays 1×1. Returned as a
 *  style fragment the grid item spreads. */
export function tileSpanStyle(size: TileOptions["size"]): { gridColumn?: string; gridRow?: string } {
  if (size === "full") return { gridColumn: "1 / -1" };
  if (size === "wide") return { gridColumn: "span 2" };
  if (size === "tall") return { gridRow: "span 2" };
  return {};
}

/** The window a tile actually renders: the dashboard-level filter, with any
 *  per-tile override winning. A tile `range` of `all` explicitly means "no time
 *  window" (so a tile can opt out of a windowed dashboard). Pure — `now` is
 *  passed in so preset resolution is testable. */
export function resolveTileWindow(
  opts: TileOptions,
  dashboard: { range: TimeRange | null; branch: string | null },
  now: number,
): { range: TimeRange | null; branch: string | null } {
  const range = opts.range
    ? opts.range === "all"
      ? null
      : rangeFromPreset(opts.range, now)
    : dashboard.range;
  return { range, branch: opts.branch ?? dashboard.branch };
}

/** The newest sample's value (largest `captured_at`), or `null` when nothing
 *  parses. Order-independent — reuses the same `seriesPoints` sort the trend
 *  chart uses, so the "latest" is consistent with what the chart plots. */
export function latestValue(samples: SeriesPoint[]): number | null {
  const pts = seriesPoints(samples);
  return pts.length ? pts[pts.length - 1]!.v : null;
}

/** A dashboard's saved default **view** — what the filter row was set to when
 *  the user hit Save, restored the next time the dashboard is opened (tsk151).
 *  Persisted as the dashboard row's opaque `settings_json`. */
export interface DashboardSettings {
  /** {@link RANGE_PRESETS} key, or `all`. */
  range?: string;
  branch?: string;
  filterDim?: string;
  filterValue?: string;
}

/** Parse a dashboard's `settings_json`, tolerating null / blank / malformed
 *  JSON (→ `{}`) and dropping any non-string field. A bad blob means "no saved
 *  view" rather than seeding the filter row with junk — same defensive posture
 *  as {@link parseTileOptions}. */
export function parseDashboardSettings(json: string | null | undefined): DashboardSettings {
  if (!json) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (typeof raw !== "object" || raw === null) return {};
  const obj = raw as Record<string, unknown>;
  const out: DashboardSettings = {};
  if (typeof obj.range === "string") out.range = obj.range;
  if (typeof obj.branch === "string") out.branch = obj.branch;
  if (typeof obj.filterDim === "string") out.filterDim = obj.filterDim;
  if (typeof obj.filterValue === "string") out.filterValue = obj.filterValue;
  return out;
}

/** The dimensions a whole dashboard can break out by: the sorted union of what
 *  each tile's metric is sliceable by. A dimension only *some* metrics declare
 *  still appears — tiles that lack it render normally, greyed, rather than
 *  disappearing (tsk150). Delegates per-metric to the canonical
 *  {@link breakdownDimensions} so the dashboard and the metric detail page can't
 *  disagree about what a metric slices by. */
export function dashboardBreakoutDims(defs: MetricSpec[]): string[] {
  const all = new Set<string>();
  for (const def of defs) for (const d of breakdownDimensions(def)) all.add(d);
  return [...all].sort();
}

/**
 * Whether a tile's series is scoped to the dashboard's selected dimension value
 * — e.g. "show me everything for package `crates/oxplow-app`" (tsk150).
 *
 * The tile keeps its own visualization either way; this only decides which
 * points feed it. Same shape as the metric-detail page's breakdown-row click,
 * which charts one group's series.
 *
 * `notApplicable` (→ render normally, dimmed) has **two** causes:
 *  1. the metric's spec doesn't declare the dimension at all;
 *  2. it declares it but has no data under the selected value — a metric can
 *     be sliceable by `package` and simply have no facts in *that* package.
 *
 * A dimension chosen with **no value yet** is not a filter and not dimmed: the
 * tile shows everything until the user narrows it. While groups are still
 * loading the tile is treated as filtered, so it doesn't flash dimmed on the
 * way to its scoped data.
 */
export function resolveGroupFilter(
  dim: string | null | undefined,
  value: string | null | undefined,
  dims: string[],
  groups: { loaded: boolean; values: string[] },
): { filtered: boolean; notApplicable: boolean } {
  if (!dim) return { filtered: false, notApplicable: false };
  if (!dims.includes(dim)) return { filtered: false, notApplicable: true };
  if (!value) return { filtered: false, notApplicable: false };
  if (groups.loaded && !groups.values.includes(value)) {
    return { filtered: false, notApplicable: true };
  }
  return { filtered: true, notApplicable: false };
}

/** Whether a change is good / bad / neutral given the metric's preferred
 *  `direction` (`higher-better` | `lower-better` | `neutral`). A zero delta or
 *  a neutral/unknown direction is `neutral` (no color). Drives the number
 *  tile's delta chip color. */
export function deltaTone(delta: number, direction: string): "good" | "bad" | "neutral" {
  if (delta === 0) return "neutral";
  if (direction === "higher-better") return delta > 0 ? "good" : "bad";
  if (direction === "lower-better") return delta > 0 ? "bad" : "good";
  return "neutral";
}

// NOTE: an earlier revision defined its own `CATEGORY_ORDER` + `buildAddMetricMenu`
// here, which grouped metrics differently from the Recorded Metrics page. Metric
// sectioning has exactly one home — `buildMetricSections` in
// `pages/metricCategories.ts` — and the picker now goes through it via
// `components/Dashboard/metricPicker.ts` (tsk145). Don't reintroduce a local
// category table.

/** Build the metric-detail "Add to dashboard ▾" menu: one entry per existing
 *  dashboard, then (when there are any) a separator and **New dashboard…**.
 *  Picking a dashboard calls `onPick(dashboardId)`; the last entry calls
 *  `onNew()`. Pure — the caller owns the `addDashboardItem` write (tsk143). */
export function buildAddToDashboardMenu(
  dashboards: Dashboard[],
  onPick: (dashboardId: string) => void,
  onNew: () => void,
): MenuItem[] {
  const rows: MenuItem[] = dashboards.map((d) => ({
    id: `add-to-dash:${d.id}`,
    label: d.title,
    enabled: true,
    run: () => onPick(d.id),
  }));
  if (rows.length > 0) rows.push({ id: "add-to-dash-sep", label: "", enabled: false, separator: true });
  rows.push({ id: "add-to-dash-new", label: "New dashboard…", enabled: true, run: onNew });
  return rows;
}
