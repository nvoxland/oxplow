import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { ExtensionPanel, Lens, LensRun, SqlCell } from "../../tauri-bridge/generated/bindings.js";

// tsk1089: an extension panel can name a lens its collapsed header shows
// (a summary, compact) and a lens whose row count is its header count.

const realApi = await import("../../api.js");
const lens = (id: string, viz: Lens["viz"] = "list"): Lens =>
  ({ id, extension: "x", slug: id, title: id, viz, params: [], columns: [], actions: [], empty: null, group: null, emphasis: null, depth: null }) as unknown as Lens;
const results: Record<string, { viz: Lens["viz"]; columns: string[]; rows: SqlCell[][] }> = {
  "x/body": { viz: "list", columns: ["t"], rows: [["body row"]] },
  "x/line": { viz: "list", columns: ["t"], rows: [["summary row"]] },
  "x/n": { viz: "list", columns: ["t"], rows: [["a"], ["b"], ["c"]] },
};
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
  listExtensions: async () => [{ name: "x", enabled: true, panels: [panel], lenses: [], ui: { slots: [], commands: [], decorators: [], replacements: [] } }],
  runLens: async (id: string, params: Record<string, SqlCell>): Promise<LensRun> => {
    const r = results[id]!;
    return {
      lens: lens(id, r.viz),
      params,
      result: { columns: r.columns, rows: r.rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: [] },
      alert: null,
      warnings: [],
    } as unknown as LensRun;
  },
  listRecentPageVisits: async () => [],
  topVisitedPages: async () => [],
  subscribePageVisitEvents: () => () => {},
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { RailHud } = await import("./RailHud.js");

afterEach(cleanup);

test("a collapsed panel shows its collapsed lens; its header counts the count lens's rows", async () => {
  const view = render(<RailHud threadId={null} streamId="str1" onOpenPage={() => {}} />);
  const section = await waitFor(() => view.getByTestId("rail-section-ext:x/w"));
  await waitFor(() => expect(view.getByTestId("rail-panel-collapsed").textContent).toContain("summary row"));
  expect(view.queryByTestId("rail-panel-body")).toBeNull();
  expect(view.getByTestId("rail-section-toggle-ext:x/w").textContent).toContain("3");
  expect(section.textContent).not.toContain("body row");
});
