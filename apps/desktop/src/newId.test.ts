import { afterEach, expect, test } from "bun:test";
import { newId } from "./newId.js";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const real = globalThis.crypto;
afterEach(() => { Object.defineProperty(globalThis, "crypto", { value: real, configurable: true }); });

test("newId uses randomUUID when it exists", () => {
  Object.defineProperty(globalThis, "crypto", { value: { randomUUID: () => "from-native" }, configurable: true });
  expect(newId()).toBe("from-native");
});

test("newId makes a v4-shaped id from getRandomValues on plain HTTP", () => {
  Object.defineProperty(globalThis, "crypto", { value: { getRandomValues: (a: Uint8Array) => real.getRandomValues(a) }, configurable: true });
  const a = newId();
  expect(a).toMatch(UUID);
  expect(newId()).not.toBe(a);
});
