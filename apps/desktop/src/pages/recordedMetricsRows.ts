// Row filtering for the Recorded Metrics page (tsk87). Pure — React-free.

import type { TabRef } from "../tabs/tabState.js";

/** Which metrics the page lists. */
export type ShowMode = "enabled" | "all";

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
];

/** Apply the Show mode + the search box. Pure; incoming order is preserved.
 *  The two compose — a search never resurfaces a disabled metric while the mode
 *  is `enabled`. */
export function filterMetricRows<T extends { key: string; title: string; enabled: boolean }>(
  rows: readonly T[],
  showMode: ShowMode,
  query: string,
): T[] {
  const q = query.trim().toLowerCase();
  return rows.filter((r) => {
    if (showMode === "enabled" && !r.enabled) return false;
    if (!q) return true;
    return r.title.toLowerCase().includes(q) || r.key.toLowerCase().includes(q);
  });
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
