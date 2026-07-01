import { expect, test } from "bun:test";
import type { MetricSpec, SeriesPoint } from "../api.js";
import { buildExplorerSeries, buildScatterPoints } from "./MetricsExplorer.js";

const s = (o: Partial<SeriesPoint>) => o as unknown as SeriesPoint;
const d = (o: Partial<MetricSpec>) => o as unknown as MetricSpec;

test("groups multiple measures by a shared dimension (drill-across)", () => {
  // The server slices by the group-by dimension and tags each point's `group`.
  const defs = [d({ key: "loc", title: "LOC" }), d({ key: "cx", title: "Complexity" })];
  const samplesByKey = {
    loc: [
      s({ value: 100, captured_at: "2026-06-22T00:00:00Z", group: "rust" }),
      s({ value: 200, captured_at: "2026-06-22T00:00:00Z", group: "typescript" }),
    ],
    cx: [s({ value: 5, captured_at: "2026-06-22T00:00:00Z", group: "rust" })],
  };
  const series = buildExplorerSeries(["loc", "cx"], samplesByKey, "language", defs);
  expect(series.map((x) => x.label).sort()).toEqual(
    ["Complexity · rust", "LOC · rust", "LOC · typescript"].sort(),
  );
  // distinct colors assigned per series.
  expect(new Set(series.map((x) => x.color)).size).toBe(series.length);
});

test("group-by none → one series per measure, points sorted-able by time", () => {
  const defs = [d({ key: "loc", title: "LOC" })];
  const series = buildExplorerSeries(
    ["loc"],
    {
      loc: [
        s({ value: 1, captured_at: "2026-06-22T00:00:00Z" }),
        s({ value: 2, captured_at: "2026-06-22T01:00:00Z" }),
      ],
    },
    "none",
    defs,
  );
  expect(series).toHaveLength(1);
  expect(series[0]!.label).toBe("LOC");
  expect(series[0]!.points).toHaveLength(2);
});

test("buildScatterPoints pairs two measures by shared group, latest per group", () => {
  const samplesByKey = {
    // newest-first per measure; the server tagged each point's `group`.
    cov: [
      s({ value: 80, group: "module:a", captured_at: "2026-06-03T00:00:00Z" }),
      s({ value: 50, group: "module:a", captured_at: "2026-06-01T00:00:00Z" }),
      s({ value: 60, group: "module:b", captured_at: "2026-06-02T00:00:00Z" }),
    ],
    cx: [
      s({ value: 3, group: "module:a", captured_at: "2026-06-03T00:00:00Z" }),
      s({ value: 9, group: "module:b", captured_at: "2026-06-02T00:00:00Z" }),
    ],
  };
  const pts = buildScatterPoints(["cov", "cx"], samplesByKey, "subject");
  expect(pts).toEqual([
    { label: "module:a", x: 80, y: 3 },
    { label: "module:b", x: 60, y: 9 },
  ]);
});

test("buildScatterPoints is empty without exactly two measures + a group-by", () => {
  expect(buildScatterPoints(["cov"], {}, "subject")).toEqual([]);
  expect(buildScatterPoints(["cov", "cx"], {}, "none")).toEqual([]);
});

test("a point missing the group dimension is dropped from grouped series", () => {
  const defs = [d({ key: "loc", title: "LOC" })];
  const series = buildExplorerSeries(
    ["loc"],
    {
      loc: [
        s({ value: 1, captured_at: "2026-06-22T00:00:00Z", group: "rust" }),
        s({ value: 9, captured_at: "2026-06-22T00:00:00Z" }), // no group → dropped
      ],
    },
    "language",
    defs,
  );
  expect(series).toHaveLength(1);
  expect(series[0]!.points).toHaveLength(1);
});
