import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { lensRef, viewParamsOf } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

// A view document (`view: area`): its `area` names the one section in
// view, its rail lists the sections, a change of section is pushed, and
// its tab is named for the section. Any lens page lands on `#section=`.

const base = {
  extension: "x",
  description: "",
  query: "",
  viz: "table",
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
  view: null,
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "x", source: null, running: "", deployment: null, bundled: false },
  tool: null,
};
const part = (id: string, title: string, lens: string) => ({ id, title, lens, markdown: null, size: null, replaceable: null, from: null });
const filter = (name: string, over: Record<string, unknown> = {}) => ({ name, label: null, default: "", options: [], type: "search", role: "filter", presets: [], ...over });
const lenses: Record<string, Lens> = {
  "x/settings": {
    ...base,
    id: "x/settings",
    slug: "settings",
    title: "Settings",
    description: "Every setting.",
    viz: "document",
    view: "area",
    params: [
      filter("area", { default: "overview", type: "choice", options: [{ value: "overview", label: "Every Setting" }, { value: "agents", label: "Agents" }] }),
      filter("q", { label: "Search settings" }),
    ],
    parts: [part("overview", "Every Setting", "x/overview"), part("agents", "Agents", "x/agents")],
  },
  // The overview reads the page's search; Agents doesn't.
  "x/overview": { ...base, id: "x/overview", slug: "overview", title: "Every Setting", params: [filter("q")] },
  "x/agents": { ...base, id: "x/agents", slug: "agents", title: "Agents", viz: "document", parts: [part("list", "Harnesses", "x/list"), part("prompt", "Prompt", "x/list")] },
  "x/list": { ...base, id: "x/list", slug: "list", title: "List" },
} as unknown as Record<string, Lens>;

const runs: { id: string; params: Record<string, SqlCell> }[] = [];
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  getLens: async (id: string) => lenses[id]!,
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
  runLens: async (id: string, params: Record<string, SqlCell>) => {
    runs.push({ id, params });
    const lens = lenses[id]!;
    const defaults = Object.fromEntries(lens.params.map((p) => [p.name, p.default ?? null]));
    return {
      lens,
      params: { ...defaults, ...params },
      result: { columns: ["n"], rows: lens.viz === "document" ? [] : [["row"]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
    } as unknown as LensRun;
  },
}));
const { LensPage } = await import("./LensPage.js");

afterEach(() => {
  cleanup();
  runs.length = 0;
});

function page(ref: TabRef, sets: { view: Record<string, unknown>; push: boolean }[], titles: string[] = [], lensId = "x/settings") {
  const nav: PageNavigation = {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref,
    viewParams: viewParamsOf(ref),
    setViewParams: (view, opts) => sets.push({ view, push: !!opts?.push }),
    setTitle: (t: string) => titles.push(t),
  } as unknown as PageNavigation;
  return (
    <PageNavigationContext.Provider value={nav}>
      <LensPage lensId={lensId} title="Settings" stream={null} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>
  );
}

test("its rail lists the sections, the one in view marked; picking another pushes it", async () => {
  const sets: { view: Record<string, unknown>; push: boolean }[] = [];
  const view = render(page(lensRef("x/settings"), sets));
  await waitFor(() => expect(view.getByTestId("lens-view-overview")).toBeTruthy());
  expect(view.getByTestId("lens-view-overview").getAttribute("aria-current")).toBe("page");
  expect(view.getByTestId("lens-view-agents").getAttribute("aria-current")).toBeNull();
  // The view param is the section rail, never a filter.
  expect(view.queryByTestId("lens-param-area")).toBeNull();
  fireEvent.click(view.getByTestId("lens-view-agents"));
  expect(sets).toEqual([{ view: { area: "agents" }, push: true }]);
});

test("it is named for the section in view: its heading always, its tab off the first one", async () => {
  const titles: string[] = [];
  const first = render(page(lensRef("x/settings"), [], titles));
  await waitFor(() => expect(first.container.querySelector("h1")!.textContent).toBe("Every Setting"));
  expect(titles.at(-1)).toBe("Settings");
  // The page's own description gives way to the section's.
  expect(first.container.textContent).not.toContain("Every setting.");
  cleanup();
  const agents = render(page(lensRef("x/settings", {}, { area: "agents" }), [], titles));
  await waitFor(() => expect(agents.container.querySelector("h1")!.textContent).toBe("Agents"));
  expect(titles.at(-1)).toBe("Settings: Agents");
});

test("a filter is offered only while the section in view declares it", async () => {
  const overview = render(page(lensRef("x/settings"), []));
  await waitFor(() => expect(overview.getByTestId("lens-param-q")).toBeTruthy());
  cleanup();
  const agents = render(page(lensRef("x/settings", {}, { area: "agents" }), []));
  await waitFor(() => expect(agents.getByTestId("lens-view-agents")).toBeTruthy());
  await waitFor(() => expect(runs.some((r) => r.id === "x/agents")).toBe(true));
  expect(agents.queryByTestId("lens-param-q")).toBeNull();
});

test("`#section=` is the page's: left out of the run, and scrolled to once drawn", async () => {
  const scrolled: string[] = [];
  const real = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolled.push(this.getAttribute("data-section") ?? "");
  };
  try {
    render(page(lensRef("x/settings", {}, { area: "agents", section: "prompt" }), []));
    await waitFor(() => expect(scrolled).toContain("prompt"));
    expect(runs.find((r) => r.id === "x/settings")!.params).toEqual({ area: "agents" });
  } finally {
    Element.prototype.scrollIntoView = real;
  }
});
