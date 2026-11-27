import { expect, test } from "bun:test";
import { createDevice } from "./device.js";

const UUID = "123e4567-e89b-42d3-a456-426614174000";

function store(initial: string | null) {
  let value = initial;
  return { get: () => value, set: (v: string) => void (value = v), peek: () => value };
}

test("the Mac app's device is the one its shell injects", () => {
  const stored = store(null);
  const d = createDevice({ injected: { id: `dev-${UUID}`, name: "Nathan's Mac" }, stored, coarsePointer: true });
  expect(d).toEqual({ id: `dev-${UUID}`, name: "Nathan's Mac", kind: "desktop" });
  expect(stored.peek()).toBeNull();
});

test("a browser makes a dev- id once and keeps it", () => {
  const stored = store(null);
  const first = createDevice({ injected: null, stored, coarsePointer: false, generate: () => UUID });
  expect(first).toEqual({ id: `dev-${UUID}`, name: "Browser", kind: "desktop" });
  expect(stored.peek()).toBe(`dev-${UUID}`);
  const again = createDevice({ injected: null, stored, coarsePointer: false, generate: () => "other" });
  expect(again.id).toBe(`dev-${UUID}`);
});

test("a stored id that is not dev-<uuid> is replaced", () => {
  const stored = store("garbage");
  const d = createDevice({ injected: null, stored, coarsePointer: false, generate: () => UUID });
  expect(d.id).toBe(`dev-${UUID}`);
  expect(stored.peek()).toBe(`dev-${UUID}`);
});

test("a browser with a coarse pointer is a tablet", () => {
  const d = createDevice({ injected: null, stored: store(null), coarsePointer: true });
  expect(d.kind).toBe("tablet");
  expect(d.id).toMatch(/^dev-[0-9a-f-]{36}$/);
});
