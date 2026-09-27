import { expect, test } from "bun:test";
import { redirectLegacyRef } from "./legacyRedirects.js";

const table = { usage: "oxplow-analytics/usage", "dashboard:quality": "oxplow-analytics/quality", dashboard: "oxplow-analytics/planning" };

test("a removed page kind or exact id opens its lens instead", () => {
  expect(redirectLegacyRef({ id: "usage", kind: "usage" as never, payload: null }, table).id).toBe("lens:oxplow-analytics/usage");
  expect(redirectLegacyRef({ id: "dashboard:quality", kind: "dashboard", payload: null }, table).id).toBe(
    "lens:oxplow-analytics/quality",
  );
  expect(redirectLegacyRef({ id: "dashboard:review", kind: "dashboard", payload: null }, table).id).toBe(
    "lens:oxplow-analytics/planning",
  );
});

test("everything else passes through untouched", () => {
  const ref = { id: "task:tsk1", kind: "task" as const, payload: { itemId: "tsk1" } };
  expect(redirectLegacyRef(ref, table)).toBe(ref);
});

test("a redirect with a param carries the old id's row id into the lens", () => {
  const t = { "effort-coverage": { lens: "oxplow-analytics/effort-tests", param: "effort_id" } };
  expect(redirectLegacyRef({ id: "effort-coverage:eff12", kind: "effort-coverage" as never, payload: null }, t).id).toBe(
    "lens:oxplow-analytics/effort-tests?effort_id=12",
  );
});
