import { expect, test } from "bun:test";
import { fuzzyMatches } from "./fuzzy-match.js";

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

test("useful for quick-open: non-contiguous filename typing", () => {
  expect(fuzzyMatches("src/ui/app.tsx", "apptsx")).toBe(true);
  expect(fuzzyMatches("src/ui/app.tsx", "srctsx")).toBe(true);
});
