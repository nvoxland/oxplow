import { describe, expect, test } from "bun:test";

import { DEFAULT_LINE_STAT, LINE_STATS, lineStatValue } from "./recordedMetricsStat";

// Samples are newest-first, as RecordedMetricsPage holds them.
const s = (...values: number[]) => values.map((value) => ({ value }));

describe("lineStatValue", () => {
  test("latest is the newest sample — today's default behavior", () => {
    expect(lineStatValue(s(7, 3, 1), "latest")).toBe(7);
    expect(DEFAULT_LINE_STAT).toBe("latest");
  });
  test("change is signed newest − oldest over the plotted window", () => {
    expect(lineStatValue(s(7, 3, 1), "change")).toBe(6);
    expect(lineStatValue(s(1, 5), "change")).toBe(-4);
  });
  test("change needs two points — a single sample has no change", () => {
    expect(lineStatValue(s(7), "change")).toBeNull();
  });
  test("mean, min, max describe the window", () => {
    expect(lineStatValue(s(6, 2, 4), "mean")).toBe(4);
    expect(lineStatValue(s(6, 2, 4), "min")).toBe(2);
    expect(lineStatValue(s(6, 2, 4), "max")).toBe(6);
  });
  test("no samples ⇒ null for every stat", () => {
    for (const { key } of LINE_STATS) {
      expect(lineStatValue([], key)).toBeNull();
    }
  });

  test("distance to target is the latest plotted value minus target, signed (tsk120)", () => {
    // Samples are newest-first, so the target stats read samples[0].
    expect(lineStatValue(s(72, 70, 68), "distance", 80)).toBe(-8);
    expect(lineStatValue(s(85, 82), "distance", 80)).toBe(5);
  });
  test("distance is null when the metric has no target", () => {
    expect(lineStatValue(s(72), "distance", null)).toBeNull();
    expect(lineStatValue(s(72), "distance")).toBeNull();
  });
  test("percent of target is the latest value over target, ×100", () => {
    expect(lineStatValue(s(72), "pctTarget", 80)).toBe(90);
    expect(lineStatValue(s(120), "pctTarget", 80)).toBe(150);
  });
  test("percent of target is null without a target, or a zero target", () => {
    expect(lineStatValue(s(72), "pctTarget", null)).toBeNull();
    expect(lineStatValue(s(72), "pctTarget", 0)).toBeNull();
  });
  test("both target stats are listed as line-value options", () => {
    const keys = LINE_STATS.map((o) => o.key);
    expect(keys).toContain("distance");
    expect(keys).toContain("pctTarget");
  });
});
