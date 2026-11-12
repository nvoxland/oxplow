import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { itemsFromResult, taskRefOfMention, taskTrackerProfilesOf, profileOfTask, type Task } from "./tasks.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: [
      "uri", "provider", "title", "body", "state", "stage", "stage_title", "parent_uri", "assignee", "rank", "closed_at",
      "created_at", "updated_at", "native", "comment_count",
    ],
    rows,
    truncated: false,
    reads: { datasets: ["v_oxplow_task", "v_oxplow_task_comment"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

// Every item reads the same way, whichever list it's on: the interface's
// columns, and the list's own fields as `native`.
test("rows read as tasks: the interface's columns and the list's own fields", () => {
  const items = itemsFromResult(
    result([
      ["oxplow:oxplow/task?id=tsk4&provider=oxtask", "oxtask", "Fix it", "", "in_progress", "in_progress", "In Progress", null, "oxplow:oxplow/thread?id=thr2", 1.5, null, "t0", "t1", '{"priority":"high","author":"agent"}', 3],
      ["oxplow:oxplow/task?id=ENG-1&provider=issues", "issues", "Theirs", "b", "done", "shipped", null, "oxplow:oxplow/task?id=ENG-0&provider=issues", null, null, "t2", "t0", "t1", null, 0],
    ]),
  );
  expect(items[0]).toEqual({
    uri: "oxplow:oxplow/task?id=tsk4&provider=oxtask",
    provider: "oxtask",
    title: "Fix it",
    body: "",
    state: "in_progress",
    stage: "in_progress",
    stageTitle: "In Progress",
    parentUri: null,
    assignee: "oxplow:oxplow/thread?id=thr2",
    rank: 1.5,
    closedAt: null,
    createdAt: "t0",
    updatedAt: "t1",
    native: { priority: "high", author: "agent" },
    commentCount: 3,
  });
  // A stage the list doesn't title shows its id.
  expect(items[1]).toMatchObject({ state: "done", stage: "shipped", stageTitle: "shipped", assignee: null, rank: null, closedAt: "t2", native: {}, parentUri: "oxplow:oxplow/task?id=ENG-0&provider=issues" });
});

const rowsOf = (columns: string[], rows: unknown[][]): SqlQueryResult =>
  ({ columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

const STAGE_COLUMNS = ["store", "id", "title", "bucket"];
const FIELD_COLUMNS = ["namespace", "name", "title", "kind", "values", "read_only"];
const STORE_COLUMNS = ["store", "id_pattern"];

// The profiles are per store: each store's own stages (in order), fields
// (not oxplow's assignee and rank) and id pattern. No store, no profile.
test("each store has its own profile; no store has none", () => {
  const stages = rowsOf(STAGE_COLUMNS, [
    ["oxtask", "ready", "Ready", "todo"],
    ["oxtask", "done", "Done", "done"],
    ["issues", "open", "Open", "todo"],
    ["issues", "review", "In review", "in_progress"],
    ["issues", "closed", "Closed", "done"],
  ]);
  const fields = rowsOf(FIELD_COLUMNS, [
    ["oxtask", "priority", "Priority", "enum", '["high","low"]', 0],
    ["issues", "milestone", "Milestone", "text", null, 1],
    ["oxplow", "assignee", "Assignee", "text", null, 0],
  ]);
  const stores = rowsOf(STORE_COLUMNS, [
    ["oxtask", "tsk[0-9]+"],
    ["issues", "[A-Z]+-[0-9]+"],
  ]);
  const profiles = taskTrackerProfilesOf(stages, fields, stores);
  expect(Object.keys(profiles).sort()).toEqual(["issues", "oxtask"]);
  expect(profiles.oxtask).toEqual({
    provider: "oxtask",
    fields: [{ name: "priority", title: "Priority", kind: "enum", values: ["high", "low"], read_only: false }],
    stages: [
      { id: "ready", title: "Ready", bucket: "todo" },
      { id: "done", title: "Done", bucket: "done" },
    ],
    idPattern: "tsk[0-9]+",
  });
  // The second store keeps its own stages, fields and pattern.
  expect(profiles.issues.stages.map((s) => s.id)).toEqual(["open", "review", "closed"]);
  expect(profiles.issues.fields.map((f) => f.name)).toEqual(["milestone"]);
  expect(profiles.issues.fields[0].read_only).toBe(true);
  expect(profiles.issues.idPattern).toBe("[A-Z]+-[0-9]+");
  // A task's profile is its own store's.
  expect(profileOfTask(profiles, { provider: "issues" })?.stages.length).toBe(3);
  expect(profileOfTask(profiles, { provider: "gone" })).toBeNull();
  expect(taskTrackerProfilesOf(rowsOf(STAGE_COLUMNS, []), fields)).toEqual({});
});

// A store with no stages row still has a profile when it is a store.
test("a store without stages has an empty profile", () => {
  const profiles = taskTrackerProfilesOf(rowsOf(STAGE_COLUMNS, []), rowsOf(FIELD_COLUMNS, []), rowsOf(STORE_COLUMNS, [["wiki", null]]));
  expect(profiles.wiki).toEqual({ provider: "wiki", fields: [], stages: [], idPattern: null });
});

// The UI offers what a store's action points offer: a profile declares no
// abilities of its own.
test("a profile has no feature flags", () => {
  const profiles = taskTrackerProfilesOf(rowsOf(STAGE_COLUMNS, [["oxtask", "a", "A", "todo"]]), rowsOf(FIELD_COLUMNS, []));
  expect("features" in profiles.oxtask).toBe(false);
  expect(Object.keys(profiles.oxtask).sort()).toEqual(["fields", "idPattern", "provider", "stages"]);
});

// A loose id in text names an item of a store only when the store says
// what its ids look like (none says nothing).
test("a mention resolves through the stores' id patterns", () => {
  const profile = (provider: string, idPattern: string | null) => ({ provider, fields: [], stages: [], idPattern });
  const profiles = { oxtask: profile("oxtask", "tsk\\d+"), issues: profile("issues", "[A-Z]+-\\d+") };
  expect(taskRefOfMention(profiles, "tsk42")).toBe("oxplow:oxplow/task?id=tsk42&provider=oxtask");
  expect(taskRefOfMention(profiles, "tsk42x")).toBeNull();
  expect(taskRefOfMention(profiles, "ENG-1")).toBe("oxplow:oxplow/task?id=ENG-1&provider=issues");
  expect(taskRefOfMention({ oxtask: profile("oxtask", null) }, "tsk42")).toBeNull();
  expect(taskRefOfMention({}, "tsk42")).toBeNull();
});
