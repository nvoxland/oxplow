/**
 * The view params of the search pages (`.context/pages-and-tabs.md` "Page
 * state"), declared in `BUILT_IN_VIEW` (`pageIdentity.ts`):
 *
 * - Git History: `q` (text), `author`, `branch` — empty for none;
 * - Comments: `days`, how many days of resolved threads to show (none:
 *   unresolved only);
 * - Settings: `q`, the search;
 * - Explore Data: `model` (the pick), `sql` (the query that ran), `viz`,
 *   `chart` (JSON), `template` (a metric, then its dimension) and `raw`.
 */

export const isGitHistoryViewParam = (key: string) => ["q", "author", "branch"].includes(key);
export const isCommentsViewParam = (key: string) => key === "days";
export const isSettingsViewParam = (key: string) => key === "q";
export const isExploreViewParam = (key: string) => ["model", "sql", "viz", "chart", "template", "raw"].includes(key);
