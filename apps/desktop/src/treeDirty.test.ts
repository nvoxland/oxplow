import { expect, test } from "bun:test";

import { treeDirtyFrom } from "./treeDirty.js";
import { offeredWhen } from "./when.js";

test("a tree is dirty when its working change holds a file", () => {
  expect(treeDirtyFrom({ rows: [[1]] })).toBe(true);
  expect(treeDirtyFrom({ rows: [[0]] })).toBe(false);
  expect(treeDirtyFrom({ rows: [] })).toBe(false);
});

test("Commit Changes… is offered on a dirty tree and hidden on a clean one", () => {
  const when = "streamShown && vcsEnabled && treeDirty";
  const at = (treeDirty: boolean) => ({ streamShown: true, vcsEnabled: true, treeDirty });
  expect(offeredWhen(when, at(true))).toBe(true);
  expect(offeredWhen(when, at(false))).toBe(false);
});
