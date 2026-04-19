import { expect, test } from "bun:test";
import { fuzzyMatches } from "./CommandPalette.js";

test("subsequence matches skip over unrelated characters", () => {
  expect(fuzzyMatches("work new work item", "wn")).toBe(true);
  expect(fuzzyMatches("file save", "fs")).toBe(true);
  expect(fuzzyMatches("work new work item", "nwi")).toBe(true);
});

test("characters must appear in order", () => {
  expect(fuzzyMatches("file save", "ef")).toBe(false);
  expect(fuzzyMatches("work save", "ws x")).toBe(false);
});

test("empty query matches everything (palette shows the full list)", () => {
  expect(fuzzyMatches("anything", "")).toBe(true);
});

test("non-matching characters fail the match", () => {
  expect(fuzzyMatches("file save", "zzz")).toBe(false);
});
