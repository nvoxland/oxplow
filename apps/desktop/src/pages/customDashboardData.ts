import type { Dashboard, MetricCatalogEntry, SeriesPoint } from "../api.js";
import type { MenuItem } from "../menu.js";
import {
  type ChartMode,
  type ChartScale,
  type TimeRange,
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
  /** Grid footprint: `wide` spans 2 columns, `tall` spans 2 rows. Default
   *  `small` (1×1). */
  size?: "small" | "wide" | "tall";
  /** Breakdown dimension for a `bar` tile (e.g. `package`, `language`). */
  dim?: string;
  /** Markdown body for a `text` tile. */
  text?: string;
  /** Per-tile time-range override: a {@link RANGE_PRESETS} key, or `all` for no
   *  window. Absent → inherit the dashboard's filter. */
  range?: string;
  /** Per-tile branch override. Absent → inherit the dashboard's filter. */
  branch?: string;
}

const VIZ = new Set<TileOptions["viz"]>(["line", "number", "sparkline", "bar"]);
const SIZES = new Set<TileOptions["size"]>(["small", "wide", "tall"]);
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
  return out;
}

/** Grid footprint for a tile size — `wide` spans two columns, `tall` two rows,
 *  anything else stays 1×1. Returned as a style fragment the grid item spreads. */
export function tileSpanStyle(size: TileOptions["size"]): { gridColumn?: string; gridRow?: string } {
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

// Category grouping for the add-metric picker — mirrors the Catalog page's
// `category` axis (`operational` | `testing` | `static-quality` | `custom`;
// null → "other"), in a fixed display order.
const CATEGORY_ORDER: Array<{ key: string; label: string }> = [
  { key: "operational", label: "Operational" },
  { key: "testing", label: "Testing" },
  { key: "static-quality", label: "Static quality" },
  { key: "custom", label: "Custom" },
  { key: "other", label: "Other" },
];

/** Build the right-click "Add metric" menu: one submenu per non-empty category
 *  (in {@link CATEGORY_ORDER}), each listing its metrics alphabetically by
 *  title. Picking a leaf calls `onPick(metricKey)`. Pure — the caller owns the
 *  actual `addDashboardItem` write. */
export function buildAddMetricMenu(
  catalog: MetricCatalogEntry[],
  onPick: (metricKey: string) => void,
): MenuItem[] {
  const byCategory = new Map<string, MetricCatalogEntry[]>();
  for (const e of catalog) {
    const cat = e.category && CATEGORY_ORDER.some((c) => c.key === e.category) ? e.category : "other";
    const bucket = byCategory.get(cat) ?? [];
    bucket.push(e);
    byCategory.set(cat, bucket);
  }
  const menu: MenuItem[] = [];
  for (const { key, label } of CATEGORY_ORDER) {
    const entries = byCategory.get(key);
    if (!entries || entries.length === 0) continue;
    const submenu = [...entries]
      .sort((a, b) => a.title.localeCompare(b.title))
      .map<MenuItem>((e) => ({
        id: `add-metric:${e.key}`,
        label: e.title,
        enabled: true,
        run: () => onPick(e.key),
      }));
    menu.push({ id: `add-metric-cat:${key}`, label, enabled: true, submenu });
  }
  return menu;
}

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
