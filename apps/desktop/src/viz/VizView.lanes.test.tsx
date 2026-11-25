import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A table's `format: lanes` column draws each row's part of the graph the
// rows form, beside the row's other cells; `reveal` marks a row and brings
// it into view.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

afterEach(cleanup);
afterAll(() => {
  mock.module("../api.js", () => realApi);
});

const col = (key: string, over: Record<string, unknown> = {}) => ({ key, label: null, link: null, unit: null, icon: null, tone: null, ...over });
const columns = ["sha", "parents", "subject", "shown"];
const rows: SqlCell[][] = [
  ["d", '["c","b"]', "Merge side", 0],
  ["c", '["a"]', "Main again", 0],
  ["b", '["a"]', "Side work", 1],
  ["a", "[]", "Main work", 0],
];
const runOf = (over: Record<string, unknown> = {}): VizRun =>
  ({
    config: {
      id: "x/history",
      extension: "x",
      slug: "history",
      title: "History",
      viz: "table",
      params: [],
      columns: [col("sha", { format: "lanes", parents: "parents" }), col("subject")],
      actions: [],
      origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
      ...over,
    },
    params: {},
    result: { columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  }) as unknown as VizRun;

test("a lanes column draws each row's lanes, the dot in its lane, as wide as the widest row", () => {
  const view = render(<VizView run={runOf()} />);
  const cells = view.getAllByTestId("viz-lanes");
  expect(cells.length).toBe(4);
  const dot = (i: number) => cells[i]!.querySelector("circle")!.getAttribute("data-lane");
  expect([0, 1, 2, 3].map(dot)).toEqual(["0", "0", "1", "0"]);
  // Two lanes wide on every row, so the column lines up.
  expect(new Set(cells.map((c) => c.getAttribute("width"))).size).toBe(1);
  // The cell has no padding: its lines run the row's full height.
  expect((cells[0]!.closest("td") as HTMLElement).style.padding).toBe("0px");
  // The id itself isn't drawn as text.
  expect(view.container.textContent).not.toContain('["c","b"]');
  expect(view.getByText("Merge side")).toBeDefined();
});

test("reveal marks the row whose column is truthy and brings it into view", () => {
  const scrolled: string[] = [];
  const original = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = function (this: HTMLElement) {
    scrolled.push(this.getAttribute("data-testid") ?? "");
  };
  try {
    const view = render(<VizView run={runOf({ reveal: "shown" })} />);
    const row = view.getByTestId("viz-row-2");
    expect(row.className).toContain("ox-emphasis");
    expect(view.getByTestId("viz-row-1").className ?? "").not.toContain("ox-emphasis");
    expect(scrolled).toEqual(["viz-row-2"]);
    // The column marks; it isn't a cell.
    expect(view.container.textContent).not.toContain("Side work1");
  } finally {
    HTMLElement.prototype.scrollIntoView = original;
  }
});
