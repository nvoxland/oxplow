import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageRail } from "./PageRail.js";
import { decl, fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import { PageReadsContext, type PageReads } from "./pageReads.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";

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

test("the page's rail tree draws below its filters", () => {
  const { container } = render(<PageRail run={run} runId="run-1" onApply={() => {}} />);
  const legend = container.querySelector('[data-node="legend"]');
  expect(legend?.textContent).toContain("key-1");
});

// A filter whose options another page lists: that page runs once, with
// the run's values it declares, and its named loader's `value`, `label`
// and `group` are the options.
test("a filter's options_from runs the page it names once and lists its loader's rows", async () => {
  const scopes = fakePage(node.viz("scopes", "scopes"), {
    id: "acme/scopes",
    params: [{ name: "stream_id", type: "text" } as never],
    loaders: [decl("first"), decl("scopes")],
  });
  const asked: unknown[][] = [];
  const reads: PageReads = {
    page: async (id) => {
      asked.push(["page", id]);
      return scopes;
    },
    run: async (id, params, streamId) => {
      asked.push(["run", id, params, streamId]);
      return fakeRun(scopes, {
        loaders: { first: loader(["value"], [["nope"]]), scopes: loader(["value", "label"], [["all", "All files"], ["mine", "Mine"]]) },
      });
    },
    text: async () => "",
  };
  const withScope = fakePage(node.viz("items", "items"), {
    params: [
      { name: "stream_id", type: "text" } as never,
      { name: "scope", role: "filter", type: "choice", options_from: "acme/scopes#scopes", default: "all", options: [] } as never,
    ],
  });
  const run = fakeRun(withScope, { params: { stream_id: "str1", scope: "all" }, loaders: { items: loader([], []) } });
  const { getByTestId } = render(
    <PageReadsContext.Provider value={reads}>
      <PageRail run={run} runId="run-1" onApply={() => {}} streamId="str1" />
    </PageReadsContext.Provider>,
  );
  await waitFor(() => expect(getByTestId("page-param-scope-value").textContent).toBe("All files"));
  expect(asked).toEqual([
    ["page", "acme/scopes"],
    ["run", "acme/scopes", { stream_id: "str1" }, "str1"],
  ]);
  fireEvent.click(getByTestId("page-param-scope-value"));
  const edit = getByTestId("page-param-scope-edit") as HTMLSelectElement;
  expect([...edit.options].map((o) => o.textContent)).toEqual(["All files", "Mine"]);
});
