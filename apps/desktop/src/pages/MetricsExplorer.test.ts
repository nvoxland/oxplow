import { expect, test } from "bun:test";
import type { MetricDefinition, MetricSample } from "../api.js";
import {
  buildExplorerSeries,
  buildScatterPoints,
  dimsValue,
} from "./MetricsExplorer.js";

const s = (o: Partial<MetricSample>) => o as unknown as MetricSample;
const d = (o: Partial<MetricDefinition>) => o as unknown as MetricDefinition;

test("dimsValue reads branch, subject, and dims_json keys", () => {
  expect(dimsValue(s({ branch: "main" }), "branch")).toBe("main");
  expect(dimsValue(s({ subject_ref: "model:opus" }), "subject")).toBe("model:opus");
  expect(dimsValue(s({ dims_json: '{"language":"rust"}' }), "language")).toBe("rust");
  // entity-style fallback to subject_ref for a model dim.
  expect(dimsValue(s({ subject_kind: "model", subject_ref: "opus" }), "model")).toBe("opus");
  expect(dimsValue(s({}), "language")).toBeNull();
  expect(dimsValue(s({ dims_json: "not json" }), "language")).toBeNull();
});

test("groups multiple measures by a shared dimension (drill-across)", () => {
  const defs = [d({ key: "loc", title: "LOC" }), d({ key: "cx", title: "Complexity" })];
  const samplesByKey = {
    loc: [
      s({ value: 100, captured_at: "2026-06-22T00:00:00Z", dims_json: '{"language":"rust"}' }),
      s({ value: 200, captured_at: "2026-06-22T00:00:00Z", dims_json: '{"language":"typescript"}' }),
    ],
    cx: [s({ value: 5, captured_at: "2026-06-22T00:00:00Z", dims_json: '{"language":"rust"}' })],
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
    // newest-first per measure
    cov: [
      s({ value: 80, subject_ref: "module:a", captured_at: "2026-06-03T00:00:00Z" }),
      s({ value: 50, subject_ref: "module:a", captured_at: "2026-06-01T00:00:00Z" }),
      s({ value: 60, subject_ref: "module:b", captured_at: "2026-06-02T00:00:00Z" }),
    ],
    cx: [
      s({ value: 3, subject_ref: "module:a", captured_at: "2026-06-03T00:00:00Z" }),
      s({ value: 9, subject_ref: "module:b", captured_at: "2026-06-02T00:00:00Z" }),
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

test("a sample missing the group dimension is dropped from grouped series", () => {
  const defs = [d({ key: "loc", title: "LOC" })];
  const series = buildExplorerSeries(
    ["loc"],
    {
      loc: [
        s({ value: 1, captured_at: "2026-06-22T00:00:00Z", dims_json: '{"language":"rust"}' }),
        s({ value: 9, captured_at: "2026-06-22T00:00:00Z" }), // no language → dropped
      ],
    },
    "language",
    defs,
  );
  expect(series).toHaveLength(1);
  expect(series[0]!.points).toHaveLength(1);
});
