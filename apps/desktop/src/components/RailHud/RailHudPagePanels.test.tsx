import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import type { ExtensionPanel, PageRun } from "../../tauri-bridge/generated/bindings.js";
import { createPageRuns, type PageKey } from "../../pageRuns.js";
import { decl, fakeAction, fakePage, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";

// A panel whose parts name pages (`pages: true`): its body is the body
// page drawn small, its header count the badge page's alert, and its
// header opens the body page.

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const panel: ExtensionPanel = {
  id: "z/w",
  extension: "z",
  title: "Waiting",
  icon: null,
  scope: "stream",
  body: "z/body",
  badge: "z/badge",
  open: null,
  collapsed: null,
  count: null,
  pages: true,
};
const pageOf = (slug: string) =>
  fakePage(node.viz("rows", "rows", { columns: [{ key: "t" } as never], actions: slug === "body" ? [fakeAction({ id: "tidy", label: "Tidy" })] : [] }), {
    id: `z/${slug}`,
    extension: "z",
    slug,
    title: slug,
    loaders: [decl("rows")],
  });
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => [],
  setPanelLayout: async () => {},
  listExtensions: async () => [
    { name: "z", namespace: "z", enabled: true, pages: [], pageFiles: [pageOf("body"), pageOf("badge")], panels: [panel], ui: { slots: [], commands: [], decorators: [], replacements: [] } },
  ],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { usePanelRuns } = await import("../Panels/PanelRunsContext.js");

afterEach(cleanup);

function fakeStore(opened: PageKey[], bodyRows: string[][] = [["body row"]]) {
  return createPageRuns({
    open: async (key) => {
      opened.push(key);
      const run: PageRun =
        key.page === "z/badge"
          ? { ...fakeRun(pageOf("badge"), { loaders: { rows: loader(["t"], [["x"]]) } }), alert: { firing: true, count: 2, value: null, message: "2 waiting" } as never }
          : fakeRun(pageOf("body"), { loaders: { rows: loader(["t"], bodyRows) } });
      // The daemon's word: the header counts by the badge (no count page).
      const panel = key.page === "z/badge" ? { counts: true, count: 2, choices: [] } : { counts: false, count: null, choices: [] };
      return { runId: `run-${key.page}`, run, panel };
    },
    close: async () => {},
    fetch: async () => ({}),
    setParams: async () => Promise.reject(new Error("no")),
    rerun: async () => Promise.reject(new Error("no")),
    onChange: () => () => {},
    onReconnect: () => () => {},
  });
}

function Alerts() {
  const { alerts } = usePanelRuns();
  return <div data-testid="alerts">{alerts.map((a) => `${a.message}@${a.open}`).join(",")}</div>;
}

test("the body page drawn small, the badge page's alert as the count and an Alerts row, the header opening the body page", async () => {
  const opened: PageKey[] = [];
  const openedPages: string[] = [];
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore(opened)}>
      <RailHud streamId="str1" onOpenPage={(ref) => openedPages.push(ref.id)} />
      <Alerts />
    </PanelRunsProvider>,
  );
  await waitFor(() => expect(view.getByTestId("rail-panel-body").textContent).toContain("body row"));
  // Each part is the daemon's to bind (`open_panel_part`): the window names
  // the panel and the part, never the stream's id.
  expect(opened.map((k) => [k.page, k.panel, k.params, k.streamId])).toEqual([
    ["z/body", { panel: "z/w", part: "body", choices: {} }, {}, "str1"],
    ["z/badge", { panel: "z/w", part: "badge", choices: {} }, {}, "str1"],
  ]);
  await waitFor(() => expect(view.getByTestId("rail-section-toggle-ext:z/w").textContent).toContain("2"));
  expect(view.getByTestId("alerts").textContent).toBe("2 waiting@oxplow:oxplow/page?page=z/badge");
  // The header's open arrow is the one every heading uses (`OpenPageLink`).
  const open = within(view.getByTestId("rail-section-open-ext:z/w")).getByTestId("open-page-link");
  expect(open.getAttribute("title")).toBe("Open Waiting");
  fireEvent.click(open);
  expect(openedPages).toEqual(["oxplow:oxplow/page?page=z/body"]);
});

// A body with more rows than a panel shows ends with how many more, which
// opens the panel's page.
test("a panel's rows past its few end with how many more, opening the panel's page", async () => {
  const openedPages: string[] = [];
  const rows = Array.from({ length: 11 }, (_, i) => [`row ${i}`]);
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([], rows)}>
      <RailHud streamId="str1" onOpenPage={(ref) => openedPages.push(ref.id)} />
    </PanelRunsProvider>,
  );
  const more = await waitFor(() => view.getByTestId("viz-more"));
  expect(more.textContent).toBe("+3 more");
  fireEvent.click(within(more).getByRole("button"));
  expect(openedPages).toEqual(["oxplow:oxplow/page?page=z/body"]);
});

// The body is drawn small but keeps its buttons: the panel is where they
// are pressed (only the collapsed line, one line of stats, has none).
test("a panel's body draws its buttons", async () => {
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([])}>
      <RailHud streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const body = await waitFor(() => view.getByTestId("rail-panel-body"));
  await waitFor(() => expect(within(body).getByTestId("viz-action-tidy").textContent).toBe("Tidy"));
  // Drawn as a compact part: the kit's sheet keeps it tight.
  expect(body.classList.contains("ox-compact")).toBe(true);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
