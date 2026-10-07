import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { FieldDecl, WorkItem } from "../../workItems.js";
import { TaskDetailRail, type TaskDetailChanges } from "./TaskDetail.js";

afterEach(cleanup);

const item: WorkItem = {
  ref: "work_item:oxplow:tsk1",
  provider: "oxplow",
  title: "Fix it",
  body: "",
  state: "todo",
  parentRef: null,
  threadId: "thr1",
  rank: 0,
  closedAt: null,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
  native: { priority: "medium", author: "agent" },
  commentCount: 0,
};

const fields: FieldDecl[] = [
  { name: "priority", title: "Priority", kind: "enum", values: ["high", "medium", "low"], read_only: false },
  { name: "author", title: "Filed by", kind: "enum", values: ["user", "agent"], read_only: true },
];

// Delete asks inline (usability.md: InlineConfirm, never window.confirm,
// which blocks the renderer): the first press arms it, Confirm runs it,
// Escape backs out.
test("the rail's Delete asks inline before it deletes", () => {
  const deleted: number[] = [];
  const view = render(<TaskDetailRail item={item} fields={[]} onUpdateTask={async () => {}} onDelete={() => deleted.push(1)} />);
  fireEvent.click(view.getByTestId("task-rail-delete-trigger"));
  expect(deleted).toEqual([]);
  fireEvent.keyDown(window, { key: "Escape" });
  expect(view.queryByTestId("task-rail-delete-confirm")).toBeNull();
  fireEvent.click(view.getByTestId("task-rail-delete-trigger"));
  fireEvent.click(view.getByTestId("task-rail-delete-confirm"));
  expect(deleted).toEqual([1]);
});

// The review was reached only from the activity card, down the page. The
// rail offers it too, once the item has an effort.
test("the rail offers the item's review once it has one, and not before", () => {
  const reviewed: number[] = [];
  const view = render(<TaskDetailRail item={item} fields={[]} onUpdateTask={async () => {}} onReview={() => reviewed.push(1)} />);
  fireEvent.click(view.getByTestId("task-rail-review"));
  expect(reviewed).toEqual([1]);
  cleanup();
  expect(render(<TaskDetailRail item={item} fields={[]} onUpdateTask={async () => {}} />).queryByTestId("task-rail-review")).toBeNull();
});

// The list's own fields show as it declares them: an editable one changes
// through `native`, a read-only one is only shown; the state moves through
// a transition.
test("the rail shows the list's own fields and changes the editable ones", () => {
  const changes: Array<[string, TaskDetailChanges]> = [];
  const view = render(
    <TaskDetailRail item={item} fields={fields} onUpdateTask={async (ref, c) => void changes.push([ref, c])} />,
  );
  fireEvent.change(view.getByLabelText("Priority"), { target: { value: "high" } });
  expect(view.queryByLabelText("Filed by")).toBeNull();
  expect(view.container.textContent).toContain("agent");
  fireEvent.change(view.getByLabelText("State"), { target: { value: "blocked" } });
  expect(changes).toEqual([
    ["work_item:oxplow:tsk1", { native: { priority: "high" } }],
    ["work_item:oxplow:tsk1", { state: "blocked" }],
  ]);
});
