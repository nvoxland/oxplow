import { metricUri } from "../uri/uri.js";
import { describe, expect, it } from "bun:test";

import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  filterMetricRows,
  metricStatus,
  metricOrigin,
  sliceGroups,
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

describe("metricOrigin", () => {
  const section = (keys: string[]) => ({
    entries: keys.map((key) => ({ key, title: key.toUpperCase() })),
  });

  it("flattens sections in render order so up/down walks the visible list (tsk119)", () => {
    // The sibling chain must continue ACROSS section boundaries — stepping
    // down from the last Tests row lands on the first TypeScript row, exactly
    // as the eye reads the page.
    const sibs = metricOrigin(
      [section(["a", "b"]), section(["c"])],
      (key) => ({ id: metricUri(key), kind: "metric", payload: null }),
    );
    expect(sibs.items.map((e) => e.ref.id)).toEqual([
      "oxplow:oxplow/metric?metric=a",
      "oxplow:oxplow/metric?metric=b",
      "oxplow:oxplow/metric?metric=c",
    ]);
    // Labels are the row titles — they feed the prev/next hover tooltips.
    expect(sibs.items.map((e) => e.label)).toEqual(["A", "B", "C"]);
    expect(sibs.indexByKey.get("c")).toBe(2);
  });

  it("returns an empty chain for no sections", () => {
    expect(metricOrigin([], () => ({ id: "x", kind: "metric", payload: null })).items).toEqual([]);
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
  // The server's `threshold_state` draws the same line: a value AT a
  // threshold hasn't crossed it.
  it("reads a value at a threshold as not past it", () => {
    expect(metricStatus(spec({ warn_at: 80, fail_at: 50 }), 80)).toBe("ok");
    expect(metricStatus(spec({ warn_at: 80, fail_at: 50 }), 50)).toBe("warn");
    expect(metricStatus(spec({ direction: "lower-better", warn_at: 5, fail_at: 10 }), 10)).toBe("warn");
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


describe("sliceGroups", () => {
  // Samples come newest first, as the Metrics list reads them.
  const point = (group: string | null, value: number, at: number) =>
    ({ group, value, captured_at: `2026-10-0${at}T00:00:00Z` }) as unknown as import("../api.js").SeriesPoint;

  it("makes one series per group, ranked by its latest value, newest point first", () => {
    const samples = [
      point("rust", 5, 3),
      point("ts", 9, 3),
      point("rust", 4, 2),
      point("ts", 8, 2),
      point("py", 20, 1),
    ];
    const { shown, hidden } = sliceGroups(samples, 5);
    expect(shown.map((g) => [g.group, g.latest.value, g.samples.length])).toEqual([
      ["py", 20, 1],
      ["ts", 9, 2],
      ["rust", 5, 2],
    ]);
    expect(hidden).toBe(0);
  });

  it("shows the top N and counts the rest", () => {
    const samples = ["a", "b", "c", "d", "e"].map((g, i) => point(g, i + 1, 1));
    const { shown, hidden } = sliceGroups(samples, 2);
    expect(shown.map((g) => g.group)).toEqual(["e", "d"]);
    expect(hidden).toBe(3);
  });

  it("ties break by name, and a point with no group isn't a group", () => {
    const { shown } = sliceGroups([point("b", 1, 1), point(null, 99, 1), point("a", 1, 1)], 5);
    expect(shown.map((g) => g.group)).toEqual(["a", "b"]);
    expect(sliceGroups([], 3)).toEqual({ shown: [], hidden: 0 });
  });
});
