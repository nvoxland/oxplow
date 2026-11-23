import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, createEvent, fireEvent, render, waitFor } from "@testing-library/react";

// The rail's three groups: this thread's panels, this stream's, the
// project's. The dividers between them and the Remove zone show only while
// a panel is dragged; a drop on one moves the panel there. A thread or
// stream panel says so on its header, and its menu can move it.

type Placement = { panel: string; hidden: boolean; collapsed: boolean };
type Layout = { thread: Placement[]; stream: Placement[]; project: Placement[] };
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const shown = (panel: string): Placement => ({ panel, hidden: false, collapsed: false });
const hiddenRow = (panel: string): Placement => ({ panel, hidden: true, collapsed: false });
const saved: Layout[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => ({
    thread: [shown("ext:x/prs")],
    stream: [shown("ext:x/git")],
    project: [shown("ext:x/go"), shown("ext:x/work"), hiddenRow("ext:x/prs"), hiddenRow("ext:x/git")],
  }),
  setPanelLayout: async (_stream: string | null, _thread: string | null, layout: Layout) => {
    saved.push(layout);
  },
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      pages: [],
      panels: ["go", "work", "git", "prs"].map((id) => ({
        id: `x/${id}`, extension: "x", title: id.toUpperCase(), icon: null, dataScope: "project", body: "x/b", badge: null, open: null, collapsed: null, count: null,
      })),
      ui: { slots: [], commands: [], decorators: [], replacements: [] },
    },
  ],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { PanelLayoutProvider } = await import("../Panels/PanelLayoutContext.js");

afterEach(() => {
  cleanup();
  saved.length = 0;
});

async function rail() {
  const view = render(
    <PanelRunsProvider streamId="str1" threadId="thr1">
      <PanelLayoutProvider streamId="str1" threadId="thr1">
        <RailHud streamId="str1" onOpenPage={() => {}} />
      </PanelLayoutProvider>
    </PanelRunsProvider>,
  );
  await waitFor(() => expect(view.getByTestId("rail-hud").getAttribute("aria-busy")).toBe("false"));
  await waitFor(() => view.getByTestId("rail-section-ext:x/prs"));
  return view;
}

function dataTransfer() {
  const data: Record<string, string> = {};
  return {
    setData: (key: string, value: string) => {
      data[key] = value;
    },
    getData: (key: string) => data[key] ?? "",
    get types() {
      return Object.keys(data);
    },
    effectAllowed: "",
    dropEffect: "",
  };
}

const order = (view: Awaited<ReturnType<typeof rail>>) =>
  [...view.container.querySelectorAll('[data-testid^="rail-section-ext:"]')].map((el) => el.getAttribute("data-testid")!.slice("rail-section-".length));

test("the rail is this thread's panels, then this stream's, then the project's", async () => {
  const view = await rail();
  expect(order(view)).toEqual(["ext:x/prs", "ext:x/git", "ext:x/go", "ext:x/work"]);
  // No dividers and no Remove zone until a drag starts.
  expect(view.queryByTestId("rail-group-divider-stream")).toBeNull();
  expect(view.queryByTestId("rail-remove-zone")).toBeNull();
});

test("a thread's or stream's panel says so on its header; a project's says nothing", async () => {
  const view = await rail();
  expect(view.getByTestId("rail-section-shown-in-ext:x/prs").getAttribute("title")).toBe("Only in this thread");
  expect(view.getByTestId("rail-section-shown-in-ext:x/git").getAttribute("title")).toBe("Only in this stream");
  expect(view.queryByTestId("rail-section-shown-in-ext:x/go")).toBeNull();
});

test("while a panel is dragged the groups' dividers show, and a drop on one moves it to that group's end", async () => {
  const view = await rail();
  const dt = dataTransfer();
  fireEvent.dragStart(view.getByTestId("rail-section-drag-ext:x/go"), { dataTransfer: dt });
  // Not during dragstart: WebKit cancels a drag whose page changes then.
  expect(view.queryByTestId("rail-group-divider-stream")).toBeNull();
  await waitFor(() => view.getByTestId("rail-group-divider-stream"));
  for (const group of ["thread", "stream", "project"]) {
    expect(view.getByTestId(`rail-group-divider-${group}`).textContent).toContain(
      group === "thread" ? "This thread" : group === "stream" ? "This stream" : "This project",
    );
  }
  const divider = view.getByTestId("rail-group-divider-stream");
  fireEvent(divider, createEvent.dragOver(divider, { dataTransfer: dt }));
  fireEvent(divider, createEvent.drop(divider, { dataTransfer: dt }));
  await waitFor(() => expect(saved.length).toBe(1));
  expect(saved[0]!.stream.map((p) => p.panel)).toEqual(["ext:x/git", "ext:x/go"]);
  expect(order(view)).toEqual(["ext:x/prs", "ext:x/git", "ext:x/go", "ext:x/work"]);
  // The drag is over: the dividers go.
  expect(view.queryByTestId("rail-group-divider-stream")).toBeNull();
});

test("a drop on the Remove zone takes the panel out of the rail", async () => {
  const view = await rail();
  const dt = dataTransfer();
  fireEvent.dragStart(view.getByTestId("rail-section-drag-ext:x/prs"), { dataTransfer: dt });
  const zone = await waitFor(() => view.getByTestId("rail-remove-zone"));
  fireEvent(zone, createEvent.dragOver(zone, { dataTransfer: dt }));
  fireEvent(zone, createEvent.drop(zone, { dataTransfer: dt }));
  await waitFor(() => expect(saved.length).toBe(1));
  expect(saved[0]!.thread).toEqual([]);
  expect(order(view)).toEqual(["ext:x/git", "ext:x/go", "ext:x/work"]);
});

test("a header's menu moves the panel with Show in, the current group checked", async () => {
  const view = await rail();
  fireEvent.contextMenu(view.getByTestId("rail-section-toggle-ext:x/go"), { clientX: 10, clientY: 10 });
  fireEvent.click(view.getByTestId("menu-item-panel.show-in"));
  expect(view.getByTestId("menu-item-panel.show-in.project").textContent).toContain("✓");
  expect(view.getByTestId("menu-item-panel.show-in.thread").textContent).not.toContain("✓");
  fireEvent.click(view.getByTestId("menu-item-panel.show-in.thread"));
  await waitFor(() => expect(saved.length).toBe(1));
  expect(saved[0]!.thread.map((p) => p.panel)).toEqual(["ext:x/prs", "ext:x/go"]);
});

test("a right-click on the rail's empty space lists its panels, checked when shown; picking one hides it", async () => {
  const view = await rail();
  expect(view.queryByTestId("rail-add-panel")).toBeNull();
  fireEvent.contextMenu(view.getByTestId("rail-panels"), { clientX: 10, clientY: 300 });
  for (const id of ["prs", "git", "go", "work"]) {
    expect(view.getByTestId(`menu-item-view.rail:ext:x/${id}`).textContent).toContain("✓");
  }
  fireEvent.click(view.getByTestId("menu-item-view.rail:ext:x/work"));
  await waitFor(() => expect(saved.length).toBe(1));
  expect(order(view)).toEqual(["ext:x/prs", "ext:x/git", "ext:x/go"]);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
