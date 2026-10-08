import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import {
  activeProviderOf,
  boardColumns,
  bucketWorkList,
  capabilityProvidersFromResult,
  createWorkItemInput,
  effortDetailsFromResult,
  featuresFor,
  itemsFromResult,
  placementFromOrder,
  workItemsQuery,
  workItemRefOfMention,
  workListProfileOf,
  NO_FEATURES,
  type CapabilityProvider,
  type WorkItem,
} from "./workItems.js";

// An item's activity is one read over the models: `v_effort` joined to
// `v_effort_file` (one row per effort and file; an effort with no files
// still appears), newest effort first.
test("effort rows join to their files: one detail per effort, counts by change kind", () => {
  const res = {
    columns: ["id", "work_item", "started_at", "ended_at", "start_snapshot_id", "end_snapshot_id", "summary", "path", "change_kind"],
    rows: [
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "src/a.rs", "updated"],
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "src/b.rs", "created"],
      [7, "work_item:oxplow:tsk4", "t2", null, 10, null, null, "old.rs", "deleted"],
      [5, "work_item:oxplow:tsk4", "t0", "t1", 8, 9, "did it", null, null],
    ],
    truncated: false,
    reads: { models: ["v_effort", "v_effort_file"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const details = effortDetailsFromResult(res);
  expect(details.map((d) => d.effort.id)).toEqual(["eff7", "eff5"]);
  expect(details[0]).toEqual({
    effort: { id: "eff7", work_item: "work_item:oxplow:tsk4", started_at: "t2", ended_at: null, start_snapshot_id: "10", end_snapshot_id: null, summary: null },
    start_snapshot: null,
    end_snapshot: null,
    changed_paths: ["src/a.rs", "src/b.rs", "old.rs"],
    counts: { created: 1, updated: 1, deleted: 1 },
  });
  expect(details[1]?.changed_paths).toEqual([]);
  expect(details[1]?.effort.summary).toBe("did it");
});

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: [
      "ref", "provider", "title", "body", "state", "parent_ref", "thread_id", "rank", "closed_at",
      "created_at", "updated_at", "native", "comment_count",
    ],
    rows,
    truncated: false,
    reads: { models: ["v_work_item", "v_work_item_comment"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

// Every item reads the same way, whichever list it's on: the interface's
// columns, and the list's own fields as `native`.
test("rows read as work items: the interface's columns and the list's own fields", () => {
  const items = itemsFromResult(
    result([
      ["work_item:oxplow:tsk4", "oxplow", "Fix it", "", "in_progress", null, 2, 1.5, null, "t0", "t1", '{"priority":"high","author":"agent"}', 3],
      ["work_item:issues:ENG-1", "issues", "Theirs", "b", "done", "work_item:issues:ENG-0", null, null, "t2", "t0", "t1", null, 0],
    ]),
  );
  expect(items[0]).toEqual({
    ref: "work_item:oxplow:tsk4",
    provider: "oxplow",
    title: "Fix it",
    body: "",
    state: "in_progress",
    parentRef: null,
    threadId: "thr2",
    rank: 1.5,
    closedAt: null,
    createdAt: "t0",
    updatedAt: "t1",
    native: { priority: "high", author: "agent" },
    commentCount: 3,
  });
  expect(items[1]).toMatchObject({ threadId: null, rank: null, closedAt: "t2", native: {}, parentRef: "work_item:issues:ENG-0" });
});

test("a query scoped to a thread, the backlog, or everything; states filter; list order", () => {
  expect(workItemsQuery({ scope: { thread: "thr2" } }).params).toEqual([2]);
  expect(workItemsQuery({ scope: { thread: "thr2" } }).sql).toContain("w.thread_id = ?1");
  // The backlog is any list's: no thread.
  expect(workItemsQuery({ scope: "backlog" }).sql).toContain("w.thread_id IS NULL");
  const all = workItemsQuery({ scope: "all", states: ["todo", "blocked"] });
  expect(all.sql).toContain("w.state IN ('todo', 'blocked')");
  expect(all.sql).toContain("ORDER BY w.rank IS NULL, w.rank, w.created_at");
});

test("the Board groups by canonical state, in workflow order, each column in list order", () => {
  const item = (ref: string, state: WorkItem["state"]): WorkItem => ({ ref, state, title: ref }) as unknown as WorkItem;
  const columns = boardColumns([item("a", "done"), item("b", "todo"), item("c", "todo"), item("d", "blocked")]);
  expect(columns.map((c) => [c.state, c.items.map((i) => i.ref)])).toEqual([
    ["todo", ["b", "c"]],
    ["in_progress", []],
    ["blocked", ["d"]],
    ["done", ["a"]],
    ["canceled", []],
  ]);
});

// A list: an item with a child on it is an epic; the rest by state, done
// and canceled together. `all` keeps the list order.
test("a work list: an item with a child is an epic, the rest by state", () => {
  const item = (ref: string, state: WorkItem["state"], parentRef: string | null = null) =>
    ({ ref, state, parentRef }) as unknown as WorkItem;
  const list = bucketWorkList(
    "thr2",
    [item("e", "todo"), item("c", "in_progress", "e"), item("b", "blocked"), item("d", "done"), item("x", "canceled"), item("n", "todo")],
    [],
    { models: [], tables: [], measures: [] },
  );
  expect(list.epics.map((i) => i.ref)).toEqual(["e"]);
  expect(list.inProgress.map((i) => i.ref)).toEqual(["c"]);
  expect(list.waiting.map((i) => i.ref)).toEqual(["b"]);
  expect(list.done.map((i) => i.ref)).toEqual(["d", "x"]);
  expect(list.items.map((i) => i.ref)).toEqual(["n"]);
  expect(list.all.map((i) => i.ref)).toEqual(["e", "c", "b", "d", "x", "n"]);
});

test("a reordered list is one item placed next to a neighbour", () => {
  expect(placementFromOrder(["a", "b", "c", "d"], ["a", "d", "b", "c"])).toEqual({ id: "d", place: { after: "a" } });
  expect(placementFromOrder(["a", "b", "c"], ["c", "a", "b"])).toEqual({ id: "c", place: { before: "a" } });
  expect(placementFromOrder(["a", "b", "c"], ["b", "c", "a"])).toEqual({ id: "a", place: { after: "c" } });
  expect(placementFromOrder(["a", "b"], ["a", "b"])).toBeNull();
});

// A provider's flags and fields come from v_capability_provider; a
// provider the model doesn't list (or a flag it doesn't declare) is off.
test("capability providers read with their features and fields; an unknown provider has none", () => {
  const providers = capabilityProvidersFromResult({
    columns: ["capability", "provider", "extension", "features", "fields", "active"],
    rows: [
      [
        "work_items",
        "oxplow",
        null,
        '{"hierarchy":true,"comments":true,"links":true,"delete":true,"ordering":true,"lists":true}',
        '[{"name":"priority","title":"Priority","kind":"enum","values":["high","low"],"read_only":false}]',
        1,
      ],
      ["work_items", "fake", "tracker", '{"comments":true}', "[]", 0],
    ],
    truncated: false,
    reads: { models: ["v_capability_provider"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult);
  expect(featuresFor(providers, "oxplow")).toEqual({ hierarchy: true, comments: true, links: true, delete: true, idempotent_writes: false, ordering: true, lists: true });
  expect(featuresFor(providers, "fake")).toEqual({ hierarchy: false, comments: true, links: false, delete: false, idempotent_writes: false, ordering: false, lists: false });
  expect(featuresFor(providers, "issues")).toEqual({ hierarchy: false, comments: false, links: false, delete: false, idempotent_writes: false, ordering: false, lists: false });
  expect(providers.find((p) => p.provider === "fake")?.extension).toBe("tracker");
  expect(workListProfileOf(providers)).toEqual({
    provider: "oxplow",
    features: { hierarchy: true, comments: true, links: true, delete: true, idempotent_writes: false, ordering: true, lists: true },
    fields: [{ name: "priority", title: "Priority", kind: "enum", values: ["high", "low"], read_only: false }],
    idPattern: null,
  });
});

// Every create files on the active list, so the input names none: the
// thread, the canonical state and the list's own fields under `native`.
test("a new item's input names no list and files on the thread", () => {
  expect(
    createWorkItemInput("thr2", {
      title: "Fix it",
      body: "why",
      state: "blocked",
      parentRef: "work_item:oxplow:tsk1",
      native: { priority: "high" },
    }),
  ).toEqual({
    title: "Fix it",
    body: "why",
    state: "blocked",
    parent_ref: "work_item:oxplow:tsk1",
    thread: "thr2",
    native: { priority: "high" },
  });
  expect(createWorkItemInput(null, { title: "Later", native: {} })).toEqual({ title: "Later" });
});

test("the active work-items provider is the row marked active", () => {
  const row = (provider: string, active: boolean): CapabilityProvider => ({
    capability: "work_items",
    provider,
    extension: provider === "oxplow" ? null : "tracker",
    features: {},
    fields: [],
    idPattern: null,
    active,
  });
  expect(activeProviderOf([row("oxplow", false), row("fake", true)])).toBe("fake");
  expect(activeProviderOf([row("oxplow", true), row("fake", false)])).toBe("oxplow");
  expect(activeProviderOf([])).toBeNull();
  expect(workListProfileOf([])).toEqual({ provider: null, features: NO_FEATURES, fields: [], idPattern: null });
});

// A loose id in text names an item of the active list only when the list
// says what its ids look like (none says nothing).
test("a mention resolves through the active list's id pattern", () => {
  const profile = { provider: "oxplow", features: NO_FEATURES, fields: [], idPattern: "tsk\\d+" };
  expect(workItemRefOfMention(profile, "tsk42")).toBe("work_item:oxplow:tsk42");
  expect(workItemRefOfMention(profile, "tsk42x")).toBeNull();
  expect(workItemRefOfMention(profile, "ENG-1")).toBeNull();
  expect(workItemRefOfMention({ ...profile, provider: "issues", idPattern: "[A-Z]+-\\d+" }, "ENG-1")).toBe("work_item:issues:ENG-1");
  expect(workItemRefOfMention({ ...profile, idPattern: null }, "tsk42")).toBeNull();
});
