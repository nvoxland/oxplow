import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender } from "@testing-library/react";

import type { VizConfig, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A column with `format: items` holds a list of items (a JSON array): each
// drawn inline, separated by " · ", its `hint` the hover text, its `uri`
// a link. The same in a detail's facts, a sections viz's facts, a table's
// cells and a list's.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  refCommands: async () => [],
  listPersonCommands: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

afterEach(cleanup);

const opened: string[] = [];
const nav = { navigate: () => {}, openRef: (ref: string) => void opened.push(ref) } as unknown as PageNavigation;
const render = (run: VizRun) =>
  rtlRender(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={run} />
    </PageNavigationContext.Provider>,
  );

const items = JSON.stringify([
  "plain",
  { text: "sql.read", hint: "may run its own SQL over your data" },
  { text: "Docs", hint: "the guide", uri: "oxplow:oxplow/page?page=docs" },
]);
const columns = ["id", "name", "perms"];
const rowsOf = (): SqlCell[][] => [["a", "Alpha", items]];
const column = { key: "perms", label: "Permissions", link: null, unit: null, icon: null, tone: null, format: "items" };
const config = (viz: string, extra: Record<string, unknown> = {}) =>
  ({
    id: "x/p#all",
    extension: "x",
    slug: "all",
    title: "All",
    viz,
    params: [],
    columns: [column],
    empty: null,
    group: null,
    emphasis: null,
    depth: null,
    fills: false,
    actions: [],
    origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
    ...extra,
  }) as unknown as VizConfig;
const runOf = (viz: string, extra: Record<string, unknown> = {}, rows = rowsOf()): VizRun =>
  ({ config: config(viz, extra), params: {}, result: { columns, rows, truncated: false } }) as unknown as VizRun;

function check(root: HTMLElement) {
  const cell = root.querySelector<HTMLElement>("[data-testid=viz-items]")!;
  expect(cell).not.toBeNull();
  // Inline, separated by " · ".
  expect(cell.textContent).toBe("plain · sql.read · Docs");
  const parts = cell.querySelectorAll("[data-testid=viz-item]");
  expect(parts.length).toBe(3);
  // A hint is the hover text and the accessible description.
  expect(parts[0]!.getAttribute("title")).toBeNull();
  expect(parts[1]!.getAttribute("title")).toBe("may run its own SQL over your data");
  expect(parts[1]!.getAttribute("aria-description")).toBe("may run its own SQL over your data");
  // A uri makes the item a link.
  expect(parts[1]!.tagName).toBe("SPAN");
  expect(parts[2]!.tagName).toBe("BUTTON");
  expect(parts[2]!.getAttribute("title")).toBe("the guide");
  opened.length = 0;
  fireEvent.click(parts[2]!);
  expect(opened).toEqual(["oxplow:oxplow/page?page=docs"]);
}

test("a detail's facts draw an items column as inline items", () => {
  check(render(runOf("detail")).container);
});

test("a sections viz's facts draw an items column as inline items", () => {
  check(render(runOf("sections", { sections: { title: "name", id: "id", link: null, body: null } })).container);
});

test("a table's cells draw an items column as inline items", () => {
  check(render(runOf("table")).container);
});

test("a list's cells draw an items column as inline items", () => {
  check(render(runOf("list")).container);
});

test("a value that is not a list of items shows as it is; an empty list shows nothing", () => {
  const bad = render(runOf("detail", {}, [["a", "Alpha", "just text"]]));
  expect(bad.container.querySelector("[data-testid=viz-items]")).toBeNull();
  expect(bad.container.textContent).toContain("just text");
  cleanup();
  const empty = render(runOf("detail", {}, [["a", "Alpha", "[]"]]));
  expect(empty.container.querySelector("[data-testid=viz-item]")).toBeNull();
});
