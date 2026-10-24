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
