import { afterEach, expect, test } from "bun:test";

import { createDockStore, DOCK_MAX_HEIGHT_MARGIN, DOCK_MIN_HEIGHT } from "./dockStore.js";

const realStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
afterEach(() => {
  if (realStorage) Object.defineProperty(globalThis, "localStorage", realStorage);
  else delete (globalThis as { localStorage?: unknown }).localStorage;
});

function fakeStorage(initial: Record<string, string> = {}) {
  const data = { ...initial };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (k: string) => data[k] ?? null,
      setItem: (k: string, v: string) => {
        data[k] = v;
      },
      removeItem: (k: string) => {
        delete data[k];
      },
    },
  });
  return data;
}

test("toggle opens a panel, closes it again, and switches to another", () => {
  fakeStorage();
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
  fakeStorage();
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

test("height is clamped, remembered, and read back", () => {
  const data = fakeStorage();
  const s = createDockStore();
  s.setHeight(10);
  expect(s.get().height).toBe(DOCK_MIN_HEIGHT);
  s.setHeight(300.4);
  expect(s.get().height).toBe(300);
  expect(data["oxplow.dock.height"]).toBe("300");
  expect(createDockStore().get().height).toBe(300);
});

test("a stored height below the minimum or not a number falls back", () => {
  fakeStorage({ "oxplow.dock.height": "nope" });
  expect(createDockStore().get().height).toBeGreaterThanOrEqual(DOCK_MIN_HEIGHT);
  fakeStorage({ "oxplow.dock.height": "5" });
  expect(createDockStore().get().height).toBe(DOCK_MIN_HEIGHT);
  expect(DOCK_MAX_HEIGHT_MARGIN).toBe(80);
});

test("storage that throws does not break the store", () => {
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    get() {
      throw new Error("blocked");
    },
  });
  const s = createDockStore();
  s.setHeight(250);
  expect(s.get().height).toBe(250);
  s.toggle("alerts");
  expect(s.get().open).toBe("alerts");
});
