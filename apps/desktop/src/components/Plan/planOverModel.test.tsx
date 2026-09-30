import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";
import { bucketThreadWork, tasksFromResult } from "../../workItems.js";
import { TaskGroupList } from "./TaskGroupList.js";
import { buildGroups } from "./plan-utils.js";

afterEach(cleanup);

// P6.E1b: the Tasks list renders from the models — `v_task` rows read by
// the data layer, bucketed into the thread's work, grouped by the plan's
// own grouping — with no typed task RPC in between.
test("the task list renders a thread's work read from v_task", () => {
  const result = {
    columns: ["id", "thread_id", "parent_id", "title", "description", "status", "priority", "sort_index", "author", "created_at", "updated_at", "completed_at", "note_count"],
    rows: [
      [1, 1, null, "Epic", "", "ready", "medium", 0, "user", "t", "t", null, 0],
      [2, 1, 1, "Child step", "", "in_progress", "high", 1, "agent", "t", "t", null, 1],
      [3, 1, null, "Loose task", "", "ready", "low", 2, "user", "t", "t", null, 0],
    ],
    truncated: false,
    reads: { models: ["v_task"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const work = bucketThreadWork("thr1", tasksFromResult(result), []);
  const groups = buildGroups(work);
  const NOOP = async () => {};
  const view = render(
    <>
      {groups.map((group, i) => (
        <TaskGroupList
          key={i}
          group={group}
          scopeThreadId="thr1"
          onUpdateTask={NOOP}
          onReorderTasks={NOOP}
          onOpenMenu={() => {}}
          epicChildrenMap={new Map(groups.flatMap((g) => [...g.epicChildren.entries()]))}
          onReparentTask={NOOP}
          isSectionCollapsed={() => false}
          onToggleSectionCollapsed={() => {}}
          visibleSections={["inProgress", "ready", "blocked", "done"]}
        />
      ))}
    </>,
  );
  expect(view.container.textContent).toContain("Epic");
  expect(view.container.textContent).toContain("Loose task");
  expect(view.getByTestId("tasks-row-tsk3")).not.toBeNull();
});
