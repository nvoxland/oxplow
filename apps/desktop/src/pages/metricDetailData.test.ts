import { expect, test } from "bun:test";

import type { MetricFinding, MetricSample } from "../api.js";
import {
  branchOptions,
  defaultChartMode,
  deltaVsFirst,
  filterByRange,
  findingRows,
  filterByBranch,
  fromLocalInput,
  inRangeStat,
  transformSeries,
  matchPresetKey,
  parseDetailPayload,
  rangeFromPreset,
  seriesPoints,
  topSubjects,
} from "./metricDetailData.js";

const s = (o: Partial<MetricSample>) => o as unknown as MetricSample;
const f = (o: Partial<MetricFinding>) => o as unknown as MetricFinding;

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

test("topSubjects sums by subject and ranks", () => {
  const top = topSubjects(
    [
      s({ subject_ref: "module:a", value: 3 }),
      s({ subject_ref: "module:b", value: 10 }),
      s({ subject_ref: "module:a", value: 4 }),
      s({ value: 1 }),
    ],
    2,
  );
  expect(top).toEqual([
    { subject: "module:b", value: 10 },
    { subject: "module:a", value: 7 },
  ]);
});

test("findingRows excludes verbatim *-detail payloads", () => {
  const rows = findingRows([
    f({ kind: "lint", rule: "no-foo" }),
    f({ kind: "analysis-detail" }),
    f({ kind: "complexity" }),
  ]);
  expect(rows.map((r) => r.kind)).toEqual(["lint", "complexity"]);
});

test("parseDetailPayload finds and parses by detail kind", () => {
  const findings = [
    f({ kind: "coverage-detail", extra_json: '{"summaryPct":71}' }),
    f({ kind: "lint" }),
  ];
  expect(parseDetailPayload(findings, "coverage-detail")).toEqual({ summaryPct: 71 });
  expect(parseDetailPayload(findings, "test-detail")).toBeNull();
  expect(
    parseDetailPayload([f({ kind: "coverage-detail", extra_json: "bad" })], "coverage-detail"),
  ).toBeNull();
});

test("rangeFromPreset builds a [now-span, now] window", () => {
  const now = Date.parse("2026-06-25T12:00:00Z");
  const r = rangeFromPreset("7d", now);
  expect(r.to).toBe(now);
  expect(r.from).toBe(now - 7 * 24 * 60 * 60 * 1000);
  // Unknown key falls back to the 7d preset.
  expect(rangeFromPreset("nope", now)).toEqual(r);
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
      s({ id: 1, captured_at: "2026-06-05T00:00:00Z", value: 1 }),
      s({ id: 2, captured_at: "2026-06-15T00:00:00Z", value: 2 }),
      s({ id: 3, captured_at: "2026-06-20T00:00:00Z", value: 3 }),
      s({ id: 4, captured_at: "nonsense", value: 4 }),
    ],
    range,
  );
  expect(kept.map((k) => k.id)).toEqual([2, 3]);
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
    s({ id: 1, branch: "main" }),
    s({ id: 2, branch: "feat" }),
    s({ id: 3, branch: null }),
    s({ id: 4, branch: "main" }),
  ];
  expect(branchOptions(rows)).toEqual(["feat", "main"]);
  expect(filterByBranch(rows, "main").map((r) => r.id)).toEqual([1, 4]);
  expect(filterByBranch(rows, null).length).toBe(4);
});

test("inRangeStat respects default_agg", () => {
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

test("defaultChartMode follows default_agg", () => {
  expect(defaultChartMode("sum")).toBe("cumulative");
  expect(defaultChartMode("avg")).toBe("avg");
  expect(defaultChartMode("last")).toBe("value");
  expect(defaultChartMode("whatever")).toBe("value");
});
