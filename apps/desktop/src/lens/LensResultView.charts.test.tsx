import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A lazy tree's branches run the lens again (`runLens`) with the lazy
// param set to the branch's id.
const branchRuns: Record<string, SqlCell>[] = [];
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  runLens: async (_id: string, params: Record<string, SqlCell>) => {
    branchRuns.push(params);
    const folder = String(params.folder ?? "");
    return {
      lens: lazyTree.lens,
      params,
      result: { columns: ["id", "p", "n", "d"], rows: [[`${folder}/lib`, folder, "lib", 1], [`${folder}/main.rs`, folder, "main.rs", 0]], truncated: false },
    } as unknown as LensRun;
  },
}));
const { LensResultView } = await import("./LensResultView.js");
beforeEach(() => {
  branchRuns.length = 0;
});

afterEach(cleanup);

const base: Lens = {
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
const run = (lens: Partial<Lens>, columns: string[], rows: (string | number)[][]): LensRun => ({
  lens: { ...base, ...lens },
  params: {},
  result: { columns, rows, truncated: false },
});
const lazyTree = run(
  {
    viz: "tree",
    tree: { id: "id", parent: "p", label: "n", lazy: "folder", branch: "d" },
    params: [{ name: "folder", label: null, default: "", options: [] }] as never,
    actions: [{ id: "open", label: "Open", command: "oxplow.file.open", input: { path: "{{row.id}}" }, row: true }] as never,
  },
  ["id", "p", "n", "d"],
  [
    ["src", "", "src", 1],
    ["a.rs", "", "a.rs", 0],
  ],
);

test("bar lenses draw one bar per row", () => {
  const { container } = render(
    <LensResultView
      run={run({ viz: "bar", chart: { ...chart, x: "day", y: "n" } }, ["day", "n"], [["mon", 3], ["tue", 5]])}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelector('[data-testid="lens-bar"]')).not.toBeNull();
  expect(container.querySelector('[title="mon: 3"]')).not.toBeNull();
  expect(container.querySelector('[title="tue: 5"]')).not.toBeNull();
});

test("line lenses draw one chart per series", () => {
  const rows = [
    [1_000, 1, "a"],
    [2_000, 2, "a"],
    [1_000, 3, "b"],
    [2_000, 4, "b"],
  ];
  const { container } = render(
    <LensResultView
      run={run({ viz: "line", chart: { ...chart, x: "t", y: "v", series: "s" } }, ["t", "v", "s"], rows)}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelectorAll('[data-testid="lens-line-series"]').length).toBe(2);
  expect(container.textContent).toContain("a");
});

test("treemap lenses draw a tile per positive item and link through the lens's column link", () => {
  const opened: string[] = [];
  const { container } = render(
    <LensResultView
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
  const tiles = container.querySelectorAll('[data-testid="lens-treemap-tile"]');
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

test("every lens has Copy and Add to Context; declared actions are buttons, row actions aren't", () => {
  const finish = { id: "finish", label: "Finish", command: "oxplow.work_item.transition", input: {}, row: false };
  const perRow = { id: "row", label: "Per Row", command: "oxplow.work_item.transition", input: {}, row: true };
  const { getByTestId, queryByTestId } = render(
    <LensResultView
      run={run({ viz: "table", actions: [finish, perRow] }, ["id"], [[1]])}
      onOpenPage={() => {}}
    />,
  );
  expect(getByTestId("lens-copy").textContent).toBe("Copy");
  expect(getByTestId("lens-add-to-context")).not.toBeNull();
  expect(getByTestId("lens-action-finish").textContent).toBe("Finish");
  expect(queryByTestId("lens-action-row")).toBeNull();
  cleanup();
  // A compact strip and a document's section have no toolbar.
  const compact = render(<LensResultView run={run({ viz: "number" }, ["n"], [[3]])} compact onOpenPage={() => {}} />);
  expect(compact.queryByTestId("lens-actions")).toBeNull();
  cleanup();
  const child = render(<LensResultView run={run({ viz: "table" }, ["n"], [[3]])} toolbar={false} onOpenPage={() => {}} />);
  expect(child.queryByTestId("lens-actions")).toBeNull();
});

test("a lens switching to and from document keeps rendering (hook order)", () => {
  const view = render(<LensResultView run={run({ viz: "document", parts: [] }, [], [])} onOpenPage={() => {}} />);
  // The agent edits the lens's YAML from document to table; the page re-runs it.
  view.rerender(<LensResultView run={run({ viz: "table" }, ["n"], [[1]])} onOpenPage={() => {}} />);
  expect(view.container.textContent).toContain("1");
  view.rerender(<LensResultView run={run({ viz: "document", parts: [] }, [], [])} onOpenPage={() => {}} />);
  // A hook-order error unmounts the tree (React reports it, render() doesn't
  // throw), so what shows is the proof.
  expect(view.container.querySelector('[data-testid="lens-document"]')).not.toBeNull();
});

test("tree lenses nest rows and collapse a branch", () => {
  const { container, getByLabelText } = render(
    <LensResultView
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
  expect(container.querySelectorAll('[data-testid="lens-tree-node"]').length).toBe(2);
  fireEvent.click(getByLabelText("Collapse"));
  expect(container.querySelectorAll('[data-testid="lens-tree-node"]').length).toBe(1);
});

test("timeline lenses list entries oldest first", () => {
  const { container } = render(
    <LensResultView
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
  expect(container.querySelector('[data-testid="lens-timeline-entry-0"]')?.textContent).toContain("started");
});

test("detail and steps lenses", () => {
  const detail = render(
    <LensResultView run={run({ viz: "detail" }, ["title", "state"], [["Fix it", "done"]])} onOpenPage={() => {}} />,
  );
  expect(detail.container.querySelector('[data-testid="lens-detail"]')?.textContent).toContain("Fix it");
  cleanup();
  const steps = render(
    <LensResultView
      run={run({ viz: "steps", steps: { label: "s", status: "st" } }, ["s", "st"], [["plan", "done"], ["ship", "x"]])}
      onOpenPage={() => {}}
    />,
  );
  expect(steps.container.querySelector('[data-testid="lens-step-0"]')?.getAttribute("data-status")).toBe("done");
  expect(steps.container.querySelector('[data-testid="lens-step-1"]')?.getAttribute("data-status")).toBe("pending");
});

// Which branches are closed is the step's history state: Back and a restart
// show the tree as it was left.
test("a tree's closed branches are the step's history state", () => {
  const states: Record<string, unknown>[] = [];
  const nav = { viewParams: {}, patchViewParams() {}, historyState: { "lens-tree.closed": ["0"] }, setHistoryState: (p: Record<string, unknown>) => states.push(p) } as unknown as PageNavigation;
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
      <LensResultView run={tree} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  expect(container.querySelectorAll('[data-testid="lens-tree-node"]').length).toBe(1);
  fireEvent.click(getByLabelText("Expand"));
  expect(states).toEqual([{ "lens-tree.closed": [] }]);
});

// A lazy tree (`tree.lazy`) shows one level, its folders closed; opening
// one runs the lens with the lazy param set to the folder's id and
// splices its rows under it, and the open folders are the step's history
// state. A spliced row is a row like any other: its menu, its marks.
test("a lazy tree opens a branch by running its lens for it", async () => {
  const view = render(<LensResultView run={lazyTree} onOpenPage={() => {}} />);
  expect(view.getAllByTestId("lens-tree-node").length).toBe(2);
  expect(view.getAllByLabelText("Expand").length).toBe(1);
  expect(branchRuns).toEqual([]);
  // A click on the folder's row opens it (a leaf keeps its own click).
  fireEvent.click(view.getAllByTestId("lens-tree-row")[0]!);
  await waitFor(() => expect(view.getAllByTestId("lens-tree-node").length).toBe(4));
  expect(branchRuns).toEqual([{ folder: "src" }]);
  expect(view.getAllByTestId("lens-tree-row").map((r) => r.textContent)).toEqual(["▾src", "▸lib", "main.rs", "a.rs"]);
  // Enter on the nested folder opens it too; its run names its own path.
  fireEvent.keyDown(view.getAllByTestId("lens-tree-row")[1]!, { key: "Enter" });
  await waitFor(() => expect(branchRuns).toHaveLength(2));
  expect(branchRuns[1]).toEqual({ folder: "src/lib" });
  // A spliced row has the row actions in its menu, and marks like any row.
  fireEvent.contextMenu(view.getAllByTestId("lens-tree-row")[2]!);
  expect(view.getByTestId("menu-item-lens-action-open")).toBeTruthy();
  fireEvent.click(view.getAllByTestId("lens-tree-row")[2]!, { metaKey: true });
  expect(view.getByTestId("lens-marks-count").textContent).toBe("1 selected");
});

test("a lazy tree's open folders are the step's history state", async () => {
  const states: Record<string, unknown>[] = [];
  const nav = {
    viewParams: {},
    patchViewParams() {},
    historyState: { "lens-tree.open:x/l": ["src"] },
    setHistoryState: (p: Record<string, unknown>) => states.push(p),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView run={lazyTree} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(view.getAllByTestId("lens-tree-node").length).toBe(4));
  expect(branchRuns).toEqual([{ folder: "src" }]);
  fireEvent.click(view.getByLabelText("Collapse"));
  expect(states).toEqual([{ "lens-tree.open:x/l": [] }]);
});
