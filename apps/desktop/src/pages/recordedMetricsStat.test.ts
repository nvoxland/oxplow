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
});
