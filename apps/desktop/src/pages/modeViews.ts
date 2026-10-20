/**
 * The view params of the pages whose view is a mode or a pick
 * (`.context/pages-and-tabs.md` "Page state"), declared in `BUILT_IN_VIEW`
 * (`pageIdentity.ts`):
 *
 * - Files: `filter`, which files the tree lists (`all`, `uncommitted`,
 *   `branch`, `unpushed`);
 * - Alerts: `open`, the row opened (a pick);
 * - Board: `show` (`thread`, `backlog`, `all`).
 */

export const isFilesViewParam = (key: string) => key === "filter";
export const isAlertsViewParam = (key: string) => key === "open";
export const isBoardViewParam = (key: string) => key === "show";
