/**
 * The view params of the metric pages (`.context/pages-and-tabs.md` "Page
 * state"): what each shows is in its tab's address. They are declared in
 * `BUILT_IN_VIEW` (`pageIdentity.ts`) so changing one re-renders the page.
 *
 * - Metrics: `range`, `branch`, `slice`, `q` (the name filter) and `show`.
 * - A custom dashboard: `range` and `branch`; its `id` is its identity.
 *
 * `range` is a range value (`kit/rangePresets.ts`); `branch` and `slice`
 * are empty for every branch and no slice, and are then left out.
 */

const METRICS = ["range", "branch", "slice", "q", "show"];
const DASHBOARD = ["range", "branch"];

export const isMetricsViewParam = (key: string) => METRICS.includes(key);
export const isDashboardViewParam = (key: string) => DASHBOARD.includes(key);
