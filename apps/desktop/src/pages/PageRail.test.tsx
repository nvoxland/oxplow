import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageRail } from "./PageRail.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import { PageReadsContext, type PageReads } from "./pageReads.js";
import type { SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const page = fakePage(node.viz("items", "items"), {
  params: [
    { name: "uri", role: "resource", type: "text" },
    // No role: a resource, as the server reads it.
    { name: "path", type: "text" },
    { name: "q", label: "Query", role: "filter", type: "text", default: null },
    { name: "picked", role: "selection", type: "text" },
  ],
  rail: node.stack("side", [node.viz("legend", "legend", { columns: [{ key: "k" }] })]),
});
const run = fakeRun(page, { params: { uri: "x", q: "old" }, loaders: { items: loader([], []), legend: loader(["k"], [["key-1"]]) } });

test("the rail lists the page's filters, never its resource or selection", () => {
  const { queryByTestId } = render(<PageRail run={run} runId="run-1" onApply={() => {}} />);
  expect(queryByTestId("page-param-q")).not.toBeNull();
  expect(queryByTestId("page-param-uri")).toBeNull();
  expect(queryByTestId("page-param-path")).toBeNull();
  expect(queryByTestId("page-param-picked")).toBeNull();
});

test("a page whose only non-resource param is a view param has no Filters group", () => {
  const viewPage = fakePage(node.viz("items", "items"), {
    params: [
      { name: "uri", role: "resource", type: "text" },
      { name: "mode", role: "view", type: "text", default: "a" },
    ],
  });
  const viewRun = fakeRun(viewPage, { params: { uri: "x", mode: "b" }, loaders: { items: loader([], []) } });
  const { queryByTestId, queryByText } = render(<PageRail run={viewRun} runId="run-v" onApply={() => {}} />);
  expect(queryByTestId("page-param-mode")).toBeNull();
  expect(queryByTestId("action-page-rail-reset")).toBeNull();
  expect(queryByText("Filters")).toBeNull();
});

test("changing a filter applies it", () => {
  const applied: [string, SqlCell][] = [];
  const { getByTestId } = render(<PageRail run={run} runId="run-1" onApply={(n, v) => applied.push([n, v])} />);
  fireEvent.click(getByTestId("page-param-q-value"));
  const input = getByTestId("page-param-q-edit") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "new" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(applied).toEqual([["q", "new"]]);
});

test("Reset puts a changed filter back to its default", () => {
  const applied: [string, SqlCell][] = [];
  const { getByTestId } = render(<PageRail run={run} runId="run-1" onApply={(n, v) => applied.push([n, v])} />);
  fireEvent.click(getByTestId("action-page-rail-reset"));
  expect(applied).toEqual([["q", null]]);
});

test("the page's rail tree draws first, under its title as a group, then its filters", () => {
  const about = fakePage(node.viz("items", "items"), {
    params: [{ name: "q", label: "Query", role: "filter", type: "text", default: null }],
    rail: node.viz("about", "about", { columns: [{ key: "branch", label: "Branch" }] }, { title: "About" }),
  });
  const aboutRun = fakeRun(about, { params: { q: null }, loaders: { items: loader([], []), about: loader(["branch"], [["main"]]) } });
  const { container } = render(<PageRail run={aboutRun} runId="run-1" onApply={() => {}} />);
  const groups = [...container.querySelectorAll(".ox-rail-group")].map((g) => g.getAttribute("aria-label"));
  expect(groups).toEqual(["About", "Filters"]);
  expect(container.querySelector('[data-testid="page-rail-tree"] [data-node="about"]')?.textContent).toContain("main");
});

test("a rail tree without a title draws bare, with no group heading", () => {
  const { container } = render(<PageRail run={run} runId="run-1" onApply={() => {}} />);
  const legend = container.querySelector('[data-node="legend"]');
  expect(legend?.textContent).toContain("key-1");
  expect([...container.querySelectorAll(".ox-rail-group")].map((g) => g.getAttribute("aria-label"))).toEqual(["Filters", "Selection"]);
});

// A filter whose options a query (or the page's own loader) lists: the
// daemon resolves it for this page and param with the run's values, and
// the rows' `value`, `label` and `group` are the options.
test("a filter's options_from is resolved by the daemon for the page and param, listing its rows", async () => {
  const asked: unknown[][] = [];
  const reads: PageReads = {
    page: async () => {
      throw new Error("not read");
    },
    run: async () => {
      throw new Error("not run");
    },
    text: async () => "",
    options: async (page, param, values, streamId) => {
      asked.push(["options", page, param, values, streamId]);
      return loader(["value", "label"], [["all", "All files"], ["mine", "Mine"]]).result as SqlQueryResult;
    },
  };
  const withScope = fakePage(node.viz("items", "items"), {
    id: "acme/files",
    params: [
      { name: "stream_id", type: "text" } as never,
      { name: "scope", role: "filter", type: "choice", options_from: "acme/scopes", default: "all", options: [] } as never,
    ],
  });
  const run = fakeRun(withScope, { params: { stream_id: "str1", scope: "all" }, loaders: { items: loader([], []) } });
  const { getByTestId } = render(
    <PageReadsContext.Provider value={reads}>
      <PageRail run={run} runId="run-1" onApply={() => {}} streamId="str1" />
    </PageReadsContext.Provider>,
  );
  await waitFor(() => expect(getByTestId("page-param-scope-value").textContent).toBe("All files"));
  expect(asked).toEqual([["options", "acme/files", "scope", { stream_id: "str1", scope: "all" }, "str1"]]);
  fireEvent.click(getByTestId("page-param-scope-value"));
  const edit = getByTestId("page-param-scope-edit") as HTMLSelectElement;
  expect([...edit.options].map((o) => o.textContent)).toEqual(["All files", "Mine"]);
});
