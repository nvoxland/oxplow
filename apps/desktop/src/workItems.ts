/**
 * Work items read through the models (P6.E1a, `.context/work-items.md`):
 * `v_work_item` for every provider's items, joined to `v_task` for
 * oxplow's own fields (thread, position, priority, notes). Each read
 * returns what it read (`reads`) so a page re-runs with
 * `useRerunOnChange`. Writes are `work_item.*` commands.
 */
import { querySql, runCommand, type EffortDetail, type SqlCell } from "./api.js";
import { NO_READS } from "./lens/lensRerun.js";
import { taskIdOf, taskRowId, threadIdOf, threadRowId } from "./modelIds.js";
import type { Followup, Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { commands } from "./tauri-bridge/index.js";

/** An oxplow task's status (`v_task.status`). */
export type TaskStatus = "ready" | "in_progress" | "blocked" | "done" | "canceled" | "archived";
export type TaskPriority = "low" | "medium" | "high" | "urgent";
export type TaskAuthor = "user" | "agent";

/** An oxplow task as `v_task` holds it, with the UI's ids (`tsk42`,
 *  `thr3`) and its note count. */
export interface Task {
  id: string;
  /** `null` on the project-wide backlog. */
  thread_id: string | null;
  parent_id: string | null;
  title: string;
  description: string;
  status: TaskStatus;
  priority: TaskPriority;
  sort_index: number;
  /** Who it came from. */
  author: TaskAuthor | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  note_count: number;
}

/** A thread's tasks as the Work panel shows them. An epic is a task with
 *  a child in the thread; the rest go by status (`done` holds done,
 *  canceled and archived). Followups are the thread's in-memory notes. */
export interface ThreadWorkState {
  threadId: string;
  waiting: Task[];
  inProgress: Task[];
  done: Task[];
  epics: Task[];
  items: Task[];
  followups: Followup[];
  /** What the read read: a change to one of these models re-reads it. */
  reads: Reads;
}

/** The backlog's tasks by status. */
export interface BacklogState {
  items: Task[];
  waiting: Task[];
  in_progress: Task[];
  done: Task[];
  /** What the read read: a change to one of these models re-reads it. */
  reads: Reads;
}

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

// ---- oxplow's tasks (v_task) ----
//
// Every read returns what it read (`reads`); a consumer re-runs it through
// `useRerunOnChange` (or `readsChanged`) when one of those models changes.
// There is no list of "task models" to keep in step with the queries.

const TASK_COLUMNS = `t.id, t.thread_id, t.parent_id, t.title, t.description, t.status, t.priority, t.sort_index,
  t.author, t.created_at, t.updated_at, t.completed_at,
  (SELECT count(*) FROM v_task_note n WHERE n.task_id = t.id) AS note_count`;

export function tasksFromResult(result: SqlQueryResult): Task[] {
  const col = (name: string) => result.columns.indexOf(name);
  return result.rows.map((row) => {
    const at = (name: string) => row[col(name)];
    const num = (name: string) => {
      const v = at(name);
      return v === null || v === undefined ? null : Number(v);
    };
    const thread = num("thread_id");
    const parent = num("parent_id");
    return {
      id: taskIdOf(Number(at("id"))),
      thread_id: thread === null ? null : threadIdOf(thread),
      parent_id: parent === null ? null : taskIdOf(parent),
      title: String(at("title") ?? ""),
      description: String(at("description") ?? ""),
      status: String(at("status")) as TaskStatus,
      priority: String(at("priority")) as TaskPriority,
      sort_index: Number(at("sort_index") ?? 0),
      author: (text(at("author")) as TaskAuthor | null) ?? null,
      created_at: String(at("created_at") ?? ""),
      updated_at: String(at("updated_at") ?? ""),
      completed_at: text(at("completed_at")),
      note_count: Number(at("note_count") ?? 0),
    };
  });
}

async function readTasks(where: string, params: SqlCell[]): Promise<{ tasks: Task[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ${TASK_COLUMNS} FROM v_task t WHERE ${where} ORDER BY t.sort_index, t.created_at`,
    params,
    10_000,
  );
  return { tasks: tasksFromResult(res), reads: res.reads };
}

export function bucketThreadWork(threadId: string, tasks: Task[], followups: Followup[], reads: Reads): ThreadWorkState {
  const parents = new Set(tasks.map((t) => t.parent_id).filter((p): p is string => p !== null));
  const work: ThreadWorkState = { threadId, waiting: [], inProgress: [], done: [], epics: [], items: [], followups, reads };
  for (const t of tasks) {
    if (parents.has(t.id)) work.epics.push(t);
    else if (t.status === "blocked") work.waiting.push(t);
    else if (t.status === "in_progress") work.inProgress.push(t);
    else if (t.status === "ready") work.items.push(t);
    else work.done.push(t);
  }
  return work;
}

/** Every task of a thread's work, in list order (`sort_index`). */
export function orderedTaskIds(work: ThreadWorkState): string[] {
  return [...work.epics, ...work.items, ...work.waiting, ...work.inProgress, ...work.done]
    .sort((a, b) => a.sort_index - b.sort_index)
    .map((t) => t.id);
}

/** A thread's work: its tasks from `v_task`, and its followups. */
export async function readThreadWork(threadId: string): Promise<ThreadWorkState> {
  const [{ tasks, reads }, followups] = await Promise.all([
    readTasks("t.thread_id = ?1", [threadRowId(threadId)]),
    commands.listFollowups(threadId).then((r) => (r.status === "ok" ? r.data : [])),
  ]);
  return bucketThreadWork(threadId, tasks, followups, reads);
}

/** The backlog's tasks by status. */
export async function readBacklog(): Promise<BacklogState> {
  const { tasks, reads } = await readTasks("t.thread_id IS NULL", []);
  const state: BacklogState = { items: [], waiting: [], in_progress: [], done: [], reads };
  for (const t of tasks) {
    if (t.status === "blocked") state.waiting.push(t);
    else if (t.status === "in_progress") state.in_progress.push(t);
    else if (t.status === "ready") state.items.push(t);
    else state.done.push(t);
  }
  return state;
}

/** One live task, or null. */
export async function readTask(id: string): Promise<{ task: Task | null; reads: Reads }> {
  const { tasks, reads } = await readTasks("t.id = ?1", [taskRowId(id)]);
  return { task: tasks[0] ?? null, reads };
}

/** Several tasks' titles and statuses, in one read. */
export async function readTasksById(ids: string[]): Promise<{ tasks: Task[]; reads: Reads }> {
  if (ids.length === 0) return { tasks: [], reads: NO_READS };
  const numbers = ids.map(taskRowId).filter((n) => Number.isFinite(n));
  return readTasks(`t.id IN (${numbers.map((_, i) => `?${i + 1}`).join(", ")})`, numbers);
}

/** `v_effort` rows joined to their `v_effort_file` rows (one row per
 *  effort and file; an effort with no files has one row with a null
 *  path), in effort order, as the task page's activity. */
export function effortDetailsFromResult(result: SqlQueryResult): EffortDetail[] {
  const col = (name: string) => result.columns.indexOf(name);
  const details: EffortDetail[] = [];
  let current: EffortDetail | null = null;
  for (const row of result.rows) {
    const at = (name: string) => row[col(name)];
    const id = `eff${Number(at("id"))}`;
    if (!current || current.effort.id !== id) {
      const snapshot = (name: string) => (at(name) === null ? null : String(at(name)));
      current = {
        effort: {
          id,
          work_item: String(at("work_item")),
          started_at: String(at("started_at")),
          ended_at: text(at("ended_at")),
          start_snapshot_id: snapshot("start_snapshot_id"),
          end_snapshot_id: snapshot("end_snapshot_id"),
          summary: text(at("summary")),
        },
        start_snapshot: null,
        end_snapshot: null,
        changed_paths: [],
        counts: { created: 0, updated: 0, deleted: 0 },
      };
      details.push(current);
    }
    const path = text(at("path"));
    if (path === null) continue;
    current.changed_paths.push(path);
    const kind = text(at("change_kind")) as keyof EffortDetail["counts"] | null;
    if (kind && kind in current.counts) current.counts[kind]++;
  }
  return details;
}

/** A task's efforts with the files each changed, newest first — one read
 *  over `v_effort` and `v_effort_file`. */
export async function readTaskEfforts(taskId: string): Promise<{ efforts: EffortDetail[]; reads: Reads }> {
  const res = await querySql(
    `SELECT e.id, e.work_item, e.started_at, e.ended_at, e.start_snapshot_id, e.end_snapshot_id, e.summary,
            f.path, f.change_kind
       FROM v_effort e
       LEFT JOIN v_effort_file f ON f.effort_id = e.id
      WHERE e.work_item = ?1
      ORDER BY e.started_at DESC, e.id DESC, f.path`,
    [`work_item:oxplow:${taskId}`],
    10_000,
  );
  return { efforts: effortDetailsFromResult(res), reads: res.reads };
}

// ---- writes (work_item.* commands) ----

const taskRef = (id: string) => `work_item:oxplow:${id}`;

/** File a task on a thread (or the backlog, `null`). */
export async function createTask(
  threadId: string | null,
  input: { title: string; description?: string; parentId?: string | null; status?: TaskStatus; priority?: TaskPriority },
): Promise<string> {
  const out = await runCommand("work_item.create", {
    title: input.title,
    ...(input.description ? { description: input.description } : {}),
    ...(input.parentId ? { parent_ref: taskRef(input.parentId) } : {}),
    ...(input.status ? { status: input.status } : {}),
    ...(input.priority ? { priority: input.priority } : {}),
    ...(threadId ? { thread: threadId } : {}),
  });
  return String((out.result as { ref?: unknown } | null)?.ref ?? "");
}

/** Edit a task's fields and/or status, atomically. */
export async function updateTask(
  id: string,
  changes: { title?: string; description?: string; parentId?: string | null; status?: TaskStatus; priority?: TaskPriority },
): Promise<void> {
  await runCommand("work_item.update", {
    ref: taskRef(id),
    ...(changes.title !== undefined ? { title: changes.title } : {}),
    ...(changes.description !== undefined ? { description: changes.description } : {}),
    ...(changes.parentId !== undefined ? { parent_ref: changes.parentId === null ? "" : taskRef(changes.parentId) } : {}),
    ...(changes.status !== undefined ? { status: changes.status } : {}),
    ...(changes.priority !== undefined ? { priority: changes.priority } : {}),
  });
}

/** Delete a task. The command asks first: call with `confirmed` once the
 *  person has (an inline confirm). */
export async function deleteTask(id: string, confirmed: boolean): Promise<void> {
  await runCommand("work_item.delete", { ref: taskRef(id) }, confirmed);
}

/** The one item a drag moved, and its new neighbour: what
 *  `work_item.reorder` takes. `null` when nothing moved. */
export function placementFromOrder(
  before: string[],
  after: string[],
): { id: string; place: { before: string } | { after: string } } | null {
  const same = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);
  if (same(before, after)) return null;
  for (let i = 0; i < after.length; i++) {
    const id = after[i]!;
    if (!same(before.filter((x) => x !== id), after.filter((x) => x !== id))) continue;
    return { id, place: i > 0 ? { after: after[i - 1]! } : { before: after[1]! } };
  }
  return null;
}

/** Apply a drag's new order to a list with `work_item.reorder`. */
export async function reorderTasks(before: string[], after: string[]): Promise<void> {
  const moved = placementFromOrder(before, after);
  if (moved) await runCommand("work_item.reorder", { ref: taskRef(moved.id), ...prefixed(moved.place) });
}

const prefixed = (place: { before: string } | { after: string }) =>
  "before" in place ? { before: taskRef(place.before) } : { after: taskRef(place.after) };

/** Take a task to a thread's list or the backlog (`null`), at its end. */
export async function moveTask(id: string, threadId: string | null): Promise<void> {
  await runCommand("work_item.move", { ref: taskRef(id), to: threadId ? { thread: threadId } : "backlog" });
}

// ---- recently finished (the rail's Finished section) ----

export type FinishedEntry =
  | { kind: "task"; itemId: string; title: string; t: string }
  | { kind: "wiki"; slug: string; title: string; t: string };

/** Newest first, after `clearedAt`, at most `limit`. */
export function recentlyFinished(entries: FinishedEntry[], clearedAt: string | null, limit: number): FinishedEntry[] {
  return entries
    .filter((e) => clearedAt === null || e.t > clearedAt)
    .sort((a, b) => (a.t < b.t ? 1 : a.t > b.t ? -1 : 0))
    .slice(0, limit);
}

/** What a thread (or, `null`, the project) recently finished: done
 *  tasks and the knowledge pages it wrote (`v_knowledge_touch`). */
export async function readRecentlyFinished(threadId: string | null, limit: number): Promise<{ entries: FinishedEntry[]; reads: Reads }> {
  const thread = threadId === null ? null : threadRowId(threadId);
  const [tasks, pages] = await Promise.all([
    querySql(
      `SELECT id, title, completed_at FROM v_task WHERE status = 'done' AND completed_at IS NOT NULL
         AND (?1 IS NULL OR thread_id = ?1) ORDER BY completed_at DESC LIMIT ?2`,
      [thread, limit],
      limit,
    ),
    thread === null
      ? querySql(`SELECT slug, title, updated_at FROM v_knowledge_page ORDER BY updated_at DESC LIMIT ?1`, [limit], limit)
      : querySql(
          `SELECT p.slug, p.title, k.last_seen_at FROM v_knowledge_touch k JOIN v_knowledge_page p ON p.ref = k.page
             WHERE k.thread_id = ?1 ORDER BY k.last_seen_at DESC LIMIT ?2`,
          [thread, limit],
          limit,
        ),
  ]);
  const entries: FinishedEntry[] = [
    ...tasks.rows.map(([id, title, t]) => ({ kind: "task" as const, itemId: taskIdOf(Number(id)), title: String(title), t: String(t) })),
    ...pages.rows.map(([slug, title, t]) => ({ kind: "wiki" as const, slug: String(slug), title: String(title), t: String(t) })),
  ];
  return {
    entries: recentlyFinished(entries, finishedClearedAt(threadId), limit),
    reads: { models: [...new Set([...tasks.reads.models, ...pages.reads.models])], tables: [], measures: [] },
  };
}

const CLEARED_KEY = "oxplow.finished.clearedAt";

/** When the person last cleared the Finished section (per thread; `""`
 *  for the project view). A viewer's own gesture, kept in this browser. */
export function finishedClearedAt(threadId: string | null): string | null {
  try {
    const all = JSON.parse(window.localStorage.getItem(CLEARED_KEY) ?? "{}") as Record<string, string>;
    return all[threadId ?? ""] ?? null;
  } catch {
    return null;
  }
}

export function clearRecentlyFinished(threadId: string | null, now = new Date().toISOString()): void {
  try {
    const all = JSON.parse(window.localStorage.getItem(CLEARED_KEY) ?? "{}") as Record<string, string>;
    all[threadId ?? ""] = now;
    window.localStorage.setItem(CLEARED_KEY, JSON.stringify(all));
  } catch {
    // Storage off: the section clears for this view only.
  }
}
