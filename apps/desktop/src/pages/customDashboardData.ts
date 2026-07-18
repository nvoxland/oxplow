import type { MetricCatalogEntry, SeriesPoint } from "../api.js";
import type { MenuItem } from "../menu.js";
import { type ChartMode, type ChartScale, seriesPoints } from "./metricDetailData.js";

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
  /** `line` (a trend chart) | `number` (a big headline value). Default `line`. */
  viz?: "line" | "number";
  /** Chart transform for a `line` tile (value / cumulative / change / avg). */
  mode?: ChartMode;
  /** Y-axis scaling for a `line` tile (auto / from-zero). */
  scale?: ChartScale;
  /** Title override — else the metric's own title. */
  title?: string;
}

const VIZ = new Set<TileOptions["viz"]>(["line", "number"]);
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
  return out;
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
