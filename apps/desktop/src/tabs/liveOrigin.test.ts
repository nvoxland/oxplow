import { expect, test } from "bun:test";

import { resolvedOrigin } from "./liveOrigin.js";
import { fileRef } from "./pageRefs.js";
import type { Origin } from "./session.js";

const live: Origin = { title: "L", items: [], index: 0, live: { lens: "x/y", params: {}, column: "k" } };
const item = (path: string) => ({ ref: fileRef(path), label: path });

// A live origin is only a lens to run: the list and the place in it are
// worked out from what the lens returns now.
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

// A page file's list names its node's lens id (`<page>#<node>`): its live
// origin runs the page and reads that node, never a lens of that name.
test("a page node's live origin runs its page and reads the node's rows", async () => {
  const { liveOriginRun } = await import("./liveOrigin.js");
  const { decl, fakePage, fakeRun, loader, node } = await import("../pages/testing/fakeRun.js");
  const ran: unknown[][] = [];
  const page = fakePage(node.stack("s", [node.viz("items", "rows", { columns: [{ key: "k", link: "{{k}}" } as never] })]), { id: "acme/list", loaders: [decl("rows")] });
  const run = fakeRun(page, { loaders: { rows: loader(["k"], [["oxplow:oxplow/file?path=a"], ["oxplow:oxplow/file?path=b"]]) } });
  const out = await liveOriginRun({ lens: "acme/list#items", params: { q: 1 }, column: "k" }, "str1", {
    runLens: async () => {
      throw new Error("no lens acme/list#items");
    },
    runPage: async (...args) => {
      ran.push(args);
      return run;
    },
  });
  expect(ran).toEqual([["acme/list", { q: 1 }, "str1"]]);
  expect(out?.result.rows.length).toBe(2);
  expect(out?.lens.columns.map((c) => c.key)).toEqual(["k"]);
});
