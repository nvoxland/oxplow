// Row filtering for the Metrics page (tsk87). Pure — React-free.

import type { TabRef } from "../tabs/tabState.js";

/** Which metrics the page lists: enabled only (default), every catalogued
 *  metric, or just the enabled ones currently missing their target (tsk121). */
export type ShowMode = "enabled" | "all" | "off-target";

/**
 * Default to **enabled**. The page's row set is the metric CATALOG, which
 * includes metrics that are seeded as specs but never `use:`d in
 * `.oxplow/project.yaml` — the bundled C#/Clojure idiom gauges, say, in a
 * Rust/TS repo. Those gauges never run (`resolved_gauges` elides them), so they
 * can only ever render `—`. Listing them by default makes the page look like it
 * tracks things it doesn't.
 */
export const DEFAULT_SHOW_MODE: ShowMode = "enabled";

export const SHOW_MODES: ReadonlyArray<{ key: ShowMode; label: string }> = [
  { key: "enabled", label: "Enabled" },
  { key: "all", label: "All" },
  { key: "off-target", label: "Off target" },
];

/** Apply the Show mode + the search box. Pure; incoming order is preserved.
 *  The two compose — a search never resurfaces a disabled metric while the mode
 *  is `enabled`. **`off-target` narrows the same way `enabled` does here** (only
 *  enabled rows + query): filterMetricRows can't see values, so
 *  MetricsPage applies the value-based `isOffTarget` test afterwards,
 *  against each row's latest value within the selected range/branch window. */
export function filterMetricRows<T extends { key: string; title: string; enabled: boolean }>(
  rows: readonly T[],
  showMode: ShowMode,
  query: string,
): T[] {
  const q = query.trim().toLowerCase();
  return rows.filter((r) => {
    if (showMode !== "all" && !r.enabled) return false;
    if (!q) return true;
    return r.title.toLowerCase().includes(q) || r.key.toLowerCase().includes(q);
  });
}

/** The subset of a metric spec that determines target status. `MetricSpec`
 *  satisfies it structurally, so callers pass their `def` straight in. */
export type StatusSpec = {
  direction: string;
  target: number | null;
  warn_at: number | null;
  fail_at: number | null;
};

/** How a value stands against its target: `ok` meets the target/warn
 *  threshold, `fail` is past `fail_at`, `warn` is any other threshold miss,
 *  `none` means there's no target (or a neutral metric) to be off of. The one
 *  classifier `statusColor` and the Off-target filter both read, so a color and
 *  a filter can't disagree about whether a row is meeting its target. */
export type MetricStatus = "ok" | "warn" | "fail" | "none";

export function metricStatus(def: StatusSpec, value: number): MetricStatus {
  if (def.direction === "neutral") return "none";
  const higher = def.direction === "higher-better";
  const meets = (t: number) => (higher ? value >= t : value <= t);
  const okThreshold = def.target ?? def.warn_at;
  if (okThreshold != null && meets(okThreshold)) return "ok";
  if (def.fail_at != null && !meets(def.fail_at)) return "fail";
  if (okThreshold != null || def.fail_at != null) return "warn";
  return "none";
}

/** True when a metric's value misses its target (warn or fail). A metric with
 *  no threshold, a neutral one, or a pruned spec (`null`) is never off target —
 *  there's nothing to miss. */
export function isOffTarget(def: StatusSpec | null, value: number): boolean {
  if (!def) return false;
  const status = metricStatus(def, value);
  return status === "warn" || status === "fail";
}

/** The color for a value judged against its target — the rendering half of
 *  {@link metricStatus}. `undefined` when there's no threshold to judge by.
 *
 *  Lives here, next to the classifier, because more than one surface paints
 *  this: the Metrics rows and the dashboard tiles' off-target
 *  highlight (tsk149). Keeping the mapping in one place is the same rule the
 *  classifier itself follows — a second copy would let two surfaces disagree
 *  about what "off target" looks like. */
export function metricStatusColor(def: StatusSpec, value: number): string | undefined {
  switch (metricStatus(def, value)) {
    case "ok":
      return "var(--ok, #3fb950)";
    case "fail":
      return "var(--err, #f85149)";
    case "warn":
      return "var(--warn, #e5a50a)";
    default:
      return undefined;
  }
}

/**
 * The sibling-navigation chain for the page's rows (tsk119): the rendered
 * sections flattened in render order, so the nav-bar up/down buttons on a
 * drilled-into metric detail walk the list exactly as the page displays it —
 * continuing across section boundaries. `indexByKey` gives each row its own
 * position for the `NavSiblings.index` it dispatches with.
 */
export function metricSiblings<T extends { key: string; title: string }>(
  sections: ReadonlyArray<{ entries: T[] }>,
  makeRef: (key: string) => TabRef,
): { entries: Array<{ ref: TabRef; label: string }>; indexByKey: Map<string, number> } {
  const entries: Array<{ ref: TabRef; label: string }> = [];
  const indexByKey = new Map<string, number>();
  for (const section of sections) {
    for (const row of section.entries) {
      indexByKey.set(row.key, entries.length);
      entries.push({ ref: makeRef(row.key), label: row.title });
    }
  }
  return { entries, indexByKey };
}
