import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { Task } from "../../api.js";
import { TaskGroupList } from "./TaskGroupList.js";
import type { TaskGroup } from "./plan-utils.js";

afterEach(cleanup);

const NOOP_ASYNC = async () => {};

// tsk144: an in-progress task with a very long title (the old layout
// rendered an un-truncated preview that ran off the column and overlapped
// the right-hand Summary panel). The row must shrink (minWidth:0) and the
// title must carry single-line ellipsis truncation. happy-dom doesn't do
// layout, so we assert the truncation contract structurally rather than by
// pixel width.
const LONG = "In progress task with an extremely long title ".repeat(12);

function inProgressTask(): Task {
  return {
    id: "tsk-long",
    title: LONG,
    status: "in_progress",
    priority: "medium",
    sort_index: 0,
  } as unknown as Task;
}

function renderRow() {
  const group: TaskGroup = {
    epic: null,
    items: [inProgressTask()],
    epicChildren: new Map(),
  };
  return render(
    <TaskGroupList
      group={group}
      scopeThreadId="thr1"
      onUpdateTask={NOOP_ASYNC}
      onReorderTasks={NOOP_ASYNC}
      onOpenMenu={() => {}}
      epicChildrenMap={new Map()}
      onReparentTask={NOOP_ASYNC}
      isSectionCollapsed={() => false}
      onToggleSectionCollapsed={() => {}}
      visibleSections={["inProgress"]}
    />,
  );
}

test("in-progress row shrinks and truncates its title (never overflows the column)", () => {
  const view = renderRow();
  const row = view.getByTestId("tasks-row-tsk-long");
  // The row must be allowed to shrink below its content width.
  expect(["0", "0px"]).toContain(row.style.minWidth);

  // The title text lives in a single-line ellipsis span.
  const titleSpan = Array.from(row.querySelectorAll("span")).find(
    (el) => el.textContent === LONG,
  );
  expect(titleSpan).toBeTruthy();
  expect(titleSpan!.style.overflow).toBe("hidden");
  expect(titleSpan!.style.textOverflow).toBe("ellipsis");
  expect(titleSpan!.style.whiteSpace).toBe("nowrap");
  expect(["0", "0px"]).toContain(titleSpan!.style.minWidth);
});
