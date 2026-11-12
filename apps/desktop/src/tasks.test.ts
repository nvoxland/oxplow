import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { itemsFromResult, taskRefOfMention, taskTrackerProfileOf, NO_FEATURES, type Task } from "./tasks.js";

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

// The profile reads the task model's declarations: the first store's
// stages in order, the stores' own fields (not oxplow's assignee and rank),
// and what an item can do; no store, nothing.
test("the profile reads the stores' stages and fields; no store has none", () => {
  const stages = {
    columns: ["store", "id", "title", "bucket"],
    rows: [
      ["oxtask", "ready", "Ready", "todo"],
      ["oxtask", "done", "Done", "done"],
    ],
    truncated: false,
    reads: { datasets: ["v_model_stage"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const fields = {
    columns: ["name", "title", "kind", "values", "read_only"],
    rows: [["priority", "Priority", "enum", '["high","low"]', 0]],
    truncated: false,
    reads: { datasets: ["v_model_field_decl"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  expect(taskTrackerProfileOf(stages, fields)).toEqual({
    provider: "oxtask",
    features: { hierarchy: true, comments: true, links: true, delete: true, idempotent_writes: false },
    fields: [{ name: "priority", title: "Priority", kind: "enum", values: ["high", "low"], read_only: false }],
    stages: [
      { id: "ready", title: "Ready", bucket: "todo" },
      { id: "done", title: "Done", bucket: "done" },
    ],
    idPattern: null,
  });
  const none = { ...stages, rows: [] } as unknown as SqlQueryResult;
  expect(taskTrackerProfileOf(none, fields)).toEqual({ provider: null, features: NO_FEATURES, fields: [], stages: [], idPattern: null });
});

// A loose id in text names an item of the active list only when the list
// says what its ids look like (none says nothing).
test("a mention resolves through the active list's id pattern", () => {
  const profile = { provider: "oxtask", features: NO_FEATURES, fields: [], stages: [], idPattern: "tsk\\d+" };
  expect(taskRefOfMention(profile, "tsk42")).toBe("oxplow:oxplow/task?id=tsk42&provider=oxtask");
  expect(taskRefOfMention(profile, "tsk42x")).toBeNull();
  expect(taskRefOfMention(profile, "ENG-1")).toBeNull();
  expect(taskRefOfMention({ ...profile, provider: "issues", idPattern: "[A-Z]+-\\d+" }, "ENG-1")).toBe("oxplow:oxplow/task?id=ENG-1&provider=issues");
  expect(taskRefOfMention({ ...profile, idPattern: null }, "tsk42")).toBeNull();
});
