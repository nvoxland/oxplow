import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { configValueResult, readsSettings } from "../configValuesTestSupport.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";

// A list, table or tree that is all of its page takes the page's height,
// as a file or a diff does; one among other nodes keeps its own.

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  subscribeOxplowEvents: () => () => {},
  querySql: async (sql: string) =>
    readsSettings(sql) ? configValueResult([]) : { columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} },
  recordUsage: async () => {},
}));
const { PageTree } = await import("./PageTree.js");

afterEach(cleanup);
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});

const rows = { rows: loader(["a"], [["x"]]) };
const fills = (el: HTMLElement) => el.style.flex === "1 1 0%" || el.style.flex === "1";

test("a table that is the whole page fills it", () => {
  const run = fakeRun(fakePage(node.viz("rows", "rows", { viz: "table", columns: [{ key: "a" }] })), { loaders: rows });
  const view = render(<PageTree run={run} runId="r" />);
  expect(fills(view.getByTestId("node-rows"))).toBe(true);
});

test("the only node of a stack fills; one of several does not", () => {
  const only = fakeRun(fakePage(node.stack("s", [node.viz("rows", "rows", { viz: "table", columns: [{ key: "a" }] }, { title: "Rows" })])), { loaders: rows });
  const a = render(<PageTree run={only} runId="r" />);
  expect(fills(a.getByTestId("node-rows"))).toBe(true);
  cleanup();
  const two = fakeRun(
    fakePage(
      node.stack("s", [
        node.viz("rows", "rows", { viz: "table", columns: [{ key: "a" }] }, { title: "Rows" }),
        node.viz("more", "rows", { viz: "table", columns: [{ key: "a" }] }, { title: "More" }),
      ]),
    ),
    { loaders: rows },
  );
  const b = render(<PageTree run={two} runId="r" />);
  expect(fills(b.getByTestId("node-rows"))).toBe(false);
});

// A node's `when` keeps a node out of the run: of two in a stack, the one
// the run shows is all of the page, and fills it.
test("the one node a run shows of a stack of two fills", () => {
  const page = fakePage(
    node.stack("s", [
      node.viz("rows", "rows", { viz: "table", columns: [{ key: "a" }] }, { title: "Rows" }),
      node.viz("more", "rows", { viz: "table", columns: [{ key: "a" }] }, { title: "More" }),
    ]),
  );
  const run = fakeRun(page, { loaders: rows, tree: { id: "s", children: [{ id: "more", children: [], embed: null, error: null, replacedBy: null, actionPoint: null }], embed: null, error: null, replacedBy: null, actionPoint: null } as never });
  const view = render(<PageTree run={run} runId="r" />);
  expect(fills(view.getByTestId("node-more"))).toBe(true);
});

// A custom component sizes as it declares (the run carries `sizing`): a
// lone `content` one is as tall as what it draws, and the page scrolls; a
// `fill` one takes the page's height (the run marks its viz `fills`).
test("a lone custom viz fills only when its component fills", () => {
  const custom = (fills: boolean) =>
    fakeRun(fakePage(node.viz("c", "rows", { viz: "custom", custom: { component: "c", props: null }, sizing: fills ? "fill" : "content", fills } as never)), { loaders: rows });
  const content = render(<PageTree run={custom(false)} runId="r" />);
  expect(fills(content.getByTestId("node-c"))).toBe(false);
  cleanup();
  const fill = render(<PageTree run={custom(true)} runId="r" />);
  expect(fills(fill.getByTestId("node-c"))).toBe(true);
});
