import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { boardColumns, effortDetailsFromResult, itemsFromResult, orderedTaskIds, workItemsQuery, type WorkItem } from "./workItems.js";

// A task's activity is one read over the models: `v_effort` joined to
// `v_effort_file` (one row per effort and file; an effort with no files
// still appears), newest effort first.
test("effort rows join to their files: one detail per effort, counts by change kind", () => {
  const res = {
    columns: ["id", "work_item", "started_at", "ended_at", "start_snapshot_id", "end_snapshot_id", "summary", "path", "change_kind"],
    rows: [
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "src/a.rs", "updated"],
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "src/b.rs", "created"],
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "old.rs", "deleted"],
      [5, "work_item:oxplow:tsk4", "t0", "t1", 8, 9, "did it", null, null],
    ],
    truncated: false,
    reads: { models: ["v_effort", "v_effort_file"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const details = effortDetailsFromResult(res);
  expect(details.map((d) => d.effort.id)).toEqual(["eff7", "eff5"]);
  expect(details[0]).toEqual({
    effort: { id: "eff7", work_item: "work_item:oxplow:tsk4", started_at: "t2", ended_at: null, start_snapshot_id: "10", end_snapshot_id: null, summary: null },
    start_snapshot: null,
    end_snapshot: null,
    changed_paths: ["src/a.rs", "src/b.rs", "old.rs"],
    counts: { created: 1, updated: 1, deleted: 1 },
  });
  expect(details[1]?.changed_paths).toEqual([]);
  expect(details[1]?.effort.summary).toBe("did it");
});

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: [
      "ref", "provider", "title", "body", "state", "native_state", "parent_ref", "created_at", "updated_at",
      "task_id", "thread_id", "status", "priority", "sort_index", "author", "completed_at", "note_count",
    ],
    rows,
    truncated: false,
    reads: { models: ["v_work_item", "v_task"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

test("rows read as work items, with oxplow's own fields when the item is a task", () => {
  const items = itemsFromResult(
    result([
      ["work_item:oxplow:tsk4", "oxplow", "Fix it", "", "in_progress", "in_progress", null, "t0", "t1", 4, 2, "in_progress", "high", 0, "agent", null, 3],
      ["work_item:linear:ENG-1", "linear", "Theirs", "b", "todo", "Backlog", "work_item:oxplow:tsk4", "t0", "t1", null, null, null, null, null, null, null, 0],
    ]),
  );
  expect(items[0]).toEqual({
    ref: "work_item:oxplow:tsk4",
    provider: "oxplow",
    title: "Fix it",
    body: "",
    state: "in_progress",
    nativeState: "in_progress",
    parentRef: null,
    createdAt: "t0",
    updatedAt: "t1",
    task: { id: "tsk4", threadId: "thr2", status: "in_progress", priority: "high", sortIndex: 0, author: "agent", completedAt: null, noteCount: 3 },
  });
  expect(items[1]!.task).toBeNull();
  expect(items[1]!.parentRef).toBe("work_item:oxplow:tsk4");
});

test("a query scoped to a thread, the backlog, or everything; states filter", () => {
  expect(workItemsQuery({ scope: { thread: "thr2" } }).params).toEqual([2]);
  expect(workItemsQuery({ scope: { thread: "thr2" } }).sql).toContain("t.thread_id = ?1");
  expect(workItemsQuery({ scope: "backlog" }).sql).toContain("t.thread_id IS NULL");
  const all = workItemsQuery({ scope: "all", states: ["todo", "blocked"] });
  expect(all.sql).toContain("w.state IN ('todo', 'blocked')");
  expect(all.sql).toContain("ORDER BY");
  expect(workItemsQuery({ scope: "all", hideArchived: true }).sql).toContain("w.native_state IS NOT 'archived'");
});

test("the Board groups by canonical state, in workflow order, each column in list order", () => {
  const item = (ref: string, state: WorkItem["state"]): WorkItem =>
    ({ ref, state, title: ref, task: null }) as unknown as WorkItem;
  const columns = boardColumns([item("a", "done"), item("b", "todo"), item("c", "todo"), item("d", "blocked")]);
  expect(columns.map((c) => [c.state, c.items.map((i) => i.ref)])).toEqual([
    ["todo", ["b", "c"]],
    ["in_progress", []],
    ["blocked", ["d"]],
    ["done", ["a"]],
    ["canceled", []],
  ]);
});

import { bucketThreadWork, placementFromOrder, recentlyFinished, tasksFromResult } from "./workItems.js";

const taskResult = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: ["id", "thread_id", "parent_id", "title", "description", "status", "priority", "sort_index", "author", "created_at", "updated_at", "completed_at", "note_count"],
    rows,
    truncated: false,
    reads: { models: ["v_task"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

test("tasks read from v_task with the UI's ids", () => {
  expect(tasksFromResult(taskResult([[4, 2, 1, "Fix", "body", "ready", "high", 3, "agent", "t0", "t1", null, 2]]))).toEqual([
    {
      id: "tsk4",
      thread_id: "thr2",
      parent_id: "tsk1",
      title: "Fix",
      description: "body",
      status: "ready",
      priority: "high",
      sort_index: 3,
      author: "agent",
      created_at: "t0",
      updated_at: "t1",
      completed_at: null,
      note_count: 2,
    },
  ]);
});

test("a thread's work: a task with a child is an epic, the rest by status", () => {
  const tasks = tasksFromResult(
    taskResult([
      [1, 2, null, "Epic", "", "ready", "medium", 0, "user", "t", "t", null, 0],
      [2, 2, 1, "Child", "", "in_progress", "medium", 1, "user", "t", "t", null, 0],
      [3, 2, null, "Blocked", "", "blocked", "medium", 2, "user", "t", "t", null, 0],
      [4, 2, null, "Done", "", "archived", "medium", 3, "user", "t", "t", "t", 0],
      [5, 2, null, "Next", "", "ready", "medium", 4, "user", "t", "t", null, 0],
    ]),
  );
  const work = bucketThreadWork("thr2", tasks, []);
  expect(work.epics.map((t) => t.id)).toEqual(["tsk1"]);
  expect(work.inProgress.map((t) => t.id)).toEqual(["tsk2"]);
  expect(work.waiting.map((t) => t.id)).toEqual(["tsk3"]);
  expect(work.done.map((t) => t.id)).toEqual(["tsk4"]);
  expect(work.items.map((t) => t.id)).toEqual(["tsk5"]);
});

test("a reordered list is one item placed next to a neighbour", () => {
  expect(placementFromOrder(["a", "b", "c", "d"], ["a", "d", "b", "c"])).toEqual({ id: "d", place: { after: "a" } });
  expect(placementFromOrder(["a", "b", "c"], ["c", "a", "b"])).toEqual({ id: "c", place: { before: "a" } });
  expect(placementFromOrder(["a", "b", "c"], ["b", "c", "a"])).toEqual({ id: "a", place: { after: "c" } });
  expect(placementFromOrder(["a", "b"], ["a", "b"])).toBeNull();
});

test("recently finished: done tasks and touched pages, newest first, after the cleared cursor", () => {
  const out = recentlyFinished(
    [
      { kind: "task", itemId: "tsk1", title: "Old", t: "2026-01-01T00:00:00Z" },
      { kind: "task", itemId: "tsk2", title: "New", t: "2026-01-03T00:00:00Z" },
      { kind: "wiki", slug: "notes", title: "Notes", t: "2026-01-02T00:00:00Z" },
    ],
    "2026-01-01T12:00:00Z",
    5,
  );
  expect(out.map((e) => e.title)).toEqual(["New", "Notes"]);
});

// A drag's "before" order is the server's list order — sort_index, then
// created_at — whatever bucket each task sits in, so equal indices can't
// make a drag pick the wrong moved item.
test("orderedTaskIds follows the server's order, ties by creation time", () => {
  const t = (id: string, status: string, sort_index: number, created_at: string) =>
    ({ id, status, sort_index, created_at, parent_id: null }) as never;
  const work = {
    threadId: "thr1",
    epics: [],
    items: [t("tsk3", "ready", 0, "2026-01-03")],
    waiting: [],
    inProgress: [t("tsk1", "in_progress", 0, "2026-01-01")],
    done: [t("tsk2", "done", 0, "2026-01-02")],
    followups: [],
    reads: { models: [], tables: [], measures: [] },
  };
  expect(orderedTaskIds(work)).toEqual(["tsk1", "tsk2", "tsk3"]);
});

import { capabilityProvidersFromResult, featuresFor, providerOf, workItemCommand } from "./workItems.js";

// P6b.C2: a provider's flags come from v_capability_provider; a provider
// the model doesn't list (or a flag it doesn't declare) is off.
test("capability providers read with their features; an unknown provider has none", () => {
  const providers = capabilityProvidersFromResult({
    columns: ["capability", "provider", "extension", "features", "active"],
    rows: [
      ["work_items", "oxplow", null, '{"hierarchy":true,"comments":true,"links":true,"in_progress_opens_effort":true}', 1],
      ["work_items", "fake", "tracker", '{"comments":true}', 1],
    ],
    truncated: false,
    reads: { models: ["v_capability_provider"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult);
  expect(featuresFor(providers, "oxplow")).toEqual({ hierarchy: true, comments: true, links: true, in_progress_opens_effort: true });
  expect(featuresFor(providers, "fake")).toEqual({ hierarchy: false, comments: true, links: false, in_progress_opens_effort: false });
  expect(featuresFor(providers, "linear")).toEqual({ hierarchy: false, comments: false, links: false, in_progress_opens_effort: false });
  expect(providers.find((p) => p.provider === "fake")?.extension).toBe("tracker");
});

// R22: a provider id is lowercase snake_case, as the Rust grammar has it;
// a string that isn't a work item ref names no command.
test("a work item ref's provider follows the id grammar; a non-ref has no command", () => {
  expect(providerOf("work_item:fake_tracker:W-1")).toBe("fake_tracker");
  expect(providerOf("work_item:fake-tracker:W-1")).toBeNull();
  expect(providerOf("task:tsk1")).toBeNull();
  expect(workItemCommand("work_item:oxplow:tsk1", "transition")).toBe("work_item.transition");
  expect(workItemCommand("work_item:fake:W-1", "comment")).toBe("fake.comment");
  expect(() => workItemCommand("tsk1", "transition")).toThrow("isn't a work item ref");
});
