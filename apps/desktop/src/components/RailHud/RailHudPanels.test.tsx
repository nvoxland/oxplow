import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { createRememberedStore, panelScope, RememberedContext } from "../../prefs/remembered.js";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ExtensionPanel, PageRun, PartView, SqlCell } from "../../tauri-bridge/generated/bindings.js";
import { decl, fakePage, fakePageRunsBy, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";
import type { PageKey } from "../../pageRuns.js";

// An extension panel can name a page its collapsed header shows
// (a summary, compact) and a page whose row count is its header count.

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
// The body page has a choice param: the panel's header toggle.
const MODE = {
  name: "mode",
  label: "Show",
  default: "recent",
  options: [{ value: "recent", label: "Recent" }, { value: "top", label: "Top" }],
};
const rowsOf: Record<string, SqlCell[][]> = {
  "x/body": [["body row"]],
  "x/line": [["summary row"]],
  "x/n": [["a"], ["b"], ["c"]],
};
const pageRun = (id: string, params: Record<string, SqlCell>): PageRun => {
  const [extension, slug] = id.split("/") as [string, string];
  const page = fakePage(node.viz("rows", "rows", { columns: [{ key: "t" } as never] }), {
    id,
    extension,
    slug,
    title: id,
    params: id === "x/body" ? ([MODE] as never) : [],
    loaders: [decl("rows")],
  });
  return fakeRun(page, { params, loaders: { rows: loader(["t"], rowsOf[id]!) } });
};
/** What the daemon says each part gives the panel: the count page counts,
 *  the body has the toggle at the value it ran with. */
const viewOf = (key: PageKey, run: PageRun): PartView => ({
  counts: key.panel?.part === "count",
  count: key.panel?.part === "count" ? rowsOf["x/n"]!.length : null,
  choices: key.panel?.part === "body" ? [{ ...MODE, value: String(run.params.mode ?? MODE.default) }] : [],
});
const panel: ExtensionPanel = {
  id: "x/w",
  extension: "x",
  title: "Work",
  icon: null,
  scope: "project",
  body: "x/body",
  badge: null,
  open: null,
  collapsed: "x/line",
  count: "x/n",
};
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => [{ panel: "ext:x/w", hidden: false, collapsed: true }],
  setPanelLayout: async () => {},
  listExtensions: async () => [{ name: "x", enabled: true, pages: [], panels: [panel], ui: { slots: [], commands: [], decorators: [], replacements: [] } }],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");

afterEach(cleanup);

test("a collapsed panel shows its collapsed page; its header counts the count page's rows", async () => {
  const { runs } = fakePageRunsBy((key) => pageRun(key.page!, key.params), viewOf);
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={runs}>
      <RailHud streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const section = await waitFor(() => view.getByTestId("rail-section-ext:x/w"));
  await waitFor(() => expect(view.getByTestId("rail-panel-collapsed").textContent).toContain("summary row"));
  expect(view.queryByTestId("rail-panel-body")).toBeNull();
  expect(view.getByTestId("rail-section-toggle-ext:x/w").textContent).toContain("3");
  expect(section.textContent).not.toContain("body row");
});

test("a body page's choice param is a header toggle; picking re-runs the body with it", async () => {
  // As the daemon runs a panel's part: the body with the viewer's picks.
  const { runs, opened } = fakePageRunsBy((key) => pageRun(key.page!, { ...key.params, ...(key.panel?.choices ?? {}) }), viewOf);
  const store = createRememberedStore({ load: async () => [], save: async () => {} });
  const view = render(
    <RememberedContext.Provider value={store}>
      <PanelRunsProvider streamId="str1" threadId={null} pageRuns={runs}>
        <RailHud streamId="str1" onOpenPage={() => {}} />
      </PanelRunsProvider>
    </RememberedContext.Provider>,
  );
  const top = await waitFor(() => view.getByTestId("rail-panel-choice-mode-top"));
  expect(view.getByTestId("rail-panel-choice-mode-recent").getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(top);
  await waitFor(() => expect(opened.some((o) => o.key.page === "x/body" && o.key.panel?.choices.mode === "top")).toBe(true));
  await waitFor(() => expect(view.getByTestId("rail-panel-choice-mode-top").getAttribute("aria-pressed")).toBe("true"));
  // The pick is the panel's memory, not the browser's.
  expect(store.get(panelScope("x/w"), "choices")).toEqual({ mode: "top" });
});

test("a count of zero draws no badge in the header", async () => {
  const saved = rowsOf["x/n"];
  rowsOf["x/n"] = [];
  try {
    const { runs } = fakePageRunsBy((key) => pageRun(key.page!, key.params), viewOf);
    const view = render(
      <PanelRunsProvider streamId="str1" threadId={null} pageRuns={runs}>
        <RailHud streamId="str1" onOpenPage={() => {}} />
      </PanelRunsProvider>,
    );
    await waitFor(() => view.getByTestId("rail-panel-collapsed"));
    expect(view.getByTestId("rail-section-toggle-ext:x/w").textContent).toBe("▸Work");
  } finally {
    rowsOf["x/n"] = saved!;
  }
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
