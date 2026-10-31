import { afterEach, expect, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";

import { isNarrowWindow, useNarrowWindow } from "./windowWidth.js";

afterEach(cleanup);

test("a window under 800 px is narrow", () => {
  expect(isNarrowWindow(700)).toBe(true);
  expect(isNarrowWindow(799)).toBe(true);
  expect(isNarrowWindow(800)).toBe(false);
});

test("the hook follows the window as it shrinks and grows", () => {
  const set = (w: number) => {
    (window as unknown as { innerWidth: number }).innerWidth = w;
    window.dispatchEvent(new Event("resize"));
  };
  set(1200);
  const { result } = renderHook(() => useNarrowWindow());
  expect(result.current).toBe(false);
  act(() => set(700));
  expect(result.current).toBe(true);
  act(() => set(1200));
  expect(result.current).toBe(false);
});
