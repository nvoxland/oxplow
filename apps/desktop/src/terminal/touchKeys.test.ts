import { expect, test } from "bun:test";

import { TOUCH_KEYS, ctrlOf, keyBytes } from "./touchKeys.js";

// An on-screen keyboard has no Esc, Tab, Ctrl or arrows, and an agent's
// first question (Claude's "trust this folder") needs the arrows. The touch
// bar sends what a hardware keyboard would.

test("Esc, Tab and Shift+Tab send what a keyboard does", () => {
  expect(keyBytes("esc", false)).toBe("\x1b");
  expect(keyBytes("tab", false)).toBe("\t");
  expect(keyBytes("backtab", false)).toBe("\x1b[Z");
});

test("arrows follow the program's cursor-key mode", () => {
  // Normal mode: a shell's line editor.
  expect(keyBytes("up", false)).toBe("\x1b[A");
  expect(keyBytes("down", false)).toBe("\x1b[B");
  expect(keyBytes("right", false)).toBe("\x1b[C");
  expect(keyBytes("left", false)).toBe("\x1b[D");
  // Application mode (DECCKM): full-screen programs and menus, an agent's included.
  expect(keyBytes("up", true)).toBe("\x1bOA");
  expect(keyBytes("left", true)).toBe("\x1bOD");
});

test("Ctrl turns the next typed letter into its control character", () => {
  expect(ctrlOf("c")).toBe("\x03");
  expect(ctrlOf("C")).toBe("\x03");
  expect(ctrlOf("d")).toBe("\x04");
  expect(ctrlOf("[")).toBe("\x1b");
  expect(ctrlOf(" ")).toBe("\x00");
  // A whole word from a keyboard's suggestion, or a key Ctrl means nothing
  // with, goes as it is.
  expect(ctrlOf("hello")).toBeNull();
  expect(ctrlOf("1")).toBeNull();
});

test("the bar lists every key once, in the order drawn", () => {
  expect(TOUCH_KEYS.map((k) => k.id)).toEqual(["esc", "backtab", "tab", "ctrl", "left", "up", "down", "right"]);
  expect(new Set(TOUCH_KEYS.map((k) => k.label)).size).toBe(TOUCH_KEYS.length);
});
