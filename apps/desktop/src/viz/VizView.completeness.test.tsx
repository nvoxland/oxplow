import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { VizConfig, VizRun, SourceCoverage, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A result that is cut short, or fed by a collector that hasn't kept up,
// says so above its rows, whatever the viz.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

afterEach(cleanup);

const configOf = (viz: VizConfig["viz"]): VizConfig => ({
  id: "x/work",
  extension: "x",
  slug: "work",
  title: "Work",
  description: "",
  query: "",
  viz,
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
});

const failed: SourceCoverage = {
  model: "v_my_gh_pull_request",
  source: "oxplow:oxplow/collector?collector=my-gh/gh",
  status: "failed",
  // Today, so the wording is the time alone.
  at: new Date(new Date().setHours(12, 0, 0, 0)).toISOString(),
  error: "rate limited",
};

const run = (viz: VizConfig["viz"], over: { truncated?: boolean; coverage?: SourceCoverage[] }): VizRun => ({
  config: configOf(viz),
  params: {},
  result: {
    columns: ["title"],
    rows: [["One"], ["Two"]] as SqlCell[][],
    truncated: over.truncated ?? false,
    reads: { models: [], tables: [], measures: [] },
    freshness: [],
    coverage: over.coverage,
  },
  alert: null,
  warnings: [],
  inactive: null,
});

for (const viz of ["table", "list", "number"] as const) {
  test(`a truncated ${viz} viz says it shows the first rows`, () => {
    const { getByTestId } = render(<VizView run={run(viz, { truncated: true })} />);
    expect(getByTestId("viz-truncated").textContent).toBe("Showing the first 2 rows.");
  });
}

test("a source that failed is named above the rows", () => {
  const { getByTestId } = render(<VizView run={run("table", { coverage: [failed] })} />);
  expect(getByTestId("viz-coverage").textContent).toMatch(/^Pull request \(collector my-gh\/gh\) failed at 12:00: rate limited\.$/);
});

test("a whole, current result shows no line, and a strip shows none either", () => {
  const whole = render(<VizView run={run("table", { coverage: [{ ...failed, status: "current" }] })} />);
  expect(whole.queryByTestId("viz-completeness")).toBeNull();
  whole.unmount();
  const strip = render(<VizView run={run("number", { truncated: true })} compact />);
  expect(strip.queryByTestId("viz-completeness")).toBeNull();
});
