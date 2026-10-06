import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// tsk972: the stored panel layout loads after the first render. A toggle
// the person makes before it arrives is theirs: the late load doesn't
// undo it, and doesn't lose the rest of what was stored either.

type Placement = { panel: string; hidden: boolean; collapsed: boolean };
const realApi = await import("../../api.js");
let deliverLayout: (layout: Placement[]) => void = () => {};
const saved: Placement[][] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: () =>
    new Promise<Placement[]>((resolve) => {
      deliverLayout = resolve;
    }),
  setPanelLayout: async (layout: Placement[]) => {
    saved.push(layout);
  },
  // Two extension panels, so one can be toggled and the other hidden by
  // the stored layout.
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      panels: [
        { id: "x/v", extension: "x", title: "V", icon: null, scope: "project", body: "x/b", badge: null, open: null, collapsed: null, count: null },
        { id: "x/w", extension: "x", title: "W", icon: null, scope: "project", body: "x/b", badge: null, open: null, collapsed: null, count: null },
      ],
      lenses: [],
      ui: { slots: [], commands: [], decorators: [], replacements: [] },
    },
  ],
  runLens: async (id: string) => ({
    lens: { id, viz: "list", params: [], columns: [], actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: [] },
    alert: null,
    warnings: [],
  }),
  listCommentsForStream: async () => [],
  subscribeCommentEvents: () => () => {},
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");

afterEach(cleanup);

test("a toggle made before the stored layout loads survives it, and keeps what was stored", async () => {
  const view = render(
    <PanelRunsProvider streamId={null} threadId={null}>
      <RailHud streamId={null} onOpenPage={() => {}} />
    </PanelRunsProvider>,
  );
  const toggle = await waitFor(() => view.getByTestId("rail-section-toggle-ext:x/v"));
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(saved, "nothing is written over a layout not loaded yet").toEqual([]);
  await act(async () => {
    deliverLayout([
      { panel: "ext:x/v", hidden: false, collapsed: false },
      { panel: "ext:x/w", hidden: true, collapsed: false },
    ]);
  });
  expect(view.getByTestId("rail-section-toggle-ext:x/v").getAttribute("aria-expanded")).toBe("false");
  expect(view.queryByTestId("rail-section-ext:x/w"), "the stored hide stands").toBeNull();
  const last = saved.at(-1)!;
  expect(last.find((p) => p.panel === "ext:x/v")).toMatchObject({ collapsed: true });
  expect(last.find((p) => p.panel === "ext:x/w")).toMatchObject({ hidden: true });
});
