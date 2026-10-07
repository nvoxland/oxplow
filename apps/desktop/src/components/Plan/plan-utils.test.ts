import { expect, test } from "bun:test";
import { bucketWorkList, emptyWorkList, type CanonicalState, type WorkItem } from "../../workItems.js";
import {
  applyStateFilter,
  buildBacklogGroups,
  buildGroups,
  classifyEpic,
  classifyRow,
  classifyState,
  filterByFields,
  finalizeReorderRefs,
  openBacklogCount,
  sectionDefaultState,
  splitIntoSections,
} from "./plan-utils.js";

const NO_READS = { models: [], tables: [], measures: [] };

function item(ref: string, state: CanonicalState, extra: Partial<WorkItem> = {}): WorkItem {
  return {
    ref,
    provider: "oxplow",
    title: ref,
    body: "",
    state,
    parentRef: null,
    threadId: "thr1",
    rank: null,
    closedAt: null,
    createdAt: "2024-01-01T00:00:00Z",
    updatedAt: "2024-01-01T00:00:00Z",
    native: {},
    commentCount: 0,
    ...extra,
  };
}

const list = (items: WorkItem[]) => bucketWorkList("thr1", items, [], NO_READS);

test("classifyState buckets each state into exactly one section", () => {
  expect(classifyState("in_progress")).toBe("inProgress");
  expect(classifyState("todo")).toBe("ready");
  expect(classifyState("blocked")).toBe("blocked");
  expect(classifyState("done")).toBe("done");
  expect(classifyState("canceled")).toBe("done");
});

test("splitIntoSections returns sections in fixed order, each in the order given", () => {
  const sections = splitIntoSections([
    item("d1", "done"),
    item("b1", "blocked"),
    item("w2", "todo"),
    item("p1", "in_progress"),
    item("w1", "todo"),
  ]);
  expect(sections.map((s) => s.kind)).toEqual(["inProgress", "ready", "blocked", "done"]);
  expect(sections[1]?.items.map((i) => i.ref)).toEqual(["w2", "w1"]);
});

test("splitIntoSections skips empty sections entirely so no header renders for them", () => {
  const sections = splitIntoSections([item("w1", "todo"), item("w2", "todo")]);
  expect(sections).toHaveLength(1);
  expect(sections[0]?.kind).toBe("ready");
});

test("sectionDefaultState maps drop-target sections to landing states; in-progress refuses", () => {
  expect(sectionDefaultState("ready")).toBe("todo");
  expect(sectionDefaultState("blocked")).toBe("blocked");
  expect(sectionDefaultState("done")).toBe("done");
  // The agent owns in_progress and its items are drag-locked — reject drops.
  expect(sectionDefaultState("inProgress")).toBeNull();
});

test("finalizeReorderRefs is a no-op when there are no descending rows", () => {
  expect(finalizeReorderRefs([{ ref: "a", state: "todo" }, { ref: "b", state: "todo" }])).toEqual(["a", "b"]);
});

test("finalizeReorderRefs reverses the done/canceled run — Done renders newest first", () => {
  const visual = [
    { ref: "t1", state: "todo" as const },
    { ref: "d3", state: "done" as const },
    { ref: "d2", state: "canceled" as const },
    { ref: "d1", state: "done" as const },
  ];
  expect(finalizeReorderRefs(visual)).toEqual(["t1", "d1", "d2", "d3"]);
});

// The backlog renders like any list: section headers and "New item" even
// when it's empty or still loading.
test("buildBacklogGroups returns a single group, empty or not", () => {
  expect(buildBacklogGroups(null)).toEqual([{ epic: null, items: [], epicChildren: new Map() }]);
  expect(buildBacklogGroups(emptyWorkList(null))[0]?.items).toEqual([]);
  const backlog = bucketWorkList(null, [item("r1", "todo"), item("b1", "blocked"), item("r2", "todo")], [], NO_READS);
  expect(buildBacklogGroups(backlog)[0]?.items.map((i) => i.ref)).toEqual(["r1", "b1", "r2"]);
});

