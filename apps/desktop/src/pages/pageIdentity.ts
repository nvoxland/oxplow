/**
 * A page's identity: its ref without its **view params** — what the page
 * is, not how it is viewed (`.context/pages-and-tabs.md` "Live by
 * default"). A tab keeps one mounted body per identity (`bodies`), so a
 * change of view re-renders the page and never remounts it, and a visit
 * counts the page once whatever its filters.
 *
 * Which params are view:
 * - a lens's: all of them (its declared params);
 * - an extension's page: all but `ref`, which names the thing it opened
 *   for (`page:<ext>/<page>?ref={ref}`);
 * - one of oxplow's own pages: the ones it declares here
 *   (`BUILT_IN_VIEW`); a page that declares none is all identity (a diff's
 *   path and sides, a dashboard's id).
 */

import { isTasksViewParam } from "../components/Plan/tasksView.js";
import { identityOf } from "../tabs/session.js";
import { extPageRef, pagePayload, pageTabRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { isBuiltInPage } from "./declaredPages.js";

/** Each built-in page's view params, by page name. */
const BUILT_IN_VIEW: Record<string, (key: string) => boolean> = {
  tasks: isTasksViewParam,
  backlog: isTasksViewParam,
};

export function pageIdentity(ref: TabRef): string {
  if (ref.kind === "lens") return identityOf(ref.id);
  const page = pagePayload(ref);
  if (!page) return ref.id;
  if (page.extension !== null) {
    const opened = page.params.ref;
    return opened === undefined ? identityOf(ref.id) : extPageRef(page.extension, page.page, { ref: opened }).id;
  }
  if (!isBuiltInPage(page.page)) return identityOf(ref.id);
  const isView = BUILT_IN_VIEW[page.page];
  if (!isView) return ref.id;
  return pageTabRef(page.page, Object.fromEntries(Object.entries(page.params).filter(([key]) => !isView(key)))).id;
}
