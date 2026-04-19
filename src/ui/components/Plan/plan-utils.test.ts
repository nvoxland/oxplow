import { expect, test } from "bun:test";
import type { WorkItem, WorkItemStatus } from "../../api.js";
import { classifyWorkItem, splitIntoSections } from "./plan-utils.js";

function item(id: string, status: WorkItemStatus, sort_index: number): WorkItem {
  return {
    id,
    batch_id: "b1",
    parent_id: null,
    kind: "task",
    title: id,
    description: "",
    acceptance_criteria: null,
    status,
    priority: "medium",
    sort_index,
    created_by: "user",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  };
}

test("classifyWorkItem buckets each status into exactly one section", () => {
  expect(classifyWorkItem("in_progress")).toBe("inProgress");
  expect(classifyWorkItem("waiting")).toBe("toDo");
  expect(classifyWorkItem("ready")).toBe("toDo");
  expect(classifyWorkItem("blocked")).toBe("toDo");
  expect(classifyWorkItem("human_check")).toBe("humanCheck");
  expect(classifyWorkItem("done")).toBe("done");
  expect(classifyWorkItem("canceled")).toBe("done");
});

test("splitIntoSections returns sections in fixed order: inProgress → toDo → humanCheck → done", () => {
  const sections = splitIntoSections([
    item("d1", "done", 3),
    item("t1", "human_check", 2),
    item("p1", "in_progress", 0),
    item("w1", "waiting", 1),
  ]);
  expect(sections.map((section) => section.kind)).toEqual([
    "inProgress",
    "toDo",
    "humanCheck",
    "done",
  ]);
});

test("splitIntoSections skips empty sections entirely so no header renders for them", () => {
  const sections = splitIntoSections([
    item("w1", "waiting", 0),
    item("w2", "waiting", 1),
  ]);
  expect(sections).toHaveLength(1);
  expect(sections[0]?.kind).toBe("toDo");
});

test("splitIntoSections sorts items within a section by sort_index", () => {
  const sections = splitIntoSections([
    item("w3", "waiting", 20),
    item("w1", "waiting", 5),
    item("w2", "waiting", 10),
  ]);
  expect(sections[0]?.items.map((i) => i.id)).toEqual(["w1", "w2", "w3"]);
});

test("splitIntoSections keeps human_check out of the in-progress bucket", () => {
  // Regression: the old BatchWorkState pre-grouped in_progress + human_check
  // together; the work panel was reorganized to separate them.
  const sections = splitIntoSections([
    item("p1", "in_progress", 0),
    item("c1", "human_check", 1),
  ]);
  const inProgress = sections.find((section) => section.kind === "inProgress");
  const humanCheck = sections.find((section) => section.kind === "humanCheck");
  expect(inProgress?.items.map((i) => i.id)).toEqual(["p1"]);
  expect(humanCheck?.items.map((i) => i.id)).toEqual(["c1"]);
});