test("openBacklogCount counts ready + blocked + in progress, not done", () => {
  const backlog = bucketWorkList(
    null,
    [item("r1", "todo"), item("r2", "todo"), item("b1", "blocked"), item("p1", "in_progress"), item("d1", "done")],
    [],
    NO_READS,
  );
  expect(openBacklogCount(backlog)).toBe(4);
  expect(openBacklogCount(null)).toBe(0);
});

test("classifyEpic rolls its children up: blocked, then all closed, then started, then ready", () => {
  const epic = item("e", "todo");
  expect(classifyEpic(epic, [item("a", "in_progress"), item("b", "blocked")])).toBe("blocked");
  expect(classifyEpic(epic, [item("a", "done"), item("b", "canceled")])).toBe("done");
  expect(classifyEpic(epic, [item("a", "todo"), item("b", "in_progress")])).toBe("inProgress");
  expect(classifyEpic(epic, [item("a", "done"), item("b", "todo")])).toBe("inProgress");
  expect(classifyEpic(epic, [item("a", "todo"), item("b", "todo")])).toBe("ready");
  // An empty epic is its own state.
  expect(classifyEpic(item("e", "in_progress"), [])).toBe("inProgress");
});

test("classifyRow uses the epic rollup for epics, the state for the rest", () => {
  const map = new Map<string, WorkItem[]>([["e", [item("c", "blocked")]]]);
  expect(classifyRow(item("e", "todo"), map)).toBe("blocked");
  expect(classifyRow(item("x", "in_progress"), map)).toBe("inProgress");
});

test("buildGroups keeps an epic's children under it, in list order", () => {
  const groups = buildGroups(
    list([
      item("e", "todo"),
      item("c1", "in_progress", { parentRef: "e" }),
      item("r", "todo"),
      item("c2", "todo", { parentRef: "e" }),
    ]),
  );
  expect(groups[0]!.items.map((i) => i.ref)).toEqual(["e", "r"]);
  expect(groups[0]!.epicChildren.get("e")!.map((i) => i.ref)).toEqual(["c1", "c2"]);
  expect(buildGroups(null)).toEqual([]);
});

// The list's own fields filter generically: keep what matches every
// chosen field; an epic anchors its children whatever its own value.
test("filterByFields keeps rows matching each chosen field; epics stay, their children filter", () => {
  const groups = [
    {
      epic: null,
      items: [
        item("u", "todo", { native: { author: "user", priority: "high" } }),
        item("a", "todo", { native: { author: "agent", priority: "high" } }),
        item("e", "todo", { native: { author: "agent" } }),
      ],
      epicChildren: new Map<string, WorkItem[]>([
        ["e", [item("c1", "todo", { native: { author: "user" } }), item("c2", "todo", { native: { author: "agent" } })]],
      ]),
    },
  ];
  const filtered = filterByFields(groups, { author: ["user"] });
  expect(filtered[0]!.items.map((i) => i.ref)).toEqual(["u", "e"]);
  expect(filtered[0]!.epicChildren.get("e")!.map((i) => i.ref)).toEqual(["c1"]);
  expect(filterByFields(groups, {})[0]!.items.map((i) => i.ref)).toEqual(["u", "a", "e"]);
});

test("applyStateFilter keeps or drops by state", () => {
  const groups = [{ epic: null, items: [item("t", "todo"), item("c", "canceled"), item("d", "done")], epicChildren: new Map() }];
  expect(applyStateFilter(groups, { exclude: ["canceled"] })[0]!.items.map((i) => i.ref)).toEqual(["t", "d"]);
  expect(applyStateFilter(groups, { only: ["done", "canceled"] })[0]!.items.map((i) => i.ref)).toEqual(["c", "d"]);
});
