import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { configValueResult, readsSettings } from "../configValuesTestSupport.js";
import { decl, fakePage, fakePageRuns, fakeRun, loader, node } from "./testing/fakeRun.js";

// A page file's node marked `replaceable: <target>` is a viz part's
// equal: oxplow's own (the node) unless an extension replaces the target.

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
let extensions: unknown[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  subscribeOxplowEvents: () => () => {},
  querySql: async (sql: string) =>
    readsSettings(sql) ? configValueResult([]) : { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
  recordUsage: async () => {},
}));
const { PageTree } = await import("./PageTree.js");

afterEach(() => {
  cleanup();
  extensions = [];
});
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});

const run = () => {
  const page = fakePage({ ...node.viz("cards", "cards", { columns: [{ key: "title" } as never] }), replaceable: "work_item.board" } as never, { loaders: [decl("cards")] });
  return fakeRun(page, { params: { scope: "all" }, loaders: { cards: loader(["title"], [["our card"]]) } });
};

test("with nothing replacing it, the node itself shows", async () => {
  extensions = [{ name: "oxplow-foundation", enabled: true, pages: [], ui: { slots: [], commands: [], decorators: [], replacements: [] } }];
  const view = render(<PageTree run={run()} runId="run-1" />);
  await waitFor(() => expect(view.container.textContent).toContain("our card"));
});

test("an extension's page replaces it, run with the page's params", async () => {
  extensions = [
    {
      name: "x",
      enabled: true,
      pages: [],
      pages: [],
      ui: {
        slots: [],
        commands: [],
        decorators: [],
        replacements: [{ id: "x/work_item.board", extension: "x", target: "work_item.board", capability: "work_items", boundToProvider: false, page: "x/board", label: "board", version: 1, view: [] }],
      },
    },
  ];
  // The replacement runs on the window's page runs; the node's own tree
  // runs nothing new.
  const theirs = fakeRun(fakePage(node.viz("theirs", "theirs", { columns: [{ key: "t" } as never] }), { id: "x/board", loaders: [decl("theirs")] }), {
    loaders: { theirs: loader(["t"], [["their card"]]) },
  });
  const { PageRunsContext } = await import("../pageRuns.js");
  const fake = fakePageRuns(theirs);
  const view = render(
    <PageRunsContext.Provider value={fake.runs}>
      <PageTree run={run()} runId="run-1" />
    </PageRunsContext.Provider>,
  );
  await waitFor(() => expect(view.getByTestId("replacement-work_item.board").textContent).toContain("their card"));
  expect(view.container.textContent).not.toContain("our card");
  expect(fake.calls.open).toEqual([{ page: "x/board", params: { scope: "all" }, streamId: null, threadId: null }]);
});

test("a prop the page's params don't hold comes from its loaders' first row, as an embed's does", async () => {
  extensions = [
    {
      name: "x",
      enabled: true,
      pages: [],
      pageFiles: [{ id: "x/board", params: [{ name: "scope" }, { name: "show" }] }],
      ui: {
        slots: [],
        commands: [],
        decorators: [],
        replacements: [{ id: "x/work_item.board", extension: "x", target: "work_item.board", capability: "work_items", boundToProvider: false, page: "x/board", label: "board", version: 1, view: [] }],
      },
    },
  ];
  const theirs = fakeRun(fakePage(node.viz("theirs", "theirs", { columns: [{ key: "t" } as never] }), { id: "x/board", loaders: [decl("theirs")] }), {
    loaders: { theirs: loader(["t"], [["their card"]]) },
  });
  const { PageRunsContext } = await import("../pageRuns.js");
  const fake = fakePageRuns(theirs);
  // The page holds `show`; its `row` loader derives the contract's `scope`.
  const page = fakePage({ ...node.viz("cards", "cards", { columns: [{ key: "title" } as never] }), replaceable: "work_item.board" } as never, {
    loaders: [decl("row"), decl("cards")],
  });
  const hosted = fakeRun(page, {
    params: { show: "stream", scope: null },
    loaders: { row: loader(["scope"], [["stream"]]), cards: loader(["title"], [["our card"]]) },
  });
  const view = render(
    <PageRunsContext.Provider value={fake.runs}>
      <PageTree run={hosted} runId="run-1" />
    </PageRunsContext.Provider>,
  );
  await waitFor(() => expect(view.getByTestId("replacement-work_item.board").textContent).toContain("their card"));
  expect(fake.calls.open).toEqual([{ page: "x/board", params: { show: "stream", scope: "stream" }, streamId: null, threadId: null }]);
});
