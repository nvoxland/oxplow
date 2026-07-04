import { expect, test } from "bun:test";

import { planRepaint, REPAINT_MIN_INTERVAL_MS } from "./terminalRepaintSchedule.js";

test("runs immediately when the last repaint is older than the min interval", () => {
  const plan = planRepaint(0, REPAINT_MIN_INTERVAL_MS + 5, REPAINT_MIN_INTERVAL_MS);
  expect(plan).toEqual({ run: "now" });
});

test("runs immediately exactly at the interval boundary", () => {
  const plan = planRepaint(1000, 1000 + REPAINT_MIN_INTERVAL_MS, REPAINT_MIN_INTERVAL_MS);
  expect(plan).toEqual({ run: "now" });
});

test("defers by the remaining interval when a repaint just ran", () => {
  // Last ran at t=1000; a write arrives 100ms into a 250ms interval.
  const plan = planRepaint(1000, 1100, 250);
  expect(plan).toEqual({ run: "defer", waitMs: 150 });
});

test("defers by the full interval when two writes land back-to-back", () => {
  const plan = planRepaint(1000, 1000, 250);
  expect(plan).toEqual({ run: "defer", waitMs: 250 });
});
