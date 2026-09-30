import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { LensResultView } from "./LensResultView.js";

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
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  path: "",
};
const chart = { x: null, y: null, series: null, label: null, size: null, group: null };
const run = (lens: Partial<Lens>, columns: string[], rows: (string | number)[][]): LensRun => ({
  lens: { ...base, ...lens },
  params: {},
  result: { columns, rows, truncated: false },
});

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
  expect(opened).toEqual(["file:a.rs"]);
});

test("every lens has Copy and Add to Agent Context; declared actions are buttons, row actions aren't", () => {
  const finish = { id: "finish", label: "Finish", command: "work_item.transition", input: {}, row: false };
  const perRow = { id: "row", label: "Per Row", command: "work_item.transition", input: {}, row: true };
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
  // A compact strip and a grid's child have no toolbar.
  const compact = render(<LensResultView run={run({ viz: "number" }, ["n"], [[3]])} compact onOpenPage={() => {}} />);
  expect(compact.queryByTestId("lens-actions")).toBeNull();
  cleanup();
  const child = render(<LensResultView run={run({ viz: "table" }, ["n"], [[3]])} toolbar={false} onOpenPage={() => {}} />);
  expect(child.queryByTestId("lens-actions")).toBeNull();
});

test("a lens switching to and from grid keeps rendering (hook order)", () => {
  const view = render(<LensResultView run={run({ viz: "grid", children: [] }, [], [])} onOpenPage={() => {}} />);
  // The agent edits the lens's YAML from grid to table; the page re-runs it.
  view.rerender(<LensResultView run={run({ viz: "table" }, ["n"], [[1]])} onOpenPage={() => {}} />);
  expect(view.container.textContent).toContain("1");
  view.rerender(<LensResultView run={run({ viz: "grid", children: [] }, [], [])} onOpenPage={() => {}} />);
  // A hook-order error unmounts the tree (React reports it, render() doesn't
  // throw), so what shows is the proof.
  expect(view.container.querySelector('[data-testid="lens-grid"]')).not.toBeNull();
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
        { viz: "timeline", timeline: { at: "at", label: "w", ref: null } },
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
