import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { RailSeeAlso } from "./railGroups.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";

afterEach(cleanup);

const page = fakePage(node.viz("items", "items"));
const links = [
  { title: "Collectors", uri: "oxplow:oxplow/page?page=oxplow-foundation/collectors", description: "Every collector." },
  { title: "Extension problems", uri: "oxplow:oxplow/page?page=oxplow-foundation/extension-problems#extension=acme", description: null },
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
