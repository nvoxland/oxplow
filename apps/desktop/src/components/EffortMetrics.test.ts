import { expect, test } from "bun:test";

import type { EffortMetricDelta } from "../api.js";
import {
  deltaColor,
  deltaSummary,
  fmtSigned,
} from "./EffortMetrics.js";

const d = (o: Partial<EffortMetricDelta>) => o as unknown as EffortMetricDelta;

test("deltaSummary shows before→after only when changed", () => {
  expect(
    deltaSummary(d({ agg: "files", changed: true, baseline: 2, current: 0 })),
  ).toBe("2 → 0");
  expect(
    deltaSummary(d({ agg: "level", changed: false, baseline: 5, current: 5 })),
  ).toBe("5");
  // A flow (sum) metric shows the signed total, never before→after.
  // Compact form comes from the shared locale-aware formatter (tsk114), so
  // the expectation is computed through the same Intl config.
  const compact = new Intl.NumberFormat(undefined, {
    notation: "compact",
    maximumFractionDigits: 1,
  });
  expect(deltaSummary(d({ agg: "sum", current: 48230 }))).toBe(`+${compact.format(48230)}`);
  // `%` unit glues to the number.
  expect(
    deltaSummary(d({ agg: "level", changed: true, baseline: 72, current: 81, unit: "%" })),
  ).toBe("72% → 81%");
});

test("deltaColor reflects whether the move improved the metric", () => {
  // lower-better: a drop is an improvement (green), a rise a regression (red).
  expect(deltaColor(d({ direction: "lower-better", delta: -2 }))).toContain("success");
  expect(deltaColor(d({ direction: "lower-better", delta: 3 }))).toContain("danger");
  // higher-better flips it.
  expect(deltaColor(d({ direction: "higher-better", delta: 5 }))).toContain("success");
  // neutral / no delta → muted.
  expect(deltaColor(d({ direction: "neutral", delta: 3 }))).toContain("muted");
  expect(deltaColor(d({ direction: "lower-better", delta: null }))).toContain("muted");
});

test("fmtSigned signs and compacts via the shared formatter", () => {
  expect(fmtSigned(3)).toBe("+3");
  expect(fmtSigned(-2)).toBe("-2");
  const compact = new Intl.NumberFormat(undefined, {
    notation: "compact",
    maximumFractionDigits: 1,
  });
  expect(fmtSigned(48230)).toBe(`+${compact.format(48230)}`);
});
