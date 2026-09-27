import type { Dashboard, SeriesPoint } from "../api.js";
import type { MenuItem } from "../menu.js";
import { type TimeRange, rangeFromPreset, seriesPoints } from "./metricDetailData.js";

// Pure helpers behind the custom-dashboard page + tiles (tsk141, epic tsk138).
// Kept React-free so the tile-option parsing and the add-metric menu assembly
// are unit-testable without mounting the page — same split as
// `metricDetailData.ts`.

/** How one tile is rendered. Persisted as the tile's opaque `options_json`
 *  blob (so the shape grows with no migration). Everything is optional — an
 *  absent field falls back to the tile default. Tiles saved with the retired
 *  options (`sparkline` / `bar` viz, chart `mode` / `scale`, a breakdown
 *  `dim`, `alertOffTarget`) parse without them. */
export interface TileOptions {
  /** `line` (trend chart, charted the way the metric rolls up) | `number`
   *  (big headline value). Default `line`. */
  viz?: "line" | "number";
  /** Title override — else the metric's own title. */
  title?: string;
  /** Grid footprint: `wide` spans 2 columns, `tall` spans 2 rows, `full` spans
   *  the whole grid width (the heading-band size text tiles default to).
   *  Default `small` (1×1). */
  size?: "small" | "wide" | "tall" | "full";
  /** Heading text for a `text` tile. Plain text, not markdown — see `TextTile`. */
  text?: string;
  /** Lens id (`<extension>/<slug>`) for a `lens` tile — see `LensTile`. */
  lensId?: string;
  /** Per-tile time-range override: a {@link RANGE_PRESETS} key, or `all` for no
   *  window. Absent → inherit the dashboard's filter. */
  range?: string;
  /** Per-tile branch override. Absent → inherit the dashboard's filter. */
  branch?: string;
}

const VIZ = new Set<TileOptions["viz"]>(["line", "number"]);
const SIZES = new Set<TileOptions["size"]>(["small", "wide", "tall", "full"]);

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
  if (typeof obj.title === "string") out.title = obj.title;
  if (typeof obj.size === "string" && SIZES.has(obj.size as TileOptions["size"])) {
    out.size = obj.size as TileOptions["size"];
  }
  if (typeof obj.text === "string") out.text = obj.text;
  if (typeof obj.lensId === "string") out.lensId = obj.lensId;
  if (typeof obj.range === "string") out.range = obj.range;
  if (typeof obj.branch === "string") out.branch = obj.branch;
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
// here, which grouped metrics differently from the Metrics page. Metric
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
