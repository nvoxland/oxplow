import { expect, test } from "bun:test";

import { edgeScrollSpeed } from "./dragAutoScroll.js";

const rect = { top: 100, bottom: 500 };

test("away from the edges a drag scrolls nothing", () => {
  expect(edgeScrollSpeed(300, rect)).toBe(0);
  expect(edgeScrollSpeed(100 + 48, rect)).toBe(0);
});

test("near the top it scrolls up, near the bottom down, faster closer to the edge", () => {
  expect(edgeScrollSpeed(130, rect)).toBeLessThan(0);
  expect(edgeScrollSpeed(470, rect)).toBeGreaterThan(0);
  expect(Math.abs(edgeScrollSpeed(102, rect))).toBeGreaterThan(Math.abs(edgeScrollSpeed(130, rect)));
  expect(edgeScrollSpeed(498, rect)).toBeGreaterThan(edgeScrollSpeed(470, rect));
});

test("past an edge (still dragging, above or below the rail) it scrolls at full speed", () => {
  expect(edgeScrollSpeed(40, rect)).toBe(edgeScrollSpeed(100, rect));
  expect(edgeScrollSpeed(560, rect)).toBe(edgeScrollSpeed(500, rect));
});
