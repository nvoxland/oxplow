import { expect, test } from "bun:test";
import type { WorkItem, WorkItemStatus } from "../../api.js";
import {
  classifyWorkItem,
  finalizeReorderIds,
  sectionDefaultStatus,
  splitIntoSections,
} from "./plan-utils.js";

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
    note_count: 0,
  };
}

test("classifyWorkItem buckets each status into exactly one section", () => {
  expect(classifyWorkItem("in_progress")).toBe("inProgress");
  expect(classifyWorkItem("ready")).toBe("toDo");
  expect(classifyWorkItem("blocked")).toBe("blocked");
  expect(classifyWorkItem("human_check")).toBe("humanCheck");
  expect(classifyWorkItem("done")).toBe("done");
  expect(classifyWorkItem("canceled")).toBe("done");
  expect(classifyWorkItem("archived")).toBe("done");
});

test("splitIntoSections returns sections in fixed order: inProgress → toDo → humanCheck → done", () => {
  const sections = splitIntoSections([
    item("d1", "done", 3),
    item("t1", "human_check", 2),
    item("p1", "in_progress", 0),
    item("w1", "ready", 1),
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
    item("w1", "ready", 0),
    item("w2", "ready", 1),
  ]);
  expect(sections).toHaveLength(1);
  expect(sections[0]?.kind).toBe("toDo");
});

test("splitIntoSections sorts items within a section by sort_index", () => {
  const sections = splitIntoSections([
    item("w3", "ready", 20),
    item("w1", "ready", 5),
    item("w2", "ready", 10),
  ]);
  expect(sections[0]?.items.map((i) => i.id)).toEqual(["w1", "w2", "w3"]);
});

test("sectionDefaultStatus maps drop-target sections to landing statuses; in-progress is blocked", () => {
  expect(sectionDefaultStatus("toDo")).toBe("ready");
  expect(sectionDefaultStatus("humanCheck")).toBe("human_check");
  expect(sectionDefaultStatus("done")).toBe("done");
  // The agent owns in_progress and its items are drag-locked — reject drops.
  expect(sectionDefaultStatus("inProgress")).toBeNull();
});

test("splitIntoSections sorts humanCheck descending so newest (highest sort_index) appears first", () => {
  const sections = splitIntoSections([
    item("c1", "human_check", 5),
    item("c2", "human_check", 20),
    item("c3", "human_check", 10),
  ]);
  const humanCheck = sections.find((s) => s.kind === "humanCheck");
  expect(humanCheck?.items.map((i) => i.id)).toEqual(["c2", "c3", "c1"]);
});

test("finalizeReorderIds reverses the humanCheck subsequence so descending visual order persists correctly", () => {
  // Visual order (what the user sees top-to-bottom):
  //   to-do: t1, t2
  //   human-check (displayed descending): h3, h2, h1
  //   done: d1
  // The persisted order must be ascending-sort_index for every section, which
  // means the humanCheck run in the id list needs to be reversed before it
  // hits the store (which rewrites sort_index = position).
  const visualRows = [
    { id: "t1", status: "ready" as const },
    { id: "t2", status: "ready" as const },
    { id: "h3", status: "human_check" as const },
    { id: "h2", status: "human_check" as const },
    { id: "h1", status: "human_check" as const },
    { id: "d1", status: "done" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual(["t1", "t2", "h1", "h2", "h3", "d1"]);
});

test("finalizeReorderIds is a no-op when there are no humanCheck rows", () => {
  const visualRows = [
    { id: "a", status: "ready" as const },
    { id: "b", status: "done" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual(["a", "b"]);
});

test("finalizeReorderIds handles a single humanCheck item (trivial reverse)", () => {
  const visualRows = [
    { id: "a", status: "ready" as const },
    { id: "h", status: "human_check" as const },
    { id: "b", status: "done" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual(["a", "h", "b"]);
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
