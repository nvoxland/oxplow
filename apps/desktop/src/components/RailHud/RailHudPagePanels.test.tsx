import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ExtensionPanel, PageRun } from "../../tauri-bridge/generated/bindings.js";
import { createPageRuns, type PageKey } from "../../pageRuns.js";
import { decl, fakePage, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";

// A panel whose parts name pages (`pages: true`): its body is the body
// page drawn small, its header count the badge page's alert, and its
// header opens the body page.

const realApi = await import("../../api.js");
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
const pageOf = (slug: string) => fakePage(node.viz("rows", "rows", { columns: [{ key: "t" } as never] }), { id: `z/${slug}`, extension: "z", slug, title: slug, loaders: [decl("rows")] });
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

function fakeStore(opened: PageKey[]) {
  return createPageRuns({
    open: async (key) => {
      opened.push(key);
      const run: PageRun =
        key.page === "z/badge"
          ? { ...fakeRun(pageOf("badge"), { loaders: { rows: loader(["t"], [["x"]]) } }), alert: { firing: true, count: 2, value: null, message: "2 waiting" } as never }
          : fakeRun(pageOf("body"), { loaders: { rows: loader(["t"], [["body row"]]) } });
      return { runId: `run-${key.page}`, run };
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
  expect(opened.map((k) => [k.page, k.params])).toEqual([
    ["z/body", { stream_id: 1 }],
    ["z/badge", { stream_id: 1 }],
  ]);
  await waitFor(() => expect(view.getByTestId("rail-section-toggle-ext:z/w").textContent).toContain("2"));
  expect(view.getByTestId("alerts").textContent).toBe("2 waiting@oxplow:oxplow/page?page=z/badge");
  fireEvent.click(view.getByTestId("rail-section-open-ext:z/w"));
  expect(openedPages).toEqual(["oxplow:oxplow/page?page=z/body"]);
});
