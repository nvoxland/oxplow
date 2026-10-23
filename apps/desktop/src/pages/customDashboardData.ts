import type { Dashboard, SeriesPoint } from "../api.js";
import { metricSeriesSql } from "../metricsSql.js";
import type { MenuItem } from "../menu.js";
import { seriesPoints } from "./metricDetailData.js";

// Pure helpers behind the dashboard tiles and the metric page's "Add to
// dashboard" menu. Kept React-free so they are unit-testable without mounting
// anything — same split as `metricDetailData.ts`.

/** A new tile showing metric `key` as the metric card: a query tile over its
 *  captures (P4.7). */
export function metricTile(key: string): { kind: "query"; sql: string; display: "metric"; optionsJson: string } {
  return {
    kind: "query",
    sql: metricSeriesSql(key),
    display: "metric",
    optionsJson: JSON.stringify({ metric: key, viz: "line" }),
  };
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
