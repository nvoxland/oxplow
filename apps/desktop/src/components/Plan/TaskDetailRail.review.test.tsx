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
  status: "done",
  priority: "medium",
  sort_index: 0,
  author: "user",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  completed_at: null,
  note_count: 0,
} as unknown as Task;

// tsk1036: the task's review was reached only from its activity card, down
// the page. The rail offers it too, once the task has an effort.
test("the rail offers the task's review once it has one", () => {
  const reviewed: number[] = [];
  const view = render(<TaskDetailRail item={task} onUpdateTask={async () => {}} onReview={() => reviewed.push(1)} />);
  fireEvent.click(view.getByTestId("task-rail-review"));
  expect(reviewed).toEqual([1]);
});

test("a task with no effort has no review to offer", () => {
  const view = render(<TaskDetailRail item={task} onUpdateTask={async () => {}} />);
  expect(view.queryByTestId("task-rail-review")).toBeNull();
});
