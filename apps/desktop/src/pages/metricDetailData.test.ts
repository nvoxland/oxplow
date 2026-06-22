import { expect, test } from "bun:test";

import type { MetricFinding, MetricSample } from "../api.js";
import {
  deltaVsFirst,
  findingRows,
  parseDetailPayload,
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
