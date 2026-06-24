import { expect, test } from "bun:test";

import type { EffortMetricDelta } from "../api.js";
import {
  deltaColor,
  deltaSummary,
  fmtSigned,
  hasDedicatedPanel,
  metricGroup,
} from "./EffortMetrics.js";

const d = (o: Partial<EffortMetricDelta>) => o as unknown as EffortMetricDelta;

test("hasDedicatedPanel hides metrics shown in a dedicated panel", () => {
  // Covered by Coverage / Tests / Static analysis / Token / Nudge panels.
  expect(hasDedicatedPanel(d({ category: "testing", key: "oxplow.tests.total" }))).toBe(true);
  expect(hasDedicatedPanel(d({ category: "coverage", key: "oxplow.coverage.abs_pct" }))).toBe(true);
  expect(hasDedicatedPanel(d({ category: "static-quality", key: "oxplow.analysis.errors" }))).toBe(true);
  expect(hasDedicatedPanel(d({ category: "operational", key: "agent.tokens.total" }))).toBe(true);
  expect(hasDedicatedPanel(d({ category: "operational", key: "agent.nudges.fired" }))).toBe(true);
  // Code-health gauges + panel-less operational metrics stay in the block.
  expect(hasDedicatedPanel(d({ category: null, key: "oxplow.rust.unsafe_blocks" }))).toBe(false);
  expect(hasDedicatedPanel(d({ category: "operational", key: "effort.cycle_time_ms" }))).toBe(false);
});

test("metricGroup buckets code-health gauges by language, producers by category", () => {
  expect(metricGroup(d({ category: null, language: "rust" })).label).toBe(
    "Rust code health",
  );
  expect(metricGroup(d({ category: null, language: "typescript" })).label).toBe(
    "Typescript code health",
  );
  expect(metricGroup(d({ category: "coverage" })).label).toBe("Coverage");
  expect(metricGroup(d({ category: "testing" })).order).toBe(2);
  expect(metricGroup(d({ category: "operational" })).order).toBe(4);
  // Code-health gauges sort before producer categories.
  expect(metricGroup(d({ category: null, language: "rust" })).order).toBe(0);
});

test("deltaSummary shows before→after only when changed", () => {
  expect(
    deltaSummary(d({ agg: "files", changed: true, baseline: 2, current: 0 })),
  ).toBe("2 → 0");
  expect(
    deltaSummary(d({ agg: "level", changed: false, baseline: 5, current: 5 })),
  ).toBe("5");
  // A flow (sum) metric shows the signed total, never before→after.
  expect(deltaSummary(d({ agg: "sum", current: 48230 }))).toBe("+48.2k");
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

test("fmtSigned signs and k-compresses", () => {
  expect(fmtSigned(3)).toBe("+3");
  expect(fmtSigned(-2)).toBe("-2");
  expect(fmtSigned(48230)).toBe("+48.2k");
});
