import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { VizConfig, VizRun } from "../tauri-bridge/generated/bindings.js";

const { VizView } = await import("./VizView.js");

afterEach(cleanup);

const base: VizConfig = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "L",
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
};
const chart = { x: null, y: null, series: null, label: null, size: null, group: null };
const run = (viz: Partial<VizConfig>, columns: string[], rows: (string | number)[][]): VizRun => ({
  config: { ...base, ...viz },
  params: {},
  result: { columns, rows, truncated: false },
});
test("bar viz nodes draw one bar per row", () => {
  const { container } = render(
    <VizView
      run={run({ viz: "bar", chart: { ...chart, x: "day", y: "n" } }, ["day", "n"], [["mon", 3], ["tue", 5]])}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelector('[data-testid="viz-bar"]')).not.toBeNull();
  expect(container.querySelector('[title="mon: 3"]')).not.toBeNull();
  expect(container.querySelector('[title="tue: 5"]')).not.toBeNull();
});

test("line viz nodes draw one chart per series", () => {
  const rows = [
    [1_000, 1, "a"],
    [2_000, 2, "a"],
    [1_000, 3, "b"],
    [2_000, 4, "b"],
  ];
  const { container } = render(
    <VizView
      run={run({ viz: "line", chart: { ...chart, x: "t", y: "v", series: "s" } }, ["t", "v", "s"], rows)}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelectorAll('[data-testid="viz-line-series"]').length).toBe(2);
  expect(container.textContent).toContain("a");
});

test("treemap viz nodes draw a tile per positive item and link through the viz's column link", () => {
  const opened: string[] = [];
  const { container } = render(
    <VizView
      run={run(
        {
          viz: "treemap",
          chart: { ...chart, label: "path", size: "churn", group: "zone" },
          columns: [{ key: "path", label: null, link: { kind: "file", from: null, line: null } }],
        },
        ["path", "churn", "zone"],
        [
          ["a.rs", 5, "core"],
          ["b.rs", 2, "ui"],
          ["c.rs", 0, "ui"],
        ],
      )}
      onOpenPage={(ref) => opened.push(ref.id)}
    />,
  );
  const tiles = container.querySelectorAll('[data-testid="viz-treemap-tile"]');
  expect(tiles.length).toBe(2);
  // Theme tokens, never inline hex (tsk371).
  for (const tile of tiles) {
    for (const el of tile.querySelectorAll("rect, text")) {
      for (const attr of ["fill", "stroke"]) {
        const v = el.getAttribute(attr);
        if (v !== null) expect(v).toMatch(/^var\(--[a-z0-9-]+\)$/);
      }
    }
  }
  (tiles[0] as SVGElement).dispatchEvent(new MouseEvent("click", { bubbles: true }));
  expect(opened).toEqual(["oxplow:oxplow/file?path=a.rs"]);
});

test("declared whole-viz actions are buttons, row actions aren't", () => {
  const finish = { id: "finish", label: "Finish", command: "oxplow.work_item.transition", input: {}, row: false };
  const perRow = { id: "row", label: "Per Row", command: "oxplow.work_item.transition", input: {}, row: true };
  const { getByTestId, queryByTestId } = render(
    <VizView
      run={run({ viz: "table", actions: [finish, perRow] }, ["id"], [[1]])}
      partButtons
      onOpenPage={() => {}}
    />,
  );
  expect(getByTestId("viz-action-finish").textContent).toBe("Finish");
  expect(queryByTestId("viz-action-row")).toBeNull();
  // The page host owns Copy and Add to Context: a viz has neither.
  expect(queryByTestId("viz-copy")).toBeNull();
  expect(queryByTestId("viz-add-to-context")).toBeNull();
  cleanup();
  // Only a page's node asks for them: without `partButtons` there are none.
  const plain = render(<VizView run={run({ viz: "table", actions: [finish] }, ["id"], [[1]])} onOpenPage={() => {}} />);
  expect(plain.queryByTestId("viz-action-finish")).toBeNull();
});

test("tree viz nodes nest rows and collapse a branch", () => {
  const { container, getByLabelText } = render(
    <VizView
      run={run(
        { viz: "tree", tree: { id: "id", parent: "p", label: "n" } },
        ["id", "p", "n"],
        [
          ["a", "", "root"],
          ["b", "a", "child"],
        ],
      )}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelectorAll('[data-testid="viz-tree-node"]').length).toBe(2);
  fireEvent.click(getByLabelText("Collapse"));
  expect(container.querySelectorAll('[data-testid="viz-tree-node"]').length).toBe(1);
});

test("timeline viz nodes list entries oldest first", () => {
  const { container } = render(
    <VizView
      run={run(
        { viz: "timeline", timeline: { at: "at", label: "w", uri: null } },
        ["at", "w"],
        [
          ["2026-09-30", "shipped"],
          ["2026-09-29", "started"],
        ],
      )}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelector('[data-testid="viz-timeline-entry-0"]')?.textContent).toContain("started");
});

test("an empty detail fact reads (none) and never links", () => {
  const link = { kind: "page", from: "target", line: null, base: null, head: null } as const;
  const { container } = render(
    <VizView
      run={{
        ...run({ viz: "detail" }, ["problems", "target"], [[null as unknown as string, "oxplow:oxplow/page?page=problems"]]),
        config: { ...base, viz: "detail", columns: [{ key: "problems", label: "Problems", link }] } as VizConfig,
      }}
      onOpenPage={() => {}}
    />,
  );
  const fact = container.querySelector('[data-testid="viz-detail"] dd');
  expect(fact?.textContent).toBe("(none)");
  expect(fact?.querySelector("a")).toBeNull();
});

test("detail and steps viz nodes", () => {
  const detail = render(
    <VizView run={run({ viz: "detail" }, ["title", "state"], [["Fix it", "done"]])} onOpenPage={() => {}} />,
  );
  expect(detail.container.querySelector('[data-testid="viz-detail"]')?.textContent).toContain("Fix it");
  cleanup();
  const steps = render(
    <VizView
      run={run({ viz: "steps", steps: { label: "s", status: "st" } }, ["s", "st"], [["plan", "done"], ["ship", "x"]])}
      onOpenPage={() => {}}
    />,
  );
  expect(steps.container.querySelector('[data-testid="viz-step-0"]')?.getAttribute("data-status")).toBe("done");
  expect(steps.container.querySelector('[data-testid="viz-step-1"]')?.getAttribute("data-status")).toBe("pending");
});

// Which branches are closed is the step's history state: Back and a restart
// show the tree as it was left.
test("a tree's closed branches are the step's history state", () => {
  const states: Record<string, unknown>[] = [];
  const nav = { viewParams: {}, patchViewParams() {}, historyState: { "viz-tree.closed": ["0"] }, setHistoryState: (p: Record<string, unknown>) => states.push(p) } as unknown as PageNavigation;
  const tree = run(
    { viz: "tree", tree: { id: "id", parent: "p", label: "n" } },
    ["id", "p", "n"],
    [
      ["a", "", "root"],
      ["b", "a", "child"],
    ],
  );
  const { container, getByLabelText } = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={tree} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  expect(container.querySelectorAll('[data-testid="viz-tree-node"]').length).toBe(1);
  fireEvent.click(getByLabelText("Expand"));
  expect(states).toEqual([{ "viz-tree.closed": [] }]);
});
