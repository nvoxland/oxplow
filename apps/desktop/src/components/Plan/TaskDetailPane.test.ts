import { expect, test } from "bun:test";
import type { WorkItem, WorkList } from "../../workItems.js";
import { summarizeThreadWork } from "./TaskDetailPane.js";

function task(ref: string, state: WorkItem["state"], updatedAt: string): WorkItem {
  return {
    ref,
    title: `item ${ref}`,
    state,
    updatedAt,
    closedAt: state === "done" || state === "canceled" ? updatedAt : null,
  } as unknown as WorkItem;
}

function workState(partial: Partial<WorkList>): WorkList {
  return {
    threadId: "th1",
    all: [],
    waiting: [],
    inProgress: [],
    done: [],
    epics: [],
    items: [],
    followups: [],
    ...partial,
  } as unknown as WorkList;
}

test("summary counts come from the bucketed arrays, not items", () => {
  // The backend puts only Ready tasks in `items`; in_progress /
  // blocked / done arrive solely via their buckets.
  const work = workState({
    items: [task("r1", "todo", "2026-06-01T00:00:00Z")],
    inProgress: [task("p1", "in_progress", "2026-06-01T00:00:00Z")],
    waiting: [
      task("b1", "blocked", "2026-06-01T00:00:00Z"),
      task("b2", "blocked", "2026-05-01T00:00:00Z"),
    ],
    done: [
      task("d1", "done", "2026-06-02T00:00:00Z"),
      task("d2", "done", "2026-06-03T00:00:00Z"),
    ],
  });
  const s = summarizeThreadWork(work);
  expect(s.counts).toEqual({ inProgress: 1, ready: 1, blocked: 2, done: 2 });
  expect(s.oldestBlocked?.ref).toBe("b2");
  expect(s.recentDone.map((t) => t.ref)).toEqual(["d2", "d1"]);
});

test("null thread work summarizes to zeros", () => {
  const s = summarizeThreadWork(null);
  expect(s.counts).toEqual({ inProgress: 0, ready: 0, blocked: 0, done: 0 });
  expect(s.oldestBlocked).toBeNull();
  expect(s.recentDone).toEqual([]);
});

test("the done bucket holds canceled items too; only done counts", () => {
  const work = workState({
    done: [task("d1", "done", "2026-06-02T00:00:00Z"), task("c1", "canceled", "2026-06-02T00:00:00Z")],
  });
  const s = summarizeThreadWork(work);
  expect(s.counts.done).toBe(1);
  expect(s.recentDone.map((t) => t.ref)).toEqual(["d1"]);
});
