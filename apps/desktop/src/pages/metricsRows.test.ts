import { describe, expect, it } from "bun:test";

import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  filterMetricRows,
  isOffTarget,
  metricStatus,
  metricSiblings,
} from "./metricsRows.js";

const row = (key: string, enabled: boolean, title = key) => ({ key, title, enabled });

const spec = (
  over: Partial<{
    direction: string;
    target: number | null;
    warn_at: number | null;
    fail_at: number | null;
  }> = {},
) => ({ direction: "higher-better", target: null, warn_at: null, fail_at: null, ...over });

const ROWS = [
  row("oxplow.rust.unsafe_blocks", true, "unsafe blocks"),
  row("oxplow.csharp.empty_catch", false, "empty catch blocks"),
  row("oxplow.clojure.defn_count", false, "defn count"),
  row("oxplow.tests.failed", true, "Failed tests"),
];

describe("filterMetricRows", () => {
  it("defaults to hiding disabled metrics", () => {
    // The C#/Clojure idiom gauges are seeded as specs but never `use:`d, so
    // they're catalog-disabled and their gauge never runs — listing them makes
    // the page look like it tracks things it doesn't.
    expect(DEFAULT_SHOW_MODE).toBe("enabled");
    expect(filterMetricRows(ROWS, "enabled", "").map((r) => r.key)).toEqual([
      "oxplow.rust.unsafe_blocks",
      "oxplow.tests.failed",
    ]);
  });

  it("shows every metric under `all`", () => {
    expect(filterMetricRows(ROWS, "all", "")).toHaveLength(4);
  });

  it("off-target mode keeps only enabled rows, like `enabled` (the value test is the page's job)", () => {
    // filterMetricRows can't see values; it narrows to the enabled set + query,
    // and MetricsPage then drops the on-target rows via isOffTarget.
    expect(filterMetricRows(ROWS, "off-target", "").map((r) => r.key)).toEqual([
      "oxplow.rust.unsafe_blocks",
      "oxplow.tests.failed",
    ]);
  });

  it("lists Off target as a Show option", () => {
    expect(SHOW_MODES.map((m) => m.key)).toContain("off-target");
  });

  it("matches the query against both title and key, case-insensitively", () => {
    expect(filterMetricRows(ROWS, "all", "EMPTY CATCH").map((r) => r.key)).toEqual([
      "oxplow.csharp.empty_catch",
    ]);
    expect(filterMetricRows(ROWS, "all", "clojure").map((r) => r.key)).toEqual([
      "oxplow.clojure.defn_count",
    ]);
  });

  it("applies the enabled filter and the query together", () => {
    // Searching must not resurrect a disabled metric while showing enabled only.
    expect(filterMetricRows(ROWS, "enabled", "catch")).toEqual([]);
    expect(filterMetricRows(ROWS, "all", "catch").map((r) => r.key)).toEqual([
      "oxplow.csharp.empty_catch",
    ]);
  });

  it("ignores surrounding whitespace in the query", () => {
    expect(filterMetricRows(ROWS, "all", "   ").map((r) => r.key)).toHaveLength(4);
    expect(filterMetricRows(ROWS, "all", "  defn  ").map((r) => r.key)).toEqual([
      "oxplow.clojure.defn_count",
    ]);
  });

  it("keeps incoming order", () => {
    expect(filterMetricRows(ROWS, "all", "count").map((r) => r.key)).toEqual([
      "oxplow.clojure.defn_count",
    ]);
    expect(filterMetricRows([...ROWS].reverse(), "enabled", "").map((r) => r.key)).toEqual([
      "oxplow.tests.failed",
      "oxplow.rust.unsafe_blocks",
    ]);
  });

  it("offers exactly the three documented modes", () => {
    expect(SHOW_MODES.map((m) => m.key)).toEqual(["enabled", "all", "off-target"]);
  });
});

describe("metricSiblings", () => {
  const section = (keys: string[]) => ({
    entries: keys.map((key) => ({ key, title: key.toUpperCase() })),
  });

  it("flattens sections in render order so up/down walks the visible list (tsk119)", () => {
    // The sibling chain must continue ACROSS section boundaries — stepping
    // down from the last Tests row lands on the first TypeScript row, exactly
    // as the eye reads the page.
    const sibs = metricSiblings(
      [section(["a", "b"]), section(["c"])],
      (key) => ({ id: `metric-detail:${key}`, kind: "metric-detail", payload: null }),
    );
    expect(sibs.entries.map((e) => e.ref.id)).toEqual([
      "metric-detail:a",
      "metric-detail:b",
      "metric-detail:c",
    ]);
    // Labels are the row titles — they feed the prev/next hover tooltips.
    expect(sibs.entries.map((e) => e.label)).toEqual(["A", "B", "C"]);
    expect(sibs.indexByKey.get("c")).toBe(2);
  });

  it("returns an empty chain for no sections", () => {
    expect(metricSiblings([], () => ({ id: "x", kind: "metric-detail", payload: null })).entries).toEqual([]);
  });
});

describe("metricStatus (tsk121)", () => {
  it("is ok when a higher-better value meets its target", () => {
    expect(metricStatus(spec({ target: 80 }), 85)).toBe("ok");
    expect(metricStatus(spec({ target: 80 }), 80)).toBe("ok");
  });
  it("warns when a target is set but unmet and no fail breach", () => {
    expect(metricStatus(spec({ target: 80 }), 72)).toBe("warn");
    expect(metricStatus(spec({ target: 80, fail_at: 50 }), 72)).toBe("warn");
  });
  it("fails when the value is past fail_at", () => {
    expect(metricStatus(spec({ target: 80, fail_at: 50 }), 40)).toBe("fail");
  });
  it("honors lower-better direction", () => {
    expect(metricStatus(spec({ direction: "lower-better", target: 10 }), 8)).toBe("ok");
    expect(metricStatus(spec({ direction: "lower-better", target: 10 }), 14)).toBe("warn");
  });
  it("is none with no thresholds or a neutral direction (nothing to be off of)", () => {
    expect(metricStatus(spec({}), 5)).toBe("none");
    expect(metricStatus(spec({ direction: "neutral", target: 80 }), 5)).toBe("none");
  });
});

describe("isOffTarget (tsk121)", () => {
  it("is true for warn or fail, false for ok / none / null def", () => {
    expect(isOffTarget(spec({ target: 80 }), 72)).toBe(true); // warn
    expect(isOffTarget(spec({ target: 80, fail_at: 50 }), 40)).toBe(true); // fail
    expect(isOffTarget(spec({ target: 80 }), 85)).toBe(false); // ok
    expect(isOffTarget(spec({}), 5)).toBe(false); // no threshold
    expect(isOffTarget(null, 5)).toBe(false); // disabled/pruned spec
  });
});
