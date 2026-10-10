import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { MetricCatalogEntry, MetricSpec, SeriesPoint } from "../api.js";
import { DIMENSIONS_SQL } from "./exploreData.js";

// Slice By on the Metrics list (tsk594): pick a dimension and each metric
// that has it lists its top groups beneath it, each with its own latest value
// and trend; the rest is counted.

const realApi = await import("../api.js");
// Taken now: after `mock.module`, the namespace is the mock.
const realQuerySql = realApi.querySql as (...a: unknown[]) => Promise<unknown>;
const noReads = { models: [], tables: [], measures: [] };
const day = new Date().toISOString();

const entry = (key: string, title: string): MetricCatalogEntry =>
  ({ key, title, category: "Code metrics", language: null, enabled: true }) as unknown as MetricCatalogEntry;
const spec = (key: string): MetricSpec =>
  ({
    key,
    title: key,
    unit: null,
    source_measure: "acme.m",
    aggregation: "sum",
    direction: "neutral",
    target: null,
    warn_at: null,
    fail_at: null,
    description: null,
    category: "Code metrics",
    language: null,
    scope: "built-in",
  }) as unknown as MetricSpec;
const point = (value: number, group: string | null): SeriesPoint =>
  ({ capture_id: 1, captured_at: day, value, numerator: null, denominator: null, group, branch: "main" }) as unknown as SeriesPoint;

const sampleCalls: (string | null)[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listMetricCatalog: async () => ({ rows: [entry("acme.todos", "TODOs"), entry("acme.flat", "Flat")], reads: noReads }),
  listMetricDefinitions: async () => ({ rows: [spec("acme.todos"), spec("acme.flat")], reads: noReads }),
  listMetricSamples: async (key: string, _limit?: number, groupBy?: string | null) => {
    sampleCalls.push(groupBy ?? null);
    if (!groupBy) return { rows: [point(30, null)], reads: noReads };
    // `acme.flat` has no such dimension: nothing is placed on a group.
    if (key === "acme.flat") return { rows: [], reads: noReads };
    return {
      rows: ["rust", "ts", "py", "go", "c"].map((g, i) => point((i + 1) * 5, g)),
      reads: noReads,
    };
  },
  // Bun's `mock.module` is process-wide: only the dimensions read is faked.
  querySql: (sql: string, ...rest: unknown[]) =>
    sql === DIMENSIONS_SQL
      ? Promise.resolve({
          columns: ["key", "label"],
          rows: [["oxplow.language", "Language"]],
          truncated: false,
          reads: noReads,
          freshness: [],
        })
      : realQuerySql(sql, ...rest),
  subscribeOxplowEvents: () => () => {},
}));
const { MetricsPage } = await import("./MetricsPage.js");

afterEach(cleanup);

test("slicing by a dimension lists each metric's top groups beneath it", async () => {
  const view = render(<MetricsPage />);
  await waitFor(() => expect(view.getByText("TODOs")).toBeTruthy());
  expect(view.queryAllByTestId("recorded-slice-group")).toHaveLength(0);

  const select = (await view.findByTestId("recorded-slice-by")) as HTMLSelectElement;
  await waitFor(() => expect([...select.options].map((o) => o.value)).toContain("oxplow.language"));
  fireEvent.change(select, { target: { value: "oxplow.language" } });

  await waitFor(() => expect(view.queryAllByTestId("recorded-slice-group").length).toBeGreaterThan(0));
  const groups = view.getAllByTestId("recorded-slice-group").map((g) => g.textContent ?? "");
  // The top three by latest value (largest first), for the one metric that has them.
  expect(groups).toHaveLength(3);
  expect(groups[0]).toContain("c");
  expect(groups[0]).toContain("25");
  expect(groups[1]).toContain("go");
  expect(groups[2]).toContain("py");
  // The other two are counted, not listed.
  expect(view.getByTestId("recorded-slice-more").textContent).toMatch(/2 more/);
  // The metric with nothing on that dimension says so, and keeps its row.
  expect(view.getByTestId("recorded-slice-none").textContent).toMatch(/Language/);
  expect(view.getByText("Flat")).toBeTruthy();
  expect(sampleCalls).toContain("oxplow.language");

  // Back to no slice: the groups go.
  fireEvent.change(select, { target: { value: "" } });
  await waitFor(() => expect(view.queryAllByTestId("recorded-slice-group")).toHaveLength(0));
});
