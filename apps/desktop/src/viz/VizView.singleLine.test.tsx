import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A column's `badges` draws another column's labels as pills before its
// text; `single_line` keeps each of a table's rows to one line, a long cell
// cut with "…".

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
const rows: SqlCell[][] = [
  ["A long subject that would wrap", '["main",{"text":"v1.0","tone":"warning"}]', "Ann"],
  ["Plain", null, "Bob"],
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
      columns: [col("subject", { badges: "refs" }), col("author", { width: 120 })],
      actions: [],
      origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
      ...over,
    },
    params: {},
    result: { columns: ["subject", "refs", "author"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  }) as unknown as VizRun;

test("a column's badges are pills before its text, each in its tone, and the badges column isn't a cell", () => {
  const view = render(<VizView run={runOf()} />);
  const pills = [...view.getByTestId("viz-row-0").querySelectorAll<HTMLElement>("[data-testid=viz-cell-badge]")];
  expect(pills.map((p) => p.textContent)).toEqual(["main", "v1.0"]);
  // Before the text, so a cut subject never hides them.
  const text = view.getByTestId("viz-row-0").querySelector(".ox-cell-text")!;
  expect(pills.every((p) => p.compareDocumentPosition(text) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
  // Untoned, the accent; a tone, its own colour.
  expect(pills[0]!.style.color).toBe("var(--accent)");
  expect(pills[1]!.style.color).toBe("var(--status-waiting)");
  expect(view.getByTestId("viz-row-1").querySelectorAll("[data-testid=viz-cell-badge]").length).toBe(0);
  expect(view.container.querySelectorAll("thead th").length).toBe(2);
});

test("single_line cuts every cell to one line; without it, text wraps", () => {
  const cut = render(<VizView run={runOf({ singleLine: true })} />);
  const cells = [...cut.getByTestId("viz-row-0").querySelectorAll("td")];
  expect(cells.every((td) => td.classList.contains("ox-truncate") && td.classList.contains("ox-nowrap"))).toBe(true);
  // The text cuts, the pills after it stay whole.
  expect(cut.getByTestId("viz-row-0").querySelector(".ox-cell-text")).not.toBeNull();
  cleanup();
  const wraps = render(<VizView run={runOf()} />);
  expect(wraps.getByTestId("viz-row-0").querySelector("td")!.classList.contains("ox-truncate")).toBe(false);
});
