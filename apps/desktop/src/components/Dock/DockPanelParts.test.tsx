import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ExtensionPanel, PageRun } from "../../tauri-bridge/generated/bindings.js";
import { createPageRuns, type PageKey } from "../../pageRuns.js";
import { decl, fakePage, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";

// A dock panel draws its parts as a rail panel does: the body's choice
// toggles in the header, the collapsed page in the bar while the panel is
// closed, and the firing badge's count on its icon.

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const panel: ExtensionPanel = {
  id: "z/d",
  extension: "z",
  title: "Docked",
  icon: null,
  dataScope: "stream",
  body: "z/body",
  badge: "z/badge",
  open: null,
  collapsed: "z/small",
  count: null,
  pages: true,
  dock: true,
} as ExtensionPanel;
const pageOf = (slug: string) =>
  fakePage(node.viz("rows", "rows", { columns: [{ key: "t" } as never] }), {
    id: `z/${slug}`,
    extension: "z",
    slug,
    title: slug,
    loaders: [decl("rows")],
  });
mock.module("../../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    { name: "z", namespace: "z", enabled: true, pages: [], pageFiles: [pageOf("body"), pageOf("badge"), pageOf("small")], panels: [panel], ui: { slots: [], commands: [], decorators: [], replacements: [] } },
  ],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { Dock } = await import("./Dock.js");
const { dockStore } = await import("./dockStore.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");

afterEach(() => {
  cleanup();
  dockStore.close();
});
afterAll(() => mock.module("../../api.js", () => realApi));

const choice = (value: string) => ({
  name: "mode",
  label: "Mode",
  value,
  options: [
    { value: "a", label: "A" },
    { value: "b", label: "B" },
  ],
});

function fakeStore(opened: PageKey[]) {
  return createPageRuns({
    open: async (key) => {
      opened.push(key);
      const picked = (key.panel?.choices as Record<string, string> | undefined)?.mode ?? "a";
      const text = key.page === "z/small" ? "summary line" : `body ${picked}`;
      const run: PageRun =
        key.page === "z/badge"
          ? { ...fakeRun(pageOf("badge"), { loaders: { rows: loader(["t"], [["x"]]) } }), alert: { firing: true, count: 3, value: null, message: "3 waiting" } as never }
          : fakeRun(pageOf(key.page.slice(2)), { loaders: { rows: loader(["t"], [[text]]) } });
      const view =
        key.page === "z/badge"
          ? { counts: true, count: 3, choices: [] }
          : key.page === "z/body"
            ? { counts: false, count: null, choices: [choice(picked)] }
            : { counts: false, count: null, choices: [] };
      return { runId: `run-${key.page}-${picked}`, run, panel: view };
    },
    close: async () => {},
    fetch: async () => ({}),
    setParams: async () => Promise.reject(new Error("no")),
    rerun: async () => Promise.reject(new Error("no")),
    onChange: () => () => {},
    onReconnect: () => () => {},
  });
}

function mount(opened: PageKey[] = []) {
  return render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore(opened)}>
      <Dock streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
}

test("the open dock panel's header holds the body's choice toggles, and a pick re-runs the parts", async () => {
  const opened: PageKey[] = [];
  const view = mount(opened);
  await waitFor(() => expect(view.getByTestId("dock-ext:z/d")).toBeTruthy());
  fireEvent.click(view.getByTestId("dock-ext:z/d"));
  await waitFor(() => expect(view.getByTestId("dock-panel-choice-mode-a").getAttribute("aria-pressed")).toBe("true"));
  expect(view.getByTestId("dock-panel-choice-mode-b").getAttribute("aria-pressed")).toBe("false");
  expect(view.getByTestId("dock-panel-header").contains(view.getByTestId("dock-panel-choice-mode-a"))).toBe(true);
  fireEvent.click(view.getByTestId("dock-panel-choice-mode-b"));
  await waitFor(() => expect(opened.some((k) => (k.panel?.choices as Record<string, string>)?.mode === "b")).toBe(true));
  await waitFor(() => expect(view.getByTestId("dock-panel-choice-mode-b").getAttribute("aria-pressed")).toBe("true"));
});

test("the collapsed page shows in the bar while the panel is closed, and goes when it opens", async () => {
  const view = mount();
  await waitFor(() => expect(view.getByTestId("dock-panel-collapsed").textContent).toContain("summary line"));
  fireEvent.click(view.getByTestId("dock-ext:z/d"));
  await waitFor(() => expect(view.getByTestId("dock-panel")).toBeTruthy());
  expect(view.queryByTestId("dock-panel-collapsed")).toBeNull();
});

test("a firing badge is the count on the panel's icon", async () => {
  const view = mount();
  await waitFor(() => expect(view.getByTestId("dock-ext:z/d-badge").textContent).toBe("3"));
});
