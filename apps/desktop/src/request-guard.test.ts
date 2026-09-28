import { expect, test } from "bun:test";
import { createRequestGuard } from "./request-guard.js";

test("only the latest request may apply its result", () => {
  const guard = createRequestGuard();
  const a = guard.begin();
  const b = guard.begin();
  expect(a()).toBe(false);
  expect(b()).toBe(true);
});

test("cancelling retires every request in flight", () => {
  const guard = createRequestGuard();
  const a = guard.begin();
  guard.cancel();
  expect(a()).toBe(false);
  expect(guard.begin()()).toBe(true);
});
