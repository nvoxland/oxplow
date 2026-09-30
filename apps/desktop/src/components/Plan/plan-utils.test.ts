import { expect, test } from "bun:test";
import type { BacklogState, Task, TaskStatus } from "../../api.js";
import {
  applyStatusFilter,
  buildBacklogGroups,
  buildGroups,
  openBacklogCount,
  classifyEpic,
  classifyRow,
  classifyTaskStatus,
  filterAutoAuthored,
  finalizeReorderIds,
  sectionDefaultStatus,
  splitIntoSections,
} from "./plan-utils.js";

function item(id: number, status: TaskStatus, sort_index: number): Task {
  return {
    id,
    thread_id: "b1",
    parent_id: null,
    title: id,
    description: "",
    status,
    priority: "medium",
    sort_index,
    author: "user",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
    note_count: 0,
  };
}

test("classifyTaskStatus buckets each status into exactly one section", () => {
  expect(classifyTaskStatus("in_progress")).toBe("inProgress");
  expect(classifyTaskStatus("ready")).toBe("ready");
  expect(classifyTaskStatus("blocked")).toBe("blocked");
  expect(classifyTaskStatus("done")).toBe("done");
  expect(classifyTaskStatus("canceled")).toBe("done");
  expect(classifyTaskStatus("archived")).toBe("done");
});

