import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// the stored panel layout loads after the first render. A toggle
// the person makes before it arrives is theirs: the late load doesn't
// undo it, and doesn't lose the rest of what was stored either.

type Placement = { panel: string; hidden: boolean; collapsed: boolean };
type Layout = { thread: Placement[]; stream: Placement[]; project: Placement[] };
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
let deliverLayout: (layout: Layout) => void = () => {};
const saved: Layout[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: () =>
    new Promise<Layout>((resolve) => {
      deliverLayout = resolve;
    }),
  setPanelLayout: async (_stream: string | null, _thread: string | null, layout: Layout) => {
    saved.push(layout);
  },
  // Two extension panels, so one can be toggled and the other hidden by
  // the stored layout.
  listExtensions: async () => [
    {
      name: "x",
      enabled: true, pages: [],
      panels: [
        { id: "x/v", extension: "x", title: "V", icon: null, dataScope: "project", body: "x/b", badge: null, open: null, collapsed: null, count: null },
        { id: "x/w", extension: "x", title: "W", icon: null, dataScope: "project", body: "x/b", badge: null, open: null, collapsed: null, count: null },
      ],
      ui: { slots: [], commands: [], decorators: [], replacements: [] },
    },
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

afterEach(cleanup);

test("a toggle made before the stored layout loads survives it, and keeps what was stored", async () => {
  const view = render(
    <PanelRunsProvider streamId={null} threadId={null}>
      <LaidOutRail streamId={null} onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const toggle = await waitFor(() => view.getByTestId("rail-section-toggle-ext:x/v"));
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(saved, "nothing is written over a layout not loaded yet").toEqual([]);
  await act(async () => {
    deliverLayout({
      thread: [],
      stream: [],
      project: [
        { panel: "ext:x/v", hidden: false, collapsed: false },
        { panel: "ext:x/w", hidden: true, collapsed: false },
      ],
    });
  });
  expect(view.getByTestId("rail-section-toggle-ext:x/v").getAttribute("aria-expanded")).toBe("false");
  expect(view.queryByTestId("rail-section-ext:x/w"), "the stored hide stands").toBeNull();
  const last = saved.at(-1)!.project;
  expect(last.find((p) => p.panel === "ext:x/v")).toMatchObject({ collapsed: true });
  expect(last.find((p) => p.panel === "ext:x/w")).toMatchObject({ hidden: true });
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
