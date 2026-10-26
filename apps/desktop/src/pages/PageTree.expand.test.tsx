import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import { PageTree } from "./PageTree.js";
import { PageActionContext, type RunPageAction } from "./useNodeActions.js";
import { PageReadsContext, type PageReads } from "./pageReads.js";
import { decl, fakeAction, fakePage, fakePageRunsBy, fakeRun, loader, node } from "./testing/fakeRun.js";
import { PageRunsContext, type PageKey } from "../pageRuns.js";
import type { Page, PageRun } from "../tauri-bridge/generated/bindings.js";

// In a page file an `expand` column and a lazy tree name *pages*: a row
// opens `<ext>/<page>[#<loader>]` as a run of its own, given the row's
// values it declares; a lazy branch runs the node's page again with the
// lazy param set to the branch's id. Both draw with the same viz
// components, and a branch's row presses on the branch's run.

afterEach(cleanup);

const detail = fakePage(node.stack("stack", [node.viz("summary", "summary", { viz: "number" }), node.viz("notes", "notes", { columns: [{ key: "note" }] })]), {
  id: "acme/detail",
  params: [{ name: "id", role: "resource", default: null, type: null, options: [] } as never],
  loaders: [decl("summary"), decl("notes")],
});

const reads = (pages: Record<string, Page>): PageReads => ({
  page: async (id) => pages[id] ?? Promise.reject(new Error(`no page ${id}`)),
  run: async () => Promise.reject(new Error("not here")),
  text: async () => "",
});

test("an expand column's row opens the page it names, given the row's values it declares, and draws the named loader", async () => {
  const page = fakePage(node.viz("items", "items", { columns: [{ key: "id", expand: "acme/detail#notes" } as never, { key: "title" }] }), {
    loaders: [decl("items")],
  });
  const run = fakeRun(page, { loaders: { items: loader(["id", "title"], [["a", "Alpha"]]) } });
  const { runs, opened } = fakePageRunsBy((key: PageKey) =>
    fakeRun(detail, { params: key.params, loaders: { summary: loader(["n"], [[9]]), notes: loader(["note"], [[`note for ${key.params.id}`]]) } }),
  );
  const { getByLabelText, findByText, container } = render(
    <PageRunsContext.Provider value={runs}>
      <PageReadsContext.Provider value={reads({ "acme/detail": detail })}>
        <PageTree run={run} runId="run-0" streamId="str1" />
      </PageReadsContext.Provider>
    </PageRunsContext.Provider>,
  );
  fireEvent.click(getByLabelText("Expand"));
  await findByText("note for a");
  // Only the params the page declares; only the named loader's node.
  expect(opened.map((o) => o.key)).toEqual([{ page: "acme/detail", params: { id: "a" }, streamId: "str1", threadId: null }]);
  expect(container.querySelector('[data-node="summary"]')).toBeNull();
});

test("an expand that names no loader draws the page's first", async () => {
  const page = fakePage(node.viz("items", "items", { columns: [{ key: "id", expand: "acme/detail" } as never] }), { loaders: [decl("items")] });
  const run = fakeRun(page, { loaders: { items: loader(["id"], [["a"]]) } });
  const { runs } = fakePageRunsBy((key) => fakeRun(detail, { params: key.params, loaders: { summary: loader(["n"], [[42]]), notes: loader(["note"], [["x"]]) } }));
  const { getByLabelText, findByText } = render(
    <PageRunsContext.Provider value={runs}>
      <PageReadsContext.Provider value={reads({ "acme/detail": detail })}>
        <PageTree run={run} runId="run-0" />
      </PageReadsContext.Provider>
    </PageRunsContext.Provider>,
  );
  fireEvent.click(getByLabelText("Expand"));
  await findByText("42");
});

