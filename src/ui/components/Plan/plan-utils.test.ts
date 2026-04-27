import { expect, test } from "bun:test";
import type { BacklogState, WorkItem, WorkItemStatus } from "../../api.js";
import {
  applyStatusFilter,
  buildBacklogGroups,
  buildGroups,
  classifyEpic,
  classifyRow,
  classifyWorkItem,
  filterAutoAuthored,
  finalizeReorderIds,
  sectionDefaultStatus,
  splitIntoSections,
} from "./plan-utils.js";

function item(id: string, status: WorkItemStatus, sort_index: number): WorkItem {
  return {
    id,
    thread_id: "b1",
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
  expect(classifyWorkItem("done")).toBe("done");
  expect(classifyWorkItem("canceled")).toBe("done");
  expect(classifyWorkItem("archived")).toBe("done");
});

test("splitIntoSections returns sections in fixed order: inProgress → toDo → blocked → done", () => {
  const sections = splitIntoSections([
    item("d1", "done", 3),
    item("b1", "blocked", 2),
    item("p1", "in_progress", 0),
    item("w1", "ready", 1),
  ]);
  expect(sections.map((section) => section.kind)).toEqual([
    "inProgress",
    "toDo",
    "blocked",
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
  expect(sectionDefaultStatus("blocked")).toBe("blocked");
  expect(sectionDefaultStatus("done")).toBe("done");
  // The agent owns in_progress and its items are drag-locked — reject drops.
  expect(sectionDefaultStatus("inProgress")).toBeNull();
});

test("finalizeReorderIds is a no-op when there are no descending rows", () => {
  const visualRows = [
    { id: "a", status: "ready" as const },
    { id: "b", status: "ready" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual(["a", "b"]);
});

test("finalizeReorderIds reverses the Done/canceled/archived run too — matches WorkGroupList's descending Done render", () => {
  // Done renders descending visually (newest-done on top); sort_index stays
  // ascending. finalizeReorderIds must flip a multi-item Done run the same
  // way it flips humanCheck so reorderItems' "sort_index = position" rule
  // produces a visual order that matches what was just rendered.
  const visualRows = [
    { id: "t1", status: "ready" as const },
    { id: "d3", status: "done" as const },
    { id: "d2", status: "done" as const },
    { id: "d1", status: "done" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual(["t1", "d1", "d2", "d3"]);
});


test("buildBacklogGroups returns a single empty group for an empty backlog so section headers still render", () => {
  // The backlog pane should look like a regular Work pane — section headers
  // + the To-Do "⋯ New task" menu must be visible even when the backlog is
  // empty so the user can seed the first task. That only happens if
  // buildBacklogGroups yields at least one group for WorkGroupList to render.
  const state: BacklogState = { waiting: [], inProgress: [], done: [] } as any;
  const groups = buildBacklogGroups(state);
  expect(groups).toHaveLength(1);
  expect(groups[0]?.items).toEqual([]);
  expect(groups[0]?.epic).toBeNull();
});

test("buildBacklogGroups still returns an empty group when state is null", () => {
  // PlanPane passes backlog=null before the first fetch resolves. We still
  // want the pane to render the empty-state section chrome rather than a
  // blank view — the user can click "⋯ New task" immediately.
  const groups = buildBacklogGroups(null);
  expect(groups).toHaveLength(1);
  expect(groups[0]?.items).toEqual([]);
});

function epicItem(id: string, sort_index: number, status: WorkItemStatus = "ready"): WorkItem {
  return { ...item(id, status, sort_index), kind: "epic" };
}

test("classifyEpic: any blocked child → blocked", () => {
  const epic = epicItem("e1", 0);
  expect(
    classifyEpic(epic, [
      item("c1", "in_progress", 1),
      item("c2", "blocked", 2),
      item("c3", "done", 3),
    ]),
  ).toBe("blocked");
});

test("classifyEpic: all children terminal → done", () => {
  const epic = epicItem("e1", 0);
  expect(classifyEpic(epic, [
    item("c1", "done", 1),
    item("c2", "canceled", 2),
    item("c3", "archived", 3),
  ])).toBe("done");
});

test("classifyEpic: in_progress child → inProgress", () => {
  const epic = epicItem("e1", 0);
  expect(classifyEpic(epic, [
    item("c1", "ready", 1),
    item("c2", "in_progress", 2),
    item("c3", "ready", 3),
  ])).toBe("inProgress");
});

test("classifyEpic: mixed done + non-blocked unfinished → inProgress", () => {
  const epic = epicItem("e1", 0);
  // Phase 1 done, Phase 2 ready: epic stays in_progress, not done.
  expect(classifyEpic(epic, [
    item("c1", "done", 1),
    item("c2", "ready", 2),
  ])).toBe("inProgress");
});

test("classifyEpic: all children ready → toDo", () => {
  const epic = epicItem("e1", 0);
  expect(classifyEpic(epic, [
    item("c1", "ready", 1),
    item("c2", "ready", 2),
  ])).toBe("toDo");
});

test("classifyEpic: empty epic falls back to its literal status", () => {
  expect(classifyEpic(epicItem("e1", 0, "ready"), [])).toBe("toDo");
  expect(classifyEpic(epicItem("e1", 0, "in_progress"), [])).toBe("inProgress");
});

test("classifyRow uses epic rollup for epics, literal status for non-epics", () => {
  const epic = epicItem("e1", 0);
  const child = item("c1", "in_progress", 1);
  const map = new Map<string, WorkItem[]>([[epic.id, [item("c2", "blocked", 2)]]]);
  expect(classifyRow(epic, map)).toBe("blocked");
  expect(classifyRow(child, map)).toBe("inProgress");
});

test("buildGroups groups epic children under their parent without lifting in_progress to root", () => {
  // Epics now move between sections as a block — children no longer
  // surface separately at the top level.
  const epic = epicItem("e1", 0);
  const c1 = { ...item("c1", "in_progress", 1), parent_id: epic.id };
  const c2 = { ...item("c2", "ready", 2), parent_id: epic.id };
  const groups = buildGroups({
    epics: [epic],
    waiting: [c2],
    inProgress: [c1],
    done: [],
  } as any);
  expect(groups).toHaveLength(1);
  // Top-level rows: only the epic. No children lifted.
  expect(groups[0]!.items.map((i) => i.id)).toEqual(["e1"]);
  // Children stay in the epic's children map for the renderer.
  expect(groups[0]!.epicChildren.get("e1")!.map((i) => i.id)).toEqual(["c1", "c2"]);
});

test("filterAutoAuthored drops agent-authored rows but keeps user-authored ones", () => {
  const groups = [{
    epic: null,
    items: [
      { ...item("u1", "ready", 0), created_by: "user" },
      { ...item("a1", "ready", 1), created_by: "agent" },
      { ...item("u2", "in_progress", 2), created_by: "user" },
    ] as WorkItem[],
    epicChildren: new Map<string, WorkItem[]>(),
  }];
  const filtered = filterAutoAuthored(groups);
  expect(filtered[0]!.items.map((i) => i.id)).toEqual(["u1", "u2"]);
});

test("filterAutoAuthored keeps epic rows even if agent-authored, and filters their children", () => {
  const epic = { ...item("e1", "ready", 0), kind: "epic" as const, created_by: "agent" };
  const groups = [{
    epic: null,
    items: [epic] as WorkItem[],
    epicChildren: new Map<string, WorkItem[]>([[
      "e1",
      [
        { ...item("u-child", "ready", 1), created_by: "user" },
        { ...item("a-child", "ready", 2), created_by: "agent" },
      ] as WorkItem[],
    ]]),
  }];
  const filtered = filterAutoAuthored(groups);
  expect(filtered[0]!.items.map((i) => i.id)).toEqual(["e1"]);
  expect(filtered[0]!.epicChildren.get("e1")!.map((i) => i.id)).toEqual(["u-child"]);
});

test("applyStatusFilter exclude drops matching items", () => {
  const groups = [{
    epic: null,
    items: [
      item("a", "ready", 0),
      item("b", "archived", 1),
      item("c", "done", 2),
    ] as WorkItem[],
    epicChildren: new Map<string, WorkItem[]>(),
  }];
  const filtered = applyStatusFilter(groups, { exclude: ["archived"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual(["a", "c"]);
});

test("applyStatusFilter only keeps matching items", () => {
  const groups = [{
    epic: null,
    items: [
      item("a", "ready", 0),
      item("b", "archived", 1),
      item("c", "done", 2),
    ] as WorkItem[],
    epicChildren: new Map<string, WorkItem[]>(),
  }];
  const filtered = applyStatusFilter(groups, { only: ["archived"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual(["b"]);
});

test("applyStatusFilter keeps epic rows even when status would exclude them, and filters their children", () => {
  const epic = { ...item("e1", "ready", 0), kind: "epic" as const };
  const groups = [{
    epic: null,
    items: [epic] as WorkItem[],
    epicChildren: new Map<string, WorkItem[]>([[
      "e1",
      [item("c1", "ready", 1), item("c2", "archived", 2)] as WorkItem[],
    ]]),
  }];
  const filtered = applyStatusFilter(groups, { only: ["ready"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual(["e1"]);
  expect(filtered[0]!.epicChildren.get("e1")!.map((i) => i.id)).toEqual(["c1"]);
});
