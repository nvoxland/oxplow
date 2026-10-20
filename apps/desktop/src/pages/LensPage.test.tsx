import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { lensRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

// A lens page is a function of its entry: its params come from its ref,
// and a change goes back through the tab — a filter replaces the entry,
// a row picked pushes one.

const runs: Record<string, SqlCell>[] = [];
const realApi = await import("../api.js");
const lens = {
  id: "x/y",
  extension: "x",
  slug: "y",
  title: "Commits",
  description: "",
  query: "",
  viz: "table",
  params: [
    { name: "range", label: "When", default: "30d", options: [], type: "date_range", role: "filter", presets: ["7d", "30d"] },
    { name: "commit", label: "Commit", default: null, options: [], type: "text", role: "selection", presets: [] },
  ],
  columns: [{ key: "sha", label: "Commit", link: null, unit: null, icon: null, tone: null, select: "commit" }],
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
  origin: { extension: "x", source: null, running: "", deployment: null, bundled: false },
  tool: null,
} as unknown as Lens;
mock.module("../api.js", () => ({
  ...realApi,
  runLens: async (_id: string, params: Record<string, SqlCell>) => {
    runs.push(params);
    return {
      lens,
      params: { range: "30d", commit: null, ...params },
      result: { columns: ["sha"], rows: [["abc"], ["def"]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
    } as unknown as LensRun;
  },
}));
const { LensPage } = await import("./LensPage.js");

afterEach(() => {
  cleanup();
  runs.length = 0;
});

function nav(ref: TabRef, replaced: { ref: TabRef; push: boolean }[]): PageNavigation {
  return {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref,
    setViewParams: (params, opts) => replaced.push({ ref: lensRef("x/y", params), push: !!opts?.push }),
  };
}

function page(ref: TabRef, replaced: { ref: TabRef; push: boolean }[]) {
  const params = (ref.payload as { params?: Record<string, SqlCell> }).params;
  return (
    <PageNavigationContext.Provider value={nav(ref, replaced)}>
      <LensPage lensId="x/y" params={params} stream={null} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>
  );
}

test("a filter replaces the tab's entry; a value back at its default leaves the ref", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y"), replaced));
  await waitFor(() => expect(view.getByTestId("lens-param-range-value").textContent).toBe("Last 30 days"));
  fireEvent.click(view.getByTestId("lens-param-range-value"));
  fireEvent.change(view.getByTestId("lens-param-range-edit"), { target: { value: "7d" } });
  expect(replaced).toEqual([{ ref: lensRef("x/y", { range: "7d" }), push: false }]);
  // The tab now shows that ref: the page redraws from it, not remounted.
  view.rerender(page(lensRef("x/y", { range: "7d" }), replaced));
  await waitFor(() => expect(runs.at(-1)).toEqual({ range: "7d" }));
  fireEvent.click(view.getByTestId("lens-param-range-value"));
  fireEvent.change(view.getByTestId("lens-param-range-edit"), { target: { value: "30d" } });
  expect(replaced.at(-1)).toEqual({ ref: lensRef("x/y"), push: false });
});

test("a row picked pushes an entry, keeping the filters", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y", { range: "7d" }), replaced));
  await waitFor(() => expect(view.getByTestId("lens-row-1")).not.toBeNull());
  fireEvent.click(view.getByTestId("lens-row-1"));
  expect(replaced).toEqual([{ ref: lensRef("x/y", { range: "7d", commit: "def" }), push: true }]);
});

test("nothing is written back while nothing changes", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y", { range: "7d" }), replaced));
  await waitFor(() => expect(runs).toEqual([{ range: "7d" }]));
  view.rerender(page(lensRef("x/y", { range: "7d" }), replaced));
  expect(replaced).toEqual([]);
  expect(runs).toHaveLength(1);
});
