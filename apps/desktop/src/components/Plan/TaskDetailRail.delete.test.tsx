import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Task } from "../../workItems.js";
import { TaskDetailRail } from "./TaskDetail.js";

afterEach(cleanup);

const task = {
  id: "tsk1",
  thread_id: "thr1",
  parent_id: null,
  title: "Fix it",
  description: "",
  status: "ready",
  priority: "medium",
  sort_index: 0,
  author: "user",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  completed_at: null,
  note_count: 0,
} as unknown as Task;

// Delete asks inline (usability.md: InlineConfirm, never window.confirm,
// which blocks the renderer): the first press arms it, Confirm runs it,
// Escape backs out.
test("the rail's Delete asks inline before it deletes", () => {
  const deleted: number[] = [];
  const view = render(<TaskDetailRail item={task} onUpdateTask={async () => {}} onDelete={() => deleted.push(1)} />);
  fireEvent.click(view.getByTestId("task-rail-delete-trigger"));
  expect(deleted).toEqual([]);
  fireEvent.keyDown(window, { key: "Escape" });
  expect(view.queryByTestId("task-rail-delete-confirm")).toBeNull();
  fireEvent.click(view.getByTestId("task-rail-delete-trigger"));
  fireEvent.click(view.getByTestId("task-rail-delete-confirm"));
  expect(deleted).toEqual([1]);
});
