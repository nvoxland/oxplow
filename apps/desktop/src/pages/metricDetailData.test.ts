import { expect, test } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  branchOptions,
  breakdownDimensions,
  defaultChartMode,
  deltaVsFirst,
  filterByRange,
  filterByBranch,
  fromLocalInput,
  inRangeStat,
  transformSeries,
  matchPresetKey,
  rangeFromPreset,
  seriesPoints,
  widestPresetWindow,
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

test("rangeFromPreset builds a [now-span, now] window", () => {
  const now = Date.parse("2026-06-25T12:00:00Z");
  const r = rangeFromPreset("7d", now);
  expect(r.to).toBe(now);
  expect(r.from).toBe(now - 7 * 24 * 60 * 60 * 1000);
  // Unknown key falls back to the 7d preset.
  expect(rangeFromPreset("nope", now)).toEqual(r);
});

test("widestPresetWindow spans the largest preset and contains every preset", () => {
  const now = Date.parse("2026-06-25T12:00:00Z");
  const widest = widestPresetWindow(now);
  expect(widest.to).toBe(now);
  // 30d is the widest preset today.
  expect(widest.from).toBe(now - 30 * 24 * 60 * 60 * 1000);
  // Every preset the UI can select is a subset — so a client-side filterByRange
  // within the fetched widest window never needs a re-fetch (tsk202).
  for (const key of ["1d", "2d", "3d", "7d", "30d"]) {
    const r = rangeFromPreset(key, now);
    expect(r.from).toBeGreaterThanOrEqual(widest.from);
    expect(r.to).toBeLessThanOrEqual(widest.to);
  }
});

test("matchPresetKey recognizes presets and flags custom", () => {
  const now = Date.parse("2026-06-25T12:00:00Z");
  expect(matchPresetKey(rangeFromPreset("3d", now), now)).toBe("3d");
  // `to` not near now → custom.
  expect(matchPresetKey({ from: now - 99, to: now - 10 * 60 * 1000 }, now)).toBe("custom");
  // odd span → custom.
  expect(matchPresetKey({ from: now - 99_999_999, to: now }, now)).toBe("custom");
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

test("fromLocalInput parses or returns null", () => {
  expect(fromLocalInput("")).toBeNull();
  expect(fromLocalInput("not-a-date")).toBeNull();
  expect(fromLocalInput("2026-06-25T12:00")).toBe(new Date("2026-06-25T12:00").getTime());
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

test("inRangeStat respects the spec aggregation", () => {
  const rows = [
    s({ captured_at: "2026-06-01T00:00:00Z", value: 10 }),
    s({ captured_at: "2026-06-02T00:00:00Z", value: 4 }),
    s({ captured_at: "2026-06-03T00:00:00Z", value: 6 }),
  ];
  expect(inRangeStat(rows, "sum")).toEqual({ label: "Total in range", value: 20, signed: false });
  expect(inRangeStat(rows, "avg")).toEqual({ label: "Avg in range", value: 20 / 3, signed: false });
  // last/level → signed last−first (6 − 10).
  expect(inRangeStat(rows, "last")).toEqual({ label: "Δ in range", value: -4, signed: true });
  // delta needs ≥2 points; sum/avg work with 1.
  expect(inRangeStat([rows[0]!], "last")).toBeNull();
  expect(inRangeStat([rows[0]!], "sum")).toEqual({ label: "Total in range", value: 10, signed: false });
  expect(inRangeStat([], "sum")).toBeNull();
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
const spec = (sliceable: string[] | null) =>
  ({
    key: "m.one",
    title: "One",
    sliceable_dims_json: sliceable === null ? null : JSON.stringify(sliceable),
  }) as never;

test("breakdown options come only from the spec", () => {
  expect(breakdownDimensions(spec(["model", "agent"]))).toEqual(["model", "agent"]);
  // A per-file metric declares package and gets it — as an ordinary dimension.
  expect(breakdownDimensions(spec(["package", "language"]))).toEqual(["package", "language"]);
});

test("a metric declaring nothing offers no breakdowns", () => {
  // Better an empty picker than one dead option: the old seed offered
  // `package` on token metrics, where it can only ever return nothing.
  expect(breakdownDimensions(spec(null))).toEqual([]);
  expect(breakdownDimensions(spec([]))).toEqual([]);
});

test("git_version and branch stay out of the breakdown picker", () => {
  // Both are spine dimensions with their own controls on the page.
  expect(breakdownDimensions(spec(["package", "git_version", "branch"]))).toEqual(["package"]);
});

test("duplicate declarations collapse", () => {
  expect(breakdownDimensions(spec(["package", "package", "model"]))).toEqual(["package", "model"]);
});
