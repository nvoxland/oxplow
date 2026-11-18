import { expect, test } from "bun:test";

import { addInput, basketsFrom, defaultDestination, itemsFromResult, nextPrompt } from "./contextBasket.js";
import { selectionUri } from "./agent-context-ref.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

const result = (columns: string[], rows: unknown[][]) =>
  ({ columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

const items = itemsFromResult(
  result(
    ["id", "kind", "source_uri", "label", "text", "note", "basket"],
    [
      [1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null],
      [2, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", "Handle the empty case.", "Auth"],
      [3, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, null, "Release"],
    ],
  ),
);
const baskets = basketsFrom(
  items,
  result(
    ["id", "name", "attached_thread_id"],
    [
      [1, "Auth", 7],
      [2, "Release", null],
    ],
  ),
);

test("the thread's own items and each named basket's are apart", () => {
  expect(baskets.own.map((i) => i.id)).toEqual([1]);
  expect(baskets.named.map((b) => [b.name, b.attachedTo, b.items.map((i) => i.id)])).toEqual([
    ["Auth", "thr7", [2]],
    ["Release", null, [3]],
  ]);
  expect(items[1]).toMatchObject({ sourceUri: "oxplow:oxplow/file?path=a.rs", text: "fn main() {}", label: null, note: "Handle the empty case." });
});

test("a thread's next prompt carries its own items and its attached baskets'", () => {
  expect(nextPrompt(baskets, "thr7").map((i) => i.id)).toEqual([1, 2]);
  expect(nextPrompt(baskets, "thr8").map((i) => i.id)).toEqual([1]);
  expect(nextPrompt(baskets, null).map((i) => i.id)).toEqual([1]);
});

test("an add names its basket and stream, with only what is set", () => {
  expect(addInput({ thread: "thr1" }, "str2", { uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" })).toEqual({ thread: "thr1", stream: "str2", uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" });
  expect(addInput({ basket: "Auth" }, null, { uri: "u", text: "t", label: "" })).toEqual({ basket: "Auth", uri: "u", text: "t" });
  // A terminal's selection brings the lines around it.
  expect(addInput({ thread: "thr1" }, null, { uri: "u", text: "t", around: "a\nt\nb" })).toEqual({ thread: "thr1", uri: "u", text: "t", around: "a\nt\nb" });
  // A note goes with it when there is one, trimmed; a blank one is none.
  expect(addInput({ thread: "thr1" }, null, { uri: "u" }, "  Rename it.  ")).toEqual({ thread: "thr1", uri: "u", note: "Rename it." });
  expect(addInput({ thread: "thr1" }, null, { uri: "u" }, "   ")).toEqual({ thread: "thr1", uri: "u" });
});

test("the picker starts at the last basket while it is there, else This Thread, else the first", () => {
  expect(defaultDestination("thr1", ["Auth"], "auth")).toEqual({ basket: "Auth" });
  expect(defaultDestination("thr1", ["Auth"], "@thread")).toEqual({ thread: "thr1" });
  expect(defaultDestination("thr1", ["Auth"], "Gone")).toEqual({ thread: "thr1" });
  expect(defaultDestination(null, ["Auth"], "@thread")).toEqual({ basket: "Auth" });
  expect(defaultDestination(null, [], null)).toBeNull();
});

// A selection ending at column 1 of a line doesn't include that line.
test("a selection's lines are the same ones Ask About This names", () => {
  expect(selectionUri("a.rs", { startLineNumber: 3, endLineNumber: 5, endColumn: 9 })).toBe(selectionUri("a.rs", { startLineNumber: 3, endLineNumber: 6, endColumn: 1 }));
});
