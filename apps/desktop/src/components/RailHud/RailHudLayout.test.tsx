import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

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
  listExtensions: async () => [],
  listCommentsForStream: async () => [],
  listRecentPageVisits: async () => [],
  topVisitedPages: async () => [],
  subscribeCommentEvents: () => () => {},
  subscribePageVisitEvents: () => () => {},
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");

afterEach(cleanup);

test("a toggle made before the stored layout loads survives it, and keeps what was stored", async () => {
  const view = render(<RailHud threadId={null} streamId={null} threadWork={null} onOpenPage={() => {}} />);
  const toggle = view.getByTestId("rail-section-toggle-core:approvals");
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(saved, "nothing is written over a layout not loaded yet").toEqual([]);
  await act(async () => {
    deliverLayout([
      { panel: "core:approvals", hidden: false, collapsed: false },
      { panel: "core:bookmarks", hidden: true, collapsed: false },
    ]);
  });
  expect(view.getByTestId("rail-section-toggle-core:approvals").getAttribute("aria-expanded")).toBe("false");
  expect(view.queryByTestId("rail-section-core:bookmarks"), "the stored hide stands").toBeNull();
  const last = saved.at(-1)!;
  expect(last.find((p) => p.panel === "core:approvals")).toMatchObject({ collapsed: true });
  expect(last.find((p) => p.panel === "core:bookmarks")).toMatchObject({ hidden: true });
});
