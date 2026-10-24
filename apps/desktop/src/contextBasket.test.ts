import { expect, test } from "bun:test";

import { addDisabledReason, addInput, basketFromResult, waiting } from "./contextBasket.js";
import { selectionUri } from "./agent-context-ref.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

const result = {
  columns: ["id", "kind", "source_uri", "label", "text", "delivered_at"],
  rows: [
    [1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null],
    [2, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", null],
    [3, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, "2026-10-24T10:00:00Z"],
  ],
  truncated: false,
  reads: { models: [], tables: [], measures: [] },
  freshness: {},
} as unknown as SqlQueryResult;

test("the basket reads its rows; only the undelivered ones wait", () => {
  const items = basketFromResult(result);
  expect(items.map((i) => [i.id, i.kind, i.delivered])).toEqual([
    [1, "page", false],
    [2, "text", false],
    [3, "page", true],
  ]);
  expect(waiting(items).map((i) => i.id)).toEqual([1, 2]);
  expect(items[1]).toMatchObject({ sourceUri: "oxplow:oxplow/file?path=a.rs", text: "fn main() {}", label: null });
});

test("a page and a text are both one add, with only what is set", () => {
  expect(addInput("thr1", { uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" })).toEqual({ thread: "thr1", uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" });
  expect(addInput("thr1", { uri: "u", text: "t", label: "" })).toEqual({ thread: "thr1", uri: "u", text: "t" });
});

test("without a thread there is nowhere to add, and it says so", () => {
  expect(addDisabledReason(null)).toBe("Select a thread first");
  expect(addDisabledReason("thr1")).toBeNull();
});

// A selection ending at column 1 of a line doesn't include that line.
test("a selection's lines are the same ones Ask About This names", () => {
  expect(selectionUri("a.rs", { startLineNumber: 3, endLineNumber: 5, endColumn: 9 })).toBe(selectionUri("a.rs", { startLineNumber: 3, endLineNumber: 6, endColumn: 1 }));
});
