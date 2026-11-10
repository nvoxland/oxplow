import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import {
  activeProviderOf,
  capabilityProvidersFromResult,
  featuresFor,
  itemsFromResult,
  taskRefOfMention,
  taskTrackerProfileOf,
  NO_FEATURES,
  type CapabilityProvider,
  type Task,
} from "./tasks.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: [
      "uri", "provider", "title", "body", "state", "stage", "stage_title", "parent_uri", "assignee", "rank", "closed_at",
      "created_at", "updated_at", "native", "comment_count",
    ],
    rows,
    truncated: false,
    reads: { datasets: ["v_task", "v_task_comment"], tables: [], measures: [] },
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

// A provider's flags and fields come from v_capability_provider; a
// provider the model doesn't list (or a flag it doesn't declare) is off.
test("capability providers read with their features and fields; an unknown provider has none", () => {
  const providers = capabilityProvidersFromResult({
    columns: ["capability", "provider", "extension", "features", "fields", "stages", "active"],
    rows: [
      [
        "task_tracker",
        "oxtask",
        null,
        '{"hierarchy":true,"comments":true,"links":true,"delete":true}',
        '[{"name":"priority","title":"Priority","kind":"enum","values":["high","low"],"read_only":false}]',
        '[{"id":"ready","title":"Ready","bucket":"todo"},{"id":"done","title":"Done","bucket":"done"}]',
        1,
      ],
      ["task_tracker", "fake", "tracker", '{"comments":true}', "[]", null, 0],
    ],
    truncated: false,
    reads: { datasets: ["v_capability_provider"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult);
  expect(featuresFor(providers, "oxtask")).toEqual({ hierarchy: true, comments: true, links: true, delete: true, idempotent_writes: false });
  expect(featuresFor(providers, "fake")).toEqual({ hierarchy: false, comments: true, links: false, delete: false, idempotent_writes: false });
  expect(featuresFor(providers, "issues")).toEqual({ hierarchy: false, comments: false, links: false, delete: false, idempotent_writes: false });
  expect(providers.find((p) => p.provider === "fake")?.extension).toBe("tracker");
  expect(taskTrackerProfileOf(providers)).toEqual({
    provider: "oxtask",
    features: { hierarchy: true, comments: true, links: true, delete: true, idempotent_writes: false },
    fields: [{ name: "priority", title: "Priority", kind: "enum", values: ["high", "low"], read_only: false }],
    stages: [
      { id: "ready", title: "Ready", bucket: "todo" },
      { id: "done", title: "Done", bucket: "done" },
    ],
    idPattern: null,
  });
  expect(providers.find((p) => p.provider === "fake")?.stages).toEqual([]);
});

test("the active task tracker provider is the row marked active", () => {
  const row = (provider: string, active: boolean): CapabilityProvider => ({
    capability: "task_tracker",
    provider,
    extension: provider === "oxtask" ? null : "tracker",
    features: {},
    fields: [],
    stages: [],
    idPattern: null,
    active,
  });
  expect(activeProviderOf([row("oxtask", false), row("fake", true)])).toBe("fake");
  expect(activeProviderOf([row("oxtask", true), row("fake", false)])).toBe("oxtask");
  expect(activeProviderOf([])).toBeNull();
  expect(taskTrackerProfileOf([])).toEqual({ provider: null, features: NO_FEATURES, fields: [], stages: [], idPattern: null });
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
