import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { boardColumns, itemsFromResult, workItemsQuery, type WorkItem } from "./workItems.js";

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