test("splitIntoSections returns sections in fixed order: inProgress → ready → blocked → done", () => {
  const sections = splitIntoSections([
    item("d1", "done", 3),
    item("b1", "blocked", 2),
    item("p1", "in_progress", 0),
    item("w1", "ready", 1),
  ]);
  expect(sections.map((section) => section.kind)).toEqual([
    "inProgress",
    "ready",
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
  expect(sections[0]?.kind).toBe("ready");
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
  expect(sectionDefaultStatus("ready")).toBe("ready");
  expect(sectionDefaultStatus("blocked")).toBe("blocked");
  expect(sectionDefaultStatus("done")).toBe("done");
  // The agent owns in_progress and its items are drag-locked — reject drops.
  expect(sectionDefaultStatus("inProgress")).toBeNull();
});

test("finalizeReorderIds is a no-op when there are no descending rows", () => {
  const visualRows = [
    { id: 501, status: "ready" as const },
    { id: 502, status: "ready" as const },
  ];
  expect(finalizeReorderIds(visualRows)).toEqual([501, 502]);
});

test("finalizeReorderIds reverses the Done/canceled/archived run too — matches TaskGroupList's descending Done render", () => {
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
  // buildBacklogGroups yields at least one group for TaskGroupList to render.
  const state: BacklogState = { items: [], waiting: [], in_progress: [], done: [] };
  const groups = buildBacklogGroups(state);
  expect(groups).toHaveLength(1);
  expect(groups[0]?.items).toEqual([]);
  expect(groups[0]?.epic).toBeNull();
});

test("buildBacklogGroups includes ready items so backlog tasks render, not just count in the chip", () => {
  // Regression: buildBacklogGroups used to spread only waiting/in_progress/done
  // and drop state.items (the `ready` bucket), so ready backlog tasks rendered
  // in no section — they only bumped the toggle-chip count. The To-Do section
  // must list them.
  const state: BacklogState = {
    items: [item("r1", "ready", 0), item("r2", "ready", 1)],
    waiting: [item("b1", "blocked", 2)],
    in_progress: [item("p1", "in_progress", 3)],
    done: [item("d1", "done", 4)],
  };
  const groups = buildBacklogGroups(state);
  expect(groups).toHaveLength(1);
  const ids = groups[0]?.items.map((i) => i.id) ?? [];
  expect(ids).toContain("r1");
  expect(ids).toContain("r2");
});

test("openBacklogCount counts ready + blocked + in_progress, excludes done", () => {
  // The BacklogDrawer badge must reflect outstanding backlog work. Counting
  // only `waiting` (the old bug) read "(0)" / "empty" for a backlog full of
  // ready tasks. Done items are finished, so they don't count toward the badge.
  const state: BacklogState = {
    items: [item("r1", "ready", 0), item("r2", "ready", 1)],
    waiting: [item("b1", "blocked", 2)],
    in_progress: [item("p1", "in_progress", 3)],
    done: [item("d1", "done", 4), item("d2", "done", 5)],
  };
  expect(openBacklogCount(state)).toBe(4);
});

test("openBacklogCount is 0 when state is null", () => {
  expect(openBacklogCount(null)).toBe(0);
});

test("buildBacklogGroups still returns an empty group when state is null", () => {
  // PlanPane passes backlog=null before the first fetch resolves. We still
  // want the pane to render the empty-state section chrome rather than a
  // blank view — the user can click "⋯ New task" immediately.
  const groups = buildBacklogGroups(null);
  expect(groups).toHaveLength(1);
  expect(groups[0]?.items).toEqual([]);
});

function epicItem(id: number, sort_index: number, status: TaskStatus = "ready"): Task {
  return { ...item(id, status, sort_index) };
}

test("classifyEpic: any blocked child → blocked", () => {
  const epic = epicItem(1, 0);
  expect(
    classifyEpic(epic, [
      item(101, "in_progress", 1),
      item(102, "blocked", 2),
      item(103, "done", 3),
    ]),
  ).toBe("blocked");
});

test("classifyEpic: all children terminal → done", () => {
  const epic = epicItem(1, 0);
  expect(classifyEpic(epic, [
    item(101, "done", 1),
    item(102, "canceled", 2),
    item(103, "archived", 3),
  ])).toBe("done");
});

test("classifyEpic: in_progress child → inProgress", () => {
  const epic = epicItem(1, 0);
  expect(classifyEpic(epic, [
    item(101, "ready", 1),
    item(102, "in_progress", 2),
    item(103, "ready", 3),
  ])).toBe("inProgress");
});

test("classifyEpic: mixed done + non-blocked unfinished → inProgress", () => {
  const epic = epicItem(1, 0);
  // Phase 1 done, Phase 2 ready: epic stays in_progress, not done.
  expect(classifyEpic(epic, [
    item(101, "done", 1),
    item(102, "ready", 2),
  ])).toBe("inProgress");
});

test("classifyEpic: all children ready → ready", () => {
  const epic = epicItem(1, 0);
  expect(classifyEpic(epic, [
    item(101, "ready", 1),
    item(102, "ready", 2),
  ])).toBe("ready");
});

test("classifyEpic: empty epic falls back to its literal status", () => {
  expect(classifyEpic(epicItem(1, 0, "ready"), [])).toBe("ready");
  expect(classifyEpic(epicItem(1, 0, "in_progress"), [])).toBe("inProgress");
});

test("classifyRow uses epic rollup for epics, literal status for non-epics", () => {
  const epic = epicItem(1, 0);
  const child = item(101, "in_progress", 1);
  const map = new Map<number, Task[]>([[epic.id, [item(102, "blocked", 2)]]]);
  expect(classifyRow(epic, map)).toBe("blocked");
  expect(classifyRow(child, map)).toBe("inProgress");
});

test("buildGroups groups epic children under their parent without lifting in_progress to root", () => {
  // Epics now move between sections as a block — children no longer
  // surface separately at the top level.
  const epic = epicItem(1, 0);
  const c1 = { ...item(101, "in_progress", 1), parent_id: epic.id };
  const c2 = { ...item(102, "ready", 2), parent_id: epic.id };
  const groups = buildGroups({
    epics: [epic],
    waiting: [c2],
    inProgress: [c1],
    done: [],
  } as any);
  expect(groups).toHaveLength(1);
  // Top-level rows: only the epic. No children lifted.
  expect(groups[0]!.items.map((i) => i.id)).toEqual([1]);
  // Children stay in the epic's children map for the renderer.
  expect(groups[0]!.epicChildren.get(1)!.map((i) => i.id)).toEqual([101, 102]);
});

test("filterAutoAuthored drops agent-authored rows but keeps user-authored ones", () => {
  const groups = [{
    epic: null,
    items: [
      { ...item(201, "ready", 0), author: "user" },
      { ...item(301, "ready", 1), author: "agent" },
      { ...item(202, "in_progress", 2), author: "user" },
    ] as Task[],
    epicChildren: new Map<number, Task[]>(),
  }];
  const filtered = filterAutoAuthored(groups);
  expect(filtered[0]!.items.map((i) => i.id)).toEqual([201, 202]);
});

test("filterAutoAuthored keeps epic rows even if agent-authored, and filters their children", () => {
  const epic = { ...item(1, "ready", 0), author: "agent" };
  const groups = [{
    epic: null,
    items: [epic] as Task[],
    epicChildren: new Map<number, Task[]>([[
      1,
      [
        { ...item(401, "ready", 1), author: "user" },
        { ...item(402, "ready", 2), author: "agent" },
      ] as Task[],
    ]]),
  }];
  const filtered = filterAutoAuthored(groups);
  expect(filtered[0]!.items.map((i) => i.id)).toEqual([1]);
  expect(filtered[0]!.epicChildren.get(1)!.map((i) => i.id)).toEqual([401]);
});

test("applyStatusFilter exclude drops matching items", () => {
  const groups = [{
    epic: null,
    items: [
      item(501, "ready", 0),
      item(502, "archived", 1),
      item(503, "done", 2),
    ] as Task[],
    epicChildren: new Map<number, Task[]>(),
  }];
  const filtered = applyStatusFilter(groups, { exclude: ["archived"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual([501, 503]);
});

test("applyStatusFilter only keeps matching items", () => {
  const groups = [{
    epic: null,
    items: [
      item(501, "ready", 0),
      item(502, "archived", 1),
      item(503, "done", 2),
    ] as Task[],
    epicChildren: new Map<number, Task[]>(),
  }];
  const filtered = applyStatusFilter(groups, { only: ["archived"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual([502]);
});

test("applyStatusFilter keeps epic rows even when status would exclude them, and filters their children", () => {
  const epic = { ...item(1, "ready", 0) };
  const groups = [{
    epic: null,
    items: [epic] as Task[],
    epicChildren: new Map<number, Task[]>([[
      1,
      [item(101, "ready", 1), item(102, "archived", 2)] as Task[],
    ]]),
  }];
  const filtered = applyStatusFilter(groups, { only: ["ready"] });
  expect(filtered[0]!.items.map((i) => i.id)).toEqual([1]);
  expect(filtered[0]!.epicChildren.get(1)!.map((i) => i.id)).toEqual([101]);
});