test("an expanded row's page offers its viz's own actions as buttons, pressed on that page's run", async () => {
  const backfill = fakePage(node.viz("plan", "plan", { viz: "detail", columns: [{ key: "what" }], actions: [fakeAction({ id: "backfill", label: "Backfill" })] }), {
    id: "acme/backfill",
    params: [{ name: "id", role: "resource", default: null, type: null, options: [] } as never],
    loaders: [decl("plan")],
  });
  const page = fakePage(node.viz("items", "items", { columns: [{ key: "id", expand: "acme/backfill" } as never] }), { loaders: [decl("items")] });
  const run = fakeRun(page, { loaders: { items: loader(["id"], [["a"]]) } });
  const { runs } = fakePageRunsBy((key) => fakeRun(backfill, { params: key.params, loaders: { plan: loader(["what"], [["3 to catch up"]]) } }));
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(args);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  const { getByLabelText, findByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageRunsContext.Provider value={runs}>
        <PageReadsContext.Provider value={reads({ "acme/backfill": backfill })}>
          <PageTree run={run} runId="run-0" />
        </PageReadsContext.Provider>
      </PageRunsContext.Provider>
    </PageActionContext.Provider>,
  );
  fireEvent.click(getByLabelText("Expand"));
  fireEvent.click(await findByTestId("lens-action-backfill"));
  await waitFor(() => expect(calls).toEqual([["run-1", "plan", "backfill", null, null, null, false]]));
});

function filesPage(folder: string, rows: (string | number)[][]): PageRun {
  const page = fakePage(
    node.viz("tree", "tree", {
      viz: "tree",
      tree: { id: "id", parent: "parent", label: "name", lazy: "folder", branch: "opens" } as never,
      columns: [{ key: "name" }],
      actions: [fakeAction({ id: "rename", label: "Rename", row: true })],
    }),
    {
      id: "acme/files",
      params: [{ name: "folder", role: "resource", default: "", type: null, options: [] } as never],
      loaders: [decl("tree")],
    },
  );
  return fakeRun(page, { params: { folder }, loaders: { tree: loader(["id", "parent", "name", "opens"], rows) } });
}

test("a lazy branch runs the node's page with the lazy param set to its id; its rows press on that run", async () => {
  const root = filesPage("", [
    ["src", "", "src", 1],
    ["a.rs", "", "a.rs", 0],
  ]);
  const { runs, opened } = fakePageRunsBy((key) =>
    filesPage(String(key.params.folder), [
      ["src/lib.rs", "src", "lib.rs", 0],
    ]),
  );
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(args);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  const view = render(
    <PageActionContext.Provider value={press}>
      <PageRunsContext.Provider value={runs}>
        <PageTree run={root} runId="run-0" streamId="str1" />
      </PageRunsContext.Provider>
    </PageActionContext.Provider>,
  );
  fireEvent.click(view.getAllByLabelText("Expand")[0]!);
  await view.findByText("lib.rs");
  expect(opened.map((o) => o.key)).toEqual([{ page: "acme/files", params: { folder: "src" }, streamId: "str1", threadId: null }]);
  // A branch row's action presses on the branch's run; a root row's on the root's.
  const rows = view.getAllByTestId("lens-tree-row");
  const lib = rows.find((r) => r.textContent?.includes("lib.rs"))!;
  fireEvent.contextMenu(lib);
  fireEvent.click(within(document.body).getByTestId("menu-item-lens-action-rename"));
  await waitFor(() => expect(calls.length).toBe(1));
  expect(calls[0]!.slice(0, 4)).toEqual(["run-1", "tree", "rename", { id: "src/lib.rs", parent: "src", name: "lib.rs", opens: 0 }]);
  const a = rows.find((r) => r.textContent?.includes("a.rs"))!;
  fireEvent.contextMenu(a);
  fireEvent.click(within(document.body).getByTestId("menu-item-lens-action-rename"));
  await waitFor(() => expect(calls.length).toBe(2));
  expect(calls[1]!.slice(0, 2)).toEqual(["run-0", "tree"]);
});
