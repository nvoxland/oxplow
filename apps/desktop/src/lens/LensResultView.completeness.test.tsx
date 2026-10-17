import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { Lens, LensRun, SourceCoverage, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A result that is cut short, or fed by a collector that hasn't kept up,
// says so above its rows, whatever the viz.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { LensResultView } = await import("./LensResultView.js");

afterEach(cleanup);

const lens = (viz: Lens["viz"]): Lens => ({
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
  path: "",
});

const failed: SourceCoverage = {
  model: "v_my_gh_pull_request",
  source: "collector:my-gh/gh",
  status: "failed",
  // Today, so the wording is the time alone.
  at: new Date(new Date().setHours(12, 0, 0, 0)).toISOString(),
  error: "rate limited",
};

const run = (viz: Lens["viz"], over: { truncated?: boolean; coverage?: SourceCoverage[] }): LensRun => ({
  lens: lens(viz),
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
  test(`a truncated ${viz} lens says it shows the first rows`, () => {
    const { getByTestId } = render(<LensResultView run={run(viz, { truncated: true })} />);
    expect(getByTestId("lens-truncated").textContent).toBe("Showing the first 2 rows.");
  });
}

test("a source that failed is named above the rows", () => {
  const { getByTestId } = render(<LensResultView run={run("table", { coverage: [failed] })} />);
  expect(getByTestId("lens-coverage").textContent).toMatch(/^Pull request \(collector my-gh\/gh\) failed at 12:00: rate limited\.$/);
});

test("a whole, current result shows no line, and a strip shows none either", () => {
  const whole = render(<LensResultView run={run("table", { coverage: [{ ...failed, status: "current" }] })} />);
  expect(whole.queryByTestId("lens-completeness")).toBeNull();
  whole.unmount();
  const strip = render(<LensResultView run={run("number", { truncated: true })} compact />);
  expect(strip.queryByTestId("lens-completeness")).toBeNull();
});
