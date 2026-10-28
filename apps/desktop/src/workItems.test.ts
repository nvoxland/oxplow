import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import {
  activeProviderOf,
  capabilityProvidersFromResult,
  featuresFor,
  itemsFromResult,
  workItemRefOfMention,
  workListProfileOf,
  NO_FEATURES,
  type CapabilityProvider,
  type WorkItem,
} from "./workItems.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: [
      "uri", "provider", "title", "body", "state", "parent_uri", "thread_id", "rank", "closed_at",
      "created_at", "updated_at", "native", "comment_count",
    ],
    rows,
    truncated: false,
    reads: { datasets: ["v_work_item", "v_work_item_comment"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

// Every item reads the same way, whichever list it's on: the interface's
// columns, and the list's own fields as `native`.
test("rows read as work items: the interface's columns and the list's own fields", () => {
  const items = itemsFromResult(
    result([
      ["oxplow:oxplow/work_item?id=tsk4&provider=oxplow", "oxplow", "Fix it", "", "in_progress", null, 2, 1.5, null, "t0", "t1", '{"priority":"high","author":"agent"}', 3],
      ["oxplow:oxplow/work_item?id=ENG-1&provider=issues", "issues", "Theirs", "b", "done", "oxplow:oxplow/work_item?id=ENG-0&provider=issues", null, null, "t2", "t0", "t1", null, 0],
    ]),
  );
  expect(items[0]).toEqual({
    uri: "oxplow:oxplow/work_item?id=tsk4&provider=oxplow",
    provider: "oxplow",
    title: "Fix it",
    body: "",
    state: "in_progress",
    parentUri: null,
    threadId: "thr2",
    rank: 1.5,
    closedAt: null,
    createdAt: "t0",
    updatedAt: "t1",
    native: { priority: "high", author: "agent" },
    commentCount: 3,
  });
  expect(items[1]).toMatchObject({ threadId: null, rank: null, closedAt: "t2", native: {}, parentUri: "oxplow:oxplow/work_item?id=ENG-0&provider=issues" });
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
    reads: { datasets: ["v_capability_provider"], tables: [], measures: [] },
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
  expect(workItemRefOfMention(profile, "tsk42")).toBe("oxplow:oxplow/work_item?id=tsk42&provider=oxplow");
  expect(workItemRefOfMention(profile, "tsk42x")).toBeNull();
  expect(workItemRefOfMention(profile, "ENG-1")).toBeNull();
  expect(workItemRefOfMention({ ...profile, provider: "issues", idPattern: "[A-Z]+-\\d+" }, "ENG-1")).toBe("oxplow:oxplow/work_item?id=ENG-1&provider=issues");
  expect(workItemRefOfMention({ ...profile, idPattern: null }, "tsk42")).toBeNull();
});
