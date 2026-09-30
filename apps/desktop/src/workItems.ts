/**
 * Work items read through the models (P6.E1a, `.context/work-items.md`):
 * `v_work_item` for every provider's items, joined to `v_task` for
 * oxplow's own fields (thread, position, priority, notes). Each read
 * returns what it read (`reads`) so a page re-runs with
 * `useRerunOnChange`. Writes are `work_item.*` commands.
 */
import { querySql, runCommand, type SqlCell } from "./api.js";
import { taskIdOf, threadIdOf, threadRowId } from "./modelIds.js";
import type { Reads, SqlQueryResult, TaskPriority, TaskStatus } from "./tauri-bridge/generated/bindings.js";

/** The state every provider maps to. */
export type CanonicalState = "todo" | "in_progress" | "blocked" | "done" | "canceled";

export const CANONICAL_STATES: readonly CanonicalState[] = ["todo", "in_progress", "blocked", "done", "canceled"];

/** oxplow's own fields, when the item is one of its tasks. */
export interface TaskFields {
  /** `tsk42`. */
  id: string;
  /** `thr3`, or null on the backlog. */
  threadId: string | null;
  status: TaskStatus;
  priority: TaskPriority;
  sortIndex: number;
  author: string | null;
  completedAt: string | null;
  noteCount: number;
}

export interface WorkItem {
  ref: string;
  provider: string;
  title: string;
  body: string;
  state: CanonicalState;
  nativeState: string;
  parentRef: string | null;
  createdAt: string;
  updatedAt: string;
  task: TaskFields | null;
}

const COLUMNS = `w.ref, w.provider, w.title, w.body, w.state, w.native_state, w.parent_ref, w.created_at, w.updated_at,
  t.id AS task_id, t.thread_id, t.status, t.priority, t.sort_index, t.author, t.completed_at,
  (SELECT count(*) FROM v_task_note n WHERE n.task_id = t.id) AS note_count`;

const FROM = `FROM v_work_item w LEFT JOIN v_task t ON w.ref = 'work_item:oxplow:tsk' || t.id`;

export type WorkItemScope = { thread: string } | "backlog" | "all";

/** The SQL for a list: a thread's, the backlog's, or every item; in
 *  list order (oxplow's `sort_index`), then oldest first. */
export function workItemsQuery(opts: {
  scope: WorkItemScope;
  states?: CanonicalState[];
  /** Leave out oxplow's archived tasks (tidied away, not a state). */
  hideArchived?: boolean;
}): { sql: string; params: SqlCell[] } {
  const where: string[] = [];
  const params: SqlCell[] = [];
  if (opts.scope === "backlog") where.push("t.id IS NOT NULL AND t.thread_id IS NULL");
  else if (opts.scope !== "all") {
    params.push(threadRowId(opts.scope.thread));
    where.push("t.thread_id = ?1");
  }
  if (opts.hideArchived) where.push("w.native_state IS NOT 'archived'");
  if (opts.states && opts.states.length > 0) {
    where.push(`w.state IN (${opts.states.map((s) => `'${s}'`).join(", ")})`);
  }
  const sql = `SELECT ${COLUMNS} ${FROM}${where.length ? ` WHERE ${where.join(" AND ")}` : ""}
    ORDER BY t.sort_index IS NULL, t.sort_index, w.created_at`;
  return { sql, params };
}

const text = (c: SqlCell | undefined): string | null => (c === null || c === undefined ? null : String(c));

export function itemsFromResult(result: SqlQueryResult): WorkItem[] {
  const col = (name: string) => result.columns.indexOf(name);
  const at = (row: SqlCell[], name: string) => row[col(name)];
  return result.rows.map((row) => {
    const taskId = at(row, "task_id");
    const thread = at(row, "thread_id");
    return {
      ref: String(at(row, "ref")),
      provider: String(at(row, "provider")),
      title: String(at(row, "title") ?? ""),
      body: String(at(row, "body") ?? ""),
      state: String(at(row, "state")) as CanonicalState,
      nativeState: String(at(row, "native_state") ?? ""),
      parentRef: text(at(row, "parent_ref")),
      createdAt: String(at(row, "created_at") ?? ""),
      updatedAt: String(at(row, "updated_at") ?? ""),
      task:
        taskId === null || taskId === undefined
          ? null
          : {
              id: taskIdOf(Number(taskId)),
              threadId: thread === null || thread === undefined ? null : threadIdOf(Number(thread)),
              status: String(at(row, "status")) as TaskStatus,
              priority: String(at(row, "priority")) as TaskPriority,
              sortIndex: Number(at(row, "sort_index") ?? 0),
              author: text(at(row, "author")),
              completedAt: text(at(row, "completed_at")),
              noteCount: Number(at(row, "note_count") ?? 0),
            },
    };
  });
}

/** A list of work items, with what it read. */
export async function readWorkItems(opts: {
  scope: WorkItemScope;
  states?: CanonicalState[];
  hideArchived?: boolean;
}): Promise<{ items: WorkItem[]; reads: Reads }> {
  const q = workItemsQuery(opts);
  const res = await querySql(q.sql, q.params, 10_000);
  return { items: itemsFromResult(res), reads: res.reads };
}

export interface BoardColumn {
  state: CanonicalState;
  items: WorkItem[];
}

/** The Board: one column per canonical state, in workflow order. */
export function boardColumns(items: WorkItem[]): BoardColumn[] {
  return CANONICAL_STATES.map((state) => ({ state, items: items.filter((i) => i.state === state) }));
}

/** oxplow's status for a canonical state (`todo` is `ready`). */
export function statusFor(state: CanonicalState): TaskStatus {
  return state === "todo" ? "ready" : state;
}

/** Move an item to a canonical state (`work_item.transition`). */
export async function transitionWorkItem(ref: string, state: CanonicalState): Promise<void> {
  await runCommand("work_item.transition", { ref, to: statusFor(state) });
}

/** Put an item before or after another in its list (`work_item.reorder`). */
export async function reorderWorkItem(ref: string, place: { before: string } | { after: string } | "end"): Promise<void> {
  await runCommand("work_item.reorder", place === "end" ? { ref } : { ref, ...place });
}

/** Take an item to a thread's list or the backlog (`work_item.move`). */
export async function moveWorkItem(
  ref: string,
  to: { thread: string } | "backlog",
  place: { before: string } | { after: string } | "end" = "end",
): Promise<void> {
  await runCommand("work_item.move", place === "end" ? { ref, to } : { ref, to, ...place });
}
