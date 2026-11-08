import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { RailSeeAlso } from "./railGroups.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";

afterEach(cleanup);

const page = fakePage(node.viz("items", "items"));
const links = [
  { title: "Collectors", uri: "oxplow:oxplow/page?page=oxplow/collectors", description: "Every collector." },
  { title: "Extension problems", uri: "oxplow:oxplow/page?page=oxplow/extension-problems#extension=acme", description: null },
];
const run = fakeRun(page, { loaders: { items: loader([], []) }, seeAlso: links });

test("See also is a group of links, each opening its uri, the description as its hover", () => {
  const opened: string[] = [];
  const { getByTestId, getByText } = render(<RailSeeAlso run={run} onOpenPage={(ref) => opened.push(ref.id)} />);
  expect(getByTestId("page-rail-see-also").getAttribute("aria-label")).toBe("See also");
  expect(getByText("Collectors").getAttribute("title")).toBe("Every collector.");
  expect(getByText("Extension problems").getAttribute("title")).toBeNull();
  fireEvent.click(getByText("Extension problems"));
  expect(opened).toEqual([links[1]!.uri]);
});

test("a page with no see-also links draws no group", () => {
  const { queryByTestId } = render(<RailSeeAlso run={fakeRun(page, { loaders: { items: loader([], []) } })} onOpenPage={() => {}} />);
  expect(queryByTestId("page-rail-see-also")).toBeNull();
});

const views = {
  options: [
    { name: "oxplow/measure", title: "Measure" },
    { name: "oxplow/measure-data", title: "Measure data" },
    { name: "oxplow/definition", title: "Definition" },
  ],
  current: "oxplow/measure",
  pick: () => {},
};

test("the other views of the tab's item follow the declared links, the current one left out", () => {
  const { getByTestId, getByText, queryByTestId } = render(<RailSeeAlso run={run} onOpenPage={() => {}} openWith={views} />);
  const labels = Array.from(getByTestId("page-rail-see-also").querySelectorAll("button")).map((b) => b.textContent);
  expect(labels).toEqual(["Collectors", "Extension problems", "View as Measure data", "View as Definition"]);
  expect(queryByTestId("see-also-view-oxplow/measure")).toBeNull();
  expect(getByText("View as Definition")).toBe(getByTestId("see-also-view-oxplow/definition"));
});

test("a view link picks that view as View as does", () => {
  const picked: string[] = [];
  const { getByTestId } = render(<RailSeeAlso run={run} onOpenPage={() => {}} openWith={{ ...views, pick: (n) => picked.push(n) }} />);
  fireEvent.click(getByTestId("see-also-view-oxplow/measure-data"));
  expect(picked).toEqual(["oxplow/measure-data"]);
});

test("view links alone make the group", () => {
  const { getByTestId } = render(<RailSeeAlso run={fakeRun(page, { loaders: { items: loader([], []) } })} openWith={views} />);
  expect(getByTestId("see-also-view-oxplow/definition")).toBeTruthy();
});

test("no view links with one view, none without View as", () => {
  const one = { options: [views.options[0]!], current: views.options[0]!.name, pick: () => {} };
  const bare = fakeRun(page, { loaders: { items: loader([], []) } });
  expect(render(<RailSeeAlso run={bare} openWith={one} />).queryByTestId("page-rail-see-also")).toBeNull();
  cleanup();
  expect(render(<RailSeeAlso run={bare} />).queryByTestId("page-rail-see-also")).toBeNull();
});
