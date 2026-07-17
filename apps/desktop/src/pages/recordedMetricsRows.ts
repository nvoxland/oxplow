// Row filtering for the Recorded Metrics page (tsk87). Pure — React-free.

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
