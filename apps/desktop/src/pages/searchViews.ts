/**
 * The view params of the search pages (`.context/pages-and-tabs.md` "Page
 * state"), declared in `BUILT_IN_VIEW` (`pageIdentity.ts`):
 *
 * - Comments: `days`, how many days of resolved threads to show (none:
 *   unresolved only);
 * - Settings: `q`, the search;
 * - New Session: `q`, the filter on the earlier sessions;
 * - Explore Data: `model` (the pick), `sql` (the query that ran), `viz`,
 *   `chart` (JSON), `template` (a metric, then its dimension) and `raw`.
 */

export const isCommentsViewParam = (key: string) => key === "days";
export const isSettingsViewParam = (key: string) => key === "q";
export const isExploreViewParam = (key: string) => ["model", "sql", "viz", "chart", "template", "raw"].includes(key);
export const isNewSessionViewParam = (key: string) => key === "q";
