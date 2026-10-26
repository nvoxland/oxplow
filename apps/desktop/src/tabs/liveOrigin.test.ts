import { expect, test } from "bun:test";

import { resolvedOrigin } from "./liveOrigin.js";
import { fileRef } from "./pageRefs.js";
import type { Origin } from "./session.js";

const live: Origin = { title: "L", items: [], index: 0, live: { node: "x/y", params: {}, column: "k" } };
const item = (path: string) => ({ ref: fileRef(path), label: path });

// A live origin is only a page node to read: the list and the place in it
// are worked out from what the node shows now.
test("a live origin takes its items and place from the resolved list", () => {
  const out = resolvedOrigin(live, [item("a"), item("b"), item("c")], fileRef("b"))!;
  expect(out.items.map((i) => i.label)).toEqual(["a", "b", "c"]);
  expect(out.index).toBe(1);
  expect(out.title).toBe("L");
});

test("a page the list no longer has, or a list not yet run, has no origin", () => {
  expect(resolvedOrigin(live, [item("a"), item("b")], fileRef("z"))).toBeNull();
  expect(resolvedOrigin(live, undefined, fileRef("a"))).toBeNull();
});

test("a stored origin is its own", () => {
  const stored: Origin = { items: [item("a"), item("b")], index: 1 };
  expect(resolvedOrigin(stored, undefined, fileRef("b"))).toBe(stored);
});

// A page file's list names its node (`<page>#<node>`): its live origin
// reads that node's rows in the producing page's run.
test("a page node's live origin reads the node's rows of its page's run", async () => {
  const { liveOriginNodeRun } = await import("./liveOrigin.js");
  const { decl, fakePage, fakeRun, loader, node } = await import("../pages/testing/fakeRun.js");
  const page = fakePage(node.stack("s", [node.viz("items", "rows", { columns: [{ key: "k", link: "{{k}}" } as never] })]), { id: "acme/list", loaders: [decl("rows")] });
  const run = fakeRun(page, { loaders: { rows: loader(["k"], [["oxplow:oxplow/file?path=a"], ["oxplow:oxplow/file?path=b"]]) } });
  const out = liveOriginNodeRun({ node: "acme/list#items", params: { q: 1 }, column: "k" }, run);
  expect(out?.result.rows.length).toBe(2);
  expect(out?.config.columns.map((c) => c.key)).toEqual(["k"]);
  // Another page's run, a node it no longer has, or no node named: none.
  expect(liveOriginNodeRun({ node: "acme/other#items", params: {}, column: "k" }, run)).toBeNull();
  expect(liveOriginNodeRun({ node: "acme/list#gone", params: {}, column: "k" }, run)).toBeNull();
  expect(liveOriginNodeRun({ node: "acme/list", params: {}, column: "k" }, run)).toBeNull();
});
