/**
 * The Tasks page's view — which threads it shows (its scope) and its
 * filter chips — as the page's **view params** (`.context/pages-and-tabs.md`
 * "Page state"): in its tab's address, so two Tasks tabs can differ, Back
 * undoes a change, a bookmark keeps it and an agent reading the open page
 * sees it. A change replaces the tab's entry (a filter, not a pick).
 *
 * Params: `scope` (`thread` with `stream` and `thread`, `stream` with
 * `stream`, or `all`; none for the thread the app shows) and one
 * `f.<field>` per filtered field, its values joined by commas. A default
 * writes nothing.
 */

import { useState } from "react";

import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import type { SqlCell } from "../../tauri-bridge/generated/bindings.js";
import type { FieldFilter } from "./plan-utils.js";
import type { TasksScope } from "./tasks-scope.js";

export interface TasksView {
  scope: TasksScope;
  filter: FieldFilter;
}

const FILTER = "f.";
const SCOPE_KEYS = ["scope", "stream", "thread"];

/** Whether `key` is one of the Tasks page's view params. */
export function isTasksViewParam(key: string): boolean {
  return SCOPE_KEYS.includes(key) || key.startsWith(FILTER);
}

const text = (v: SqlCell | undefined) => (typeof v === "string" ? v : typeof v === "number" ? String(v) : "");

/** The view `params` say; its defaults for what they don't. */
export function tasksViewOf(params: Record<string, SqlCell>): TasksView {
  const stream = text(params.stream);
  const thread = text(params.thread);
  const kind = text(params.scope);
  const scope: TasksScope =
    kind === "thread" && stream && thread
      ? { kind: "thread", streamId: stream, threadId: thread }
      : kind === "stream" && stream
        ? { kind: "stream", streamId: stream }
        : kind === "all"
          ? { kind: "all" }
          : { kind: "currentThread" };
  const filter: Record<string, string[]> = {};
  for (const [key, value] of Object.entries(params)) {
    if (!key.startsWith(FILTER)) continue;
    const values = text(value).split(",").filter(Boolean);
    if (values.length > 0) filter[key.slice(FILTER.length)] = values;
  }
  return { scope, filter };
}

/** `view` as view params, the defaults left out. */
export function tasksViewParams(view: TasksView): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  const s = view.scope;
  if (s.kind === "thread") Object.assign(out, { scope: "thread", stream: s.streamId, thread: s.threadId });
  else if (s.kind === "stream") Object.assign(out, { scope: "stream", stream: s.streamId });
  else if (s.kind === "all") out.scope = "all";
  for (const [name, values] of Object.entries(view.filter)) {
    if (values.length > 0) out[`${FILTER}${name}`] = values.join(",");
  }
  return out;
}

/** The Tasks page's view, and how to change it: its tab's view params in a
 *  tab (other params kept), its own state outside one. */
export function useTasksView(): [TasksView, (next: TasksView) => void] {
  const nav = useOptionalPageNavigation();
  const [own, setOwn] = useState<TasksView>({ scope: { kind: "currentThread" }, filter: {} });
  if (!nav?.setViewParams) return [own, setOwn];
  const params = nav.viewParams ?? {};
  const set = (next: TasksView) => {
    const kept = Object.fromEntries(Object.entries(params).filter(([k]) => !SCOPE_KEYS.includes(k) && !k.startsWith(FILTER)));
    nav.setViewParams!({ ...kept, ...tasksViewParams(next) });
  };
  return [tasksViewOf(params), set];
}
