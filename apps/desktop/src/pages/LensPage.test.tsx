import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { lensRef, resourceParamsOf, viewParamsOf } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

// A lens page is a function of its entry: its filters and picks are its
// URI's `#` view, and a change goes back through the tab — a filter
// replaces the entry, a row picked pushes one.

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

function nav(ref: TabRef, replaced: { ref: TabRef; push: boolean }[], historyState?: Record<string, unknown>, states?: Record<string, unknown>[]): PageNavigation {
  return {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref,
    viewParams: viewParamsOf(ref),
    setViewParams: (params, opts) => replaced.push({ ref: lensRef("x/y", {}, params), push: !!opts?.push }),
    historyState,
    setHistoryState: (patch) => states?.push(patch),
  };
}

function page(ref: TabRef, replaced: { ref: TabRef; push: boolean }[], historyState?: Record<string, unknown>, states?: Record<string, unknown>[]) {
  const params = resourceParamsOf(ref);
  return (
    <PageNavigationContext.Provider value={nav(ref, replaced, historyState, states)}>
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
  expect(replaced).toEqual([{ ref: lensRef("x/y", {}, { range: "7d" }), push: false }]);
  // The tab now shows that ref: the page redraws from it, not remounted.
  view.rerender(page(lensRef("x/y", {}, { range: "7d" }), replaced));
  await waitFor(() => expect(runs.at(-1)).toEqual({ range: "7d" }));
  fireEvent.click(view.getByTestId("lens-param-range-value"));
  fireEvent.change(view.getByTestId("lens-param-range-edit"), { target: { value: "30d" } });
  expect(replaced.at(-1)).toEqual({ ref: lensRef("x/y"), push: false });
});

test("a row picked pushes an entry, keeping the filters", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y", {}, { range: "7d" }), replaced));
  await waitFor(() => expect(view.getByTestId("lens-row-1")).not.toBeNull());
  fireEvent.click(view.getByTestId("lens-row-1"));
  expect(replaced).toEqual([{ ref: lensRef("x/y", {}, { range: "7d", commit: "def" }), push: true }]);
});

// A filter or a pick in the URI's `?` is a link spelled wrong: refused,
// never moved into the view.
test("a filter in the URI's `?` is refused, naming it", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y", { range: "7d" }), replaced));
  await waitFor(() => expect(view.getByTestId("lens-error").textContent).toContain("`range` is a filter or a pick"));
  expect(replaced).toEqual([]);
});

test("nothing is written back while nothing changes", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const view = render(page(lensRef("x/y", {}, { range: "7d" }), replaced));
  await waitFor(() => expect(runs).toEqual([{ range: "7d" }]));
  view.rerender(page(lensRef("x/y", {}, { range: "7d" }), replaced));
  expect(replaced).toEqual([]);
  expect(runs).toHaveLength(1);
});

// The row a click picked is the entry's history state: after Back or a
// restart the rail shows the row's facts, not the bare value.
test("the picked row is written to, and read from, the entry's history state", async () => {
  const replaced: { ref: TabRef; push: boolean }[] = [];
  const states: Record<string, unknown>[] = [];
  const view = render(page(lensRef("x/y", {}, { range: "7d" }), replaced, undefined, states));
  await waitFor(() => expect(view.getByTestId("lens-row-1")).not.toBeNull());
  fireEvent.click(view.getByTestId("lens-row-1"));
  expect(states).toEqual([{ picked: { commit: { value: "def", facts: [{ label: "Commit", text: "def" }] } } }]);
  cleanup();
  const back = render(
    page(lensRef("x/y", {}, { commit: "def" }), replaced, { picked: { commit: { value: "def", facts: [{ label: "Subject", text: "Fix the thing" }] } } }),
  );
  await waitFor(() => expect(back.getByTestId("lens-selection-commit").textContent).toContain("Fix the thing"));
});
