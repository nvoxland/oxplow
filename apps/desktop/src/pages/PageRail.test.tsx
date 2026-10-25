import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageRail } from "./PageRail.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const page = fakePage(node.viz("items", "items"), {
  params: [
    { name: "uri", role: "resource", type: "text" },
    { name: "q", label: "Query", role: "filter", type: "text", default: null },
    { name: "picked", role: "selection", type: "text" },
  ],
  rail: node.stack("side", [node.viz("legend", "legend", { columns: [{ key: "k" }] })]),
});
const run = fakeRun(page, { params: { uri: "x", q: "old" }, loaders: { items: loader([], []), legend: loader(["k"], [["key-1"]]) } });

test("the rail lists the page's filters, never its resource or selection", () => {
  const { queryByTestId } = render(<PageRail run={run} runId="run-1" onApply={() => {}} />);
  expect(queryByTestId("lens-param-q")).not.toBeNull();
  expect(queryByTestId("lens-param-uri")).toBeNull();
  expect(queryByTestId("lens-param-picked")).toBeNull();
});

test("changing a filter applies it", () => {
  const applied: [string, SqlCell][] = [];
  const { getByTestId } = render(<PageRail run={run} runId="run-1" onApply={(n, v) => applied.push([n, v])} />);
  fireEvent.click(getByTestId("lens-param-q-value"));
  const input = getByTestId("lens-param-q-edit") as HTMLInputElement;
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
