import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import type { ExtensionPanel, PageRun, SqlCell, VizGroup } from "../../tauri-bridge/generated/bindings.js";
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
  dataScope: "stream",
  body: "z/body",
  badge: "z/badge",
  open: null,
  collapsed: null,
  count: null,
  pages: true,
};
// A second panel, so a panel that grows has others to hide.
const other: ExtensionPanel = { ...panel, id: "z/o", title: "Other", body: "z/other", badge: null };
const layoutSaves: unknown[] = [];
// The body's grouping, when a test sets one: rows `[group, title]`.
let bodyGroup: VizGroup | null = null;
const pageOf = (slug: string) =>
  fakePage(
    node.viz("rows", "rows", {
      columns: [{ key: "t" } as never],
      actions: slug === "body" ? [fakeAction({ id: "tidy", label: "Tidy" })] : [],
      ...(slug === "body" && bodyGroup ? { viz: "list" as const, group: bodyGroup } : {}),
    }),
    {
    id: `z/${slug}`,
    extension: "z",
    slug,
    title: slug,
    loaders: [decl("rows")],
  },
  );
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => ({ thread: [], stream: [], project: [] }),
  setPanelLayout: async (_stream: unknown, _thread: unknown, layout: unknown) => {
    layoutSaves.push(layout);
  },
  listExtensions: async () => [
    { name: "z", namespace: "z", enabled: true, pages: [], pageFiles: [pageOf("body"), pageOf("badge"), pageOf("other")], panels: [panel, other], ui: { slots: [], commands: [], decorators: [], replacements: [] } },
  ],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");
const { PanelLayoutProvider } = await import("../Panels/PanelLayoutContext.js");
/** The rail as the app draws it: inside the viewer's layout. */
function LaidOutRail(props: Parameters<typeof RailHud>[0]) {
  return (
    <PanelLayoutProvider streamId={props.streamId ?? null} threadId={null}>
      <RailHud {...props} />
    </PanelLayoutProvider>
  );
}
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { usePanelRuns } = await import("../Panels/PanelRunsContext.js");

afterEach(cleanup);

function fakeStore(opened: PageKey[], bodyRows: SqlCell[][] = [["body row"]], bodyColumns: string[] = ["t"]) {
  return createPageRuns({
    open: async (key) => {
      opened.push(key);
      const run: PageRun =
        key.page === "z/badge"
          ? { ...fakeRun(pageOf("badge"), { loaders: { rows: loader(["t"], [["x"]]) } }), alert: { firing: true, count: 2, value: null, message: "2 waiting" } as never }
          : key.page === "z/other"
            ? fakeRun(pageOf("other"), { loaders: { rows: loader(["t"], [["other row"]]) } })
            : fakeRun(pageOf("body"), { loaders: { rows: loader(bodyColumns, bodyRows) } });
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
      <LaidOutRail streamId="str1" onOpenPage={(ref) => openedPages.push(ref.id)} />
      <Alerts />
    </PanelRunsProvider>,
  );
  await waitFor(() => expect(view.getAllByTestId("rail-panel-body")[0]!.textContent).toContain("body row"));
  // Each part is the daemon's to bind (`open_panel_part`): the window names
  // the panel and the part, never the stream's id.
  expect(opened.map((k) => [k.page, k.panel, k.params, k.streamId])).toEqual([
    ["z/body", { panel: "z/w", part: "body", choices: {} }, {}, "str1"],
    ["z/badge", { panel: "z/w", part: "badge", choices: {} }, {}, "str1"],
    ["z/other", { panel: "z/o", part: "body", choices: {} }, {}, "str1"],
  ]);
  await waitFor(() => expect(view.getByTestId("rail-section-toggle-ext:z/w").textContent).toContain("2"));
  expect(view.getByTestId("alerts").textContent).toBe("2 waiting@oxplow:oxplow/page?page=z/badge");
  // The header's open arrow is the one every heading uses (`OpenPageLink`).
  const open = within(view.getByTestId("rail-section-open-ext:z/w")).getByTestId("open-page-link");
  expect(open.getAttribute("title")).toBe("Open Waiting");
  fireEvent.click(open);
  expect(openedPages).toEqual(["oxplow:oxplow/page?page=z/body"]);
});

// A body with more rows than a panel shows ends with how many more. That
// row grows the panel to the whole rail: the other panels are hidden, the
// body shows every row and scrolls. The shrink button or Escape puts the
// rail back. It is the window's state only: the layout is never saved.
test("a panel's \"+N more\" grows it to the whole rail, and shrink or Escape puts the rail back", async () => {
  layoutSaves.length = 0;
  const rows = Array.from({ length: 11 }, (_, i) => [`row ${i}`]);
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([], rows)}>
      <LaidOutRail streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const section = () => view.getByTestId("rail-section-ext:z/w");
  const otherSection = () => view.getByTestId("rail-section-ext:z/o");
  const grow = async () => {
    const more = await waitFor(() => within(section()).getByTestId("viz-more"));
    expect(more.textContent).toBe("+3 more");
    fireEvent.click(within(more).getByRole("button"));
  };
  await waitFor(() => expect(otherSection().textContent).toContain("other row"));
  await grow();
  expect(section().getAttribute("data-filled")).toBe("true");
  expect(otherSection().style.display).toBe("none");
  // Every row, and no "+N more": the body scrolls instead.
  expect(within(section()).queryByTestId("viz-more")).toBeNull();
  expect(section().textContent).toContain("row 10");
  expect(within(section()).getByTestId("rail-panel-body").style.overflowY).toBe("auto");
  // Nothing to reorder it among: no drag handle.
  expect(within(section()).queryByTestId("rail-section-drag-ext:z/w")).toBeNull();
  // The header's chevron is the shrink button now.
  expect(within(section()).queryByTestId("rail-section-toggle-ext:z/w")?.getAttribute("title")).toBe("Shrink");
  fireEvent.click(within(section()).getByTestId("rail-section-toggle-ext:z/w"));
  expect(section().getAttribute("data-filled")).toBeNull();
  expect(otherSection().style.display).toBe("");
  // Restored, it is capped again and its chevron collapses it.
  expect(within(section()).getByTestId("viz-more").textContent).toBe("+3 more");
  expect(within(section()).getByTestId("rail-section-toggle-ext:z/w").getAttribute("title")).toBe("Collapse");
  expect(within(section()).getByTestId("rail-section-drag-ext:z/w")).toBeTruthy();
  // Escape puts the rail back too.
  await grow();
  fireEvent.keyDown(section(), { key: "Escape" });
  expect(section().getAttribute("data-filled")).toBeNull();
  expect(otherSection().style.display).toBe("");
  expect(layoutSaves).toEqual([]);
});

