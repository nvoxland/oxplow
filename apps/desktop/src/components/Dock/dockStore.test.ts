import { expect, test } from "bun:test";

import { createDockStore, DOCK_MAX_HEIGHT_MARGIN, DOCK_MIN_HEIGHT, dockHeightOf } from "./dockStore.js";

test("toggle opens a panel, closes it again, and switches to another", () => {
  const s = createDockStore();
  expect(s.get().open).toBeNull();
  s.toggle("alerts");
  expect(s.get().open).toBe("alerts");
  s.toggle("context");
  expect(s.get().open).toBe("context");
  s.toggle("context");
  expect(s.get().open).toBeNull();
  s.open("alerts");
  s.close();
  expect(s.get().open).toBeNull();
});

test("subscribers hear each change, not a no-op", () => {
  const s = createDockStore();
  let n = 0;
  const off = s.subscribe(() => n++);
  s.open("a");
  s.open("a");
  expect(n).toBe(1);
  off();
  s.close();
  expect(n).toBe(1);
});

test("a remembered height below the minimum or not a number falls back", () => {
  expect(dockHeightOf(300.4)).toBe(300);
  expect(dockHeightOf(5)).toBe(DOCK_MIN_HEIGHT);
  expect(dockHeightOf("nope")).toBeGreaterThanOrEqual(DOCK_MIN_HEIGHT);
  expect(dockHeightOf(undefined)).toBeGreaterThanOrEqual(DOCK_MIN_HEIGHT);
  expect(DOCK_MAX_HEIGHT_MARGIN).toBe(80);
});
