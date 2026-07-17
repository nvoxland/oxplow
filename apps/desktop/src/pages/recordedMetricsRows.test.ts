import { describe, expect, it } from "bun:test";

import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  filterMetricRows,
  metricSiblings,
} from "./recordedMetricsRows.js";

const row = (key: string, enabled: boolean, title = key) => ({ key, title, enabled });

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

  it("offers exactly the two documented modes", () => {
    expect(SHOW_MODES.map((m) => m.key)).toEqual(["enabled", "all"]);
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