// A list that caps its groups (`group.compact`, `group.grown`) ends each
// capped group in "+N more": in the rail it grows the panel, grown it
// opens the panel's page.
test("a capped group's +N more grows the panel, and grown it opens the panel's page", async () => {
  bodyGroup = { by: "g", link: null, values: [], labels: {}, compact: { Ready: 1 }, grown: { Ready: 2 } };
  const opened: string[] = [];
  const rows = ["a", "b", "c"].map((t): SqlCell[] => ["Ready", t]);
  try {
    const view = render(
      <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([], rows, ["g", "t"])}>
        <LaidOutRail streamId="str1" onOpenPage={(ref) => opened.push(ref.id)} />
      </PanelRunsProvider>,
    );
    const section = () => view.getByTestId("rail-section-ext:z/w");
    const more = () => within(section()).getByTestId("viz-group-more-Ready");
    await waitFor(() => expect(more().textContent).toBe("+2 more"));
    fireEvent.click(within(more()).getByRole("button"));
    expect(section().getAttribute("data-filled")).toBe("true");
    expect(more().textContent).toBe("+1 more");
    fireEvent.click(within(more()).getByRole("button"));
    expect(opened).toEqual(["oxplow:oxplow/page?page=z/body"]);
  } finally {
    bodyGroup = null;
  }
});

// A section is named for a view transition only while one runs: a name
// makes the section the box its fixed descendants place against, so a
// row's right-click menu would be clipped by the rail.
test("at rest no rail section carries a view-transition name", async () => {
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([])}>
      <LaidOutRail streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  await waitFor(() => expect(view.getByTestId("rail-section-ext:z/w").textContent).toContain("body row"));
  for (const id of ["ext:z/w", "ext:z/o"]) {
    expect(view.getByTestId(`rail-section-${id}`).getAttribute("style") ?? "").not.toContain("view-transition-name");
  }
});

// A panel whose rows fit has no "+N more", so nothing to grow it.
test("a panel whose rows fit offers no way to grow it", async () => {
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([])}>
      <LaidOutRail streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  await waitFor(() => expect(view.getByTestId("rail-section-ext:z/w").textContent).toContain("body row"));
  expect(view.queryByTestId("viz-more")).toBeNull();
});

// The body is drawn small but keeps its buttons: the panel is where they
// are pressed (only the collapsed line, one line of stats, has none).
test("a panel's body draws its buttons", async () => {
  const view = render(
    <PanelRunsProvider streamId="str1" threadId={null} pageRuns={fakeStore([])}>
      <LaidOutRail streamId="str1" onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const body = await waitFor(() => within(view.getByTestId("rail-section-ext:z/w")).getByTestId("rail-panel-body"));
  await waitFor(() => expect(within(body).getByTestId("viz-action-tidy").textContent).toBe("Tidy"));
  // Drawn as a compact part: the kit's sheet keeps it tight.
  expect(body.classList.contains("ox-compact")).toBe(true);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
