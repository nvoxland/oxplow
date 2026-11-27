import { afterEach, expect, test } from "bun:test";
import { cleanup, renderHook } from "@testing-library/react";
import { useReportUse } from "./useReportUse.js";

// The window tells the daemon it is in use, at most once a second, on a
// press or a key.

afterEach(() => cleanup());

test("a press or a key is told, at most once a second", () => {
  let at = 0;
  let told = 0;
  const report = () => (told += 1);
  const now = () => at;
  renderHook(() => useReportUse(report, 1_000, now));
  window.dispatchEvent(new Event("pointerdown"));
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
  expect(told).toBe(1);
  at = 1_500;
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "b" }));
  expect(told).toBe(2);
});
