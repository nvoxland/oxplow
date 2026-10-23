import { expect, test } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  branchOptions,
  defaultChartMode,
  deltaVsFirst,
  filterByRange,
  filterByBranch,
  transformSeries,
  seriesPoints,
  yDomain,
} from "./metricDetailData.js";

const s = (o: Partial<SeriesPoint>) => o as unknown as SeriesPoint;

test("seriesPoints sorts ascending and drops bad timestamps", () => {
  const pts = seriesPoints([
    s({ captured_at: "2026-06-02T00:00:00Z", value: 2 }),
    s({ captured_at: "nonsense", value: 9 }),
    s({ captured_at: "2026-06-01T00:00:00Z", value: 1 }),
  ]);
  expect(pts.map((p) => p.v)).toEqual([1, 2]);
});

test("deltaVsFirst is last-minus-first, null under two points", () => {
  expect(
    deltaVsFirst([
      s({ captured_at: "2026-06-01T00:00:00Z", value: 5 }),
      s({ captured_at: "2026-06-03T00:00:00Z", value: 12 }),
    ]),
  ).toBe(7);
  expect(deltaVsFirst([s({ captured_at: "2026-06-01T00:00:00Z", value: 5 })])).toBeNull();
});

test("filterByRange keeps inclusive in-window samples, drops bad timestamps", () => {
  const range = { from: Date.parse("2026-06-10T00:00:00Z"), to: Date.parse("2026-06-20T00:00:00Z") };
  const kept = filterByRange(
    [
      s({ capture_id: 1, captured_at: "2026-06-05T00:00:00Z", value: 1 }),
      s({ capture_id: 2, captured_at: "2026-06-15T00:00:00Z", value: 2 }),
      s({ capture_id: 3, captured_at: "2026-06-20T00:00:00Z", value: 3 }),
      s({ capture_id: 4, captured_at: "nonsense", value: 4 }),
    ],
    range,
  );
  expect(kept.map((k) => k.capture_id)).toEqual([2, 3]);
});

test("transformSeries: value/cumulative/change/avg", () => {
  const pts = [
    { t: 1, v: 2 },
    { t: 2, v: 4 },
    { t: 3, v: 6 },
  ];
  expect(transformSeries(pts, "value")).toEqual(pts);
  expect(transformSeries(pts, "cumulative")).toEqual([
    { t: 1, v: 2 },
    { t: 2, v: 6 },
    { t: 3, v: 12 },
  ]);
  expect(transformSeries(pts, "change")).toEqual([
    { t: 2, v: 2 },
    { t: 3, v: 2 },
  ]);
  // trailing avg: [2], [2,4]/2=3, [2,4,6]/3=4
  expect(transformSeries(pts, "avg")).toEqual([
    { t: 1, v: 2 },
    { t: 2, v: 3 },
    { t: 3, v: 4 },
  ]);
});

test("branchOptions + filterByBranch", () => {
  const rows = [
    s({ capture_id: 1, branch: "main" }),
    s({ capture_id: 2, branch: "feat" }),
    s({ capture_id: 3, branch: null }),
    s({ capture_id: 4, branch: "main" }),
  ];
  expect(branchOptions(rows)).toEqual(["feat", "main"]);
  expect(filterByBranch(rows, "main").map((r) => r.capture_id)).toEqual([1, 4]);
  expect(filterByBranch(rows, null).length).toBe(4);
});

test("defaultChartMode follows the spec aggregation", () => {
  expect(defaultChartMode("sum")).toBe("cumulative");
  expect(defaultChartMode("avg")).toBe("avg");
  expect(defaultChartMode("last")).toBe("value");
  expect(defaultChartMode("whatever")).toBe("value");
});

test("yDomain zero-scale anchors the axis at 0", () => {
  // The squished case: near-constant ~1.96 data on a 0-based axis (tsk133).
  expect(yDomain([1.95, 1.96, 1.96], null, "zero")).toEqual({ min: 0, max: 1.96 });
  // Negative data still spans through 0.
  expect(yDomain([-3, -1], null, "zero")).toEqual({ min: -3, max: 0 });
});

test("yDomain auto-scale fits the data with padding (not through 0)", () => {
  const d = yDomain([1.95, 1.96], null, "auto");
  // Tight window around the data, NOT anchored at 0 — this is what un-squishes it.
  expect(d.min).toBeGreaterThan(1.9);
  expect(d.min).toBeLessThan(1.95);
  expect(d.max).toBeGreaterThan(1.96);
  expect(d.max).toBeLessThan(2.0);
});

test("yDomain folds the target line into both bounds", () => {
  // A target above the data extends the top; below extends the bottom.
  expect(yDomain([10, 12], 20, "zero").max).toBe(20);
  expect(yDomain([10, 12], 5, "zero").min).toBe(0);
  expect(yDomain([10, 12], 20, "auto").max).toBeGreaterThanOrEqual(20);
});

test("yDomain auto pads a flat series so it isn't on an edge", () => {
  const d = yDomain([5, 5, 5], null, "auto");
  expect(d.min).toBeLessThan(5);
  expect(d.max).toBeGreaterThan(5);
});


// tsk179: breakdown options are whatever the SPEC declares — nothing more.
// `package` used to be seeded unconditionally, which meant a token metric
// offered a breakdown its facts can't answer (they carry no path, so every
// group is empty) while hiding `model` and `agent`, which they do carry.
// `package` is not special: `oxplow.package` is a registered dimension like
// `oxplow.model`, and `dim_value` resolves both.

