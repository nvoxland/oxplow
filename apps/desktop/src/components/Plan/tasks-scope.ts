import { bucketWorkList, type WorkList } from "../../workItems.js";
import { unionReads } from "../../lens/lensRerun.js";

/**
 * Tasks page view scope. The page can show:
 *  - currentThread: the thread the rest of the app is focused on
 *  - thread: a specific (stream, thread) the user picked
 *  - stream: every thread in the picked stream, merged together
 *  - all: every thread in every stream, merged together
 *
 * Cross-thread/stream views are read-only — mutations are scoped to a
 * single (stream, thread) by the existing handlers, so the picker
 * disables them when the view spans multiple threads.
 */
export type TasksScope =
  | { kind: "currentThread" }
  | { kind: "thread"; streamId: string; threadId: string }
  | { kind: "stream"; streamId: string }
  | { kind: "all" };

export const STORAGE_KEY = "tasks-scope";

export function loadScope(): TasksScope {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { kind: "currentThread" };
    const parsed = JSON.parse(raw) as TasksScope;
    if (parsed && typeof parsed === "object" && "kind" in parsed) return parsed;
  } catch {}
  return { kind: "currentThread" };
}

export function saveScope(scope: TasksScope): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(scope));
  } catch {}
}

/**
 * Merge several threads' lists into one so the TasksList/PlanPane render
 * path can show items across threads. The result's `threadId` is empty
 * since the rows came from many threads; the page treats it as read-only.
 */
export function mergeThreadWork(lists: WorkList[]): WorkList {
  return bucketWorkList(
    "",
    lists.flatMap((l) => l.all),
    lists.flatMap((l) => l.followups),
    unionReads(lists.map((l) => l.reads)),
  );
}

export function isReadOnlyScope(scope: TasksScope): boolean {
  return scope.kind === "stream" || scope.kind === "all";
}
