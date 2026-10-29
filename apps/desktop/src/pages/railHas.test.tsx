import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { PageHost } from "./PageHost.js";
import { contentsHas, railHas } from "./railHas.js";
import { decl, fakePage, fakePageRuns, fakeRun, loader, node } from "./testing/fakeRun.js";
import type { PageKey } from "../pageRuns.js";
import type { Node, PageParam } from "../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const key: PageKey = { page: "acme/page", params: {}, streamId: null, threadId: null };
const items = loader(["id"], [["a"]]);
const cols = { columns: [{ key: "id" }] };
const filter = { name: "q", role: "filter", type: "text", default: null } as PageParam;
const selection = { name: "picked", role: "selection", type: "text" } as PageParam;

const run = (view: Node, params: PageParam[] = [], rail: Node | null = null) =>
  fakeRun(fakePage(view, { params, rail, loaders: [decl("items")] }), { loaders: { items, other: items } });
const sections = (n: number) => node.stack("s", Array.from({ length: n }, (_, i) => node.viz(`v${i}`, "items", cols, { title: `Section ${i}` })));
const withSections = (rows: number, pad = 0) =>
  fakeRun(
    fakePage(
      node.stack("s", [
        node.viz("pages", "many", { viz: "sections", columns: [{ key: "id" }], sections: { title: "id" } } as never, { title: "Pages" }),
        ...Array.from({ length: pad }, (_, i) => node.viz(`v${i}`, "items", cols, { title: `Other ${i}` })),
      ]),
      { loaders: [decl("items"), decl("many")] },
    ),
    { loaders: { items, many: loader(["id"], Array.from({ length: rows }, (_, i) => [`r${i}`])) } },
  );
const filling = () => node.viz("file", "items", { ...cols, fills: true });
const no = { fills: false, saveOpen: false };

test("railHas: nothing in the rail for a page with no filter, selection, rail node, switch or sections", () => {
  expect(railHas(run(node.viz("items", "items", cols)), no)).toBe(false);
  expect(railHas(run(sections(1)), no)).toBe(false);
});

test("railHas: each of a filter, a selection, a rail node, a switch and the save form is something", () => {
  const plain = node.viz("items", "items", cols);
  expect(railHas(run(plain, [filter]), no)).toBe(true);
  expect(railHas(run(plain, [selection]), no)).toBe(true);
  expect(railHas(run(plain, [], node.viz("legend", "items", cols)), no)).toBe(true);
  expect(railHas(run(node.switch("m", "mode", [plain])), no)).toBe(true);
  expect(railHas(run(plain), { fills: false, saveOpen: true })).toBe(true);
});

test("railHas: a view param is never drawn, so it is not something; a resource param neither", () => {
  const plain = node.viz("items", "items", cols);
  expect(railHas(run(plain, [{ name: "lines", role: "view", type: "text" } as PageParam, { name: "uri", role: "resource", type: "text" } as PageParam]), no)).toBe(false);
});

test("contentsHas: two sections list, one does not, a page that fills never does", () => {
  expect(contentsHas(run(sections(2)), false)).toBe(true);
  expect(contentsHas(run(sections(1)), false)).toBe(false);
  expect(contentsHas(run(sections(4)), true)).toBe(false);
  expect(railHas(run(filling()), { fills: true, saveOpen: false })).toBe(false);
});

test("contentsHas: markdown with a heading lists; markdown without does not", () => {
  expect(contentsHas(run(node.markdown("md", "## Hi\n\ntext")), false)).toBe(true);
  expect(contentsHas(run(node.markdown("md", "plain")), false)).toBe(false);
});

const host = (r: ReturnType<typeof run>) => render(<PageHost pageKey={key} runs={fakePageRuns(r).runs} />);

test("PageHost: a page that fills with no filter draws no Details", async () => {
  const view = host(run(filling()));
  await view.findByTestId("page-file");
  expect(view.queryByTestId("page-rail")).toBeNull();
  expect(view.queryByTestId("viz-rail-contents")).toBeNull();
});

test("PageHost: a page that fills with a filter draws Details with Filters and no Contents", async () => {
  const view = host(run(filling(), [filter]));
  await view.findByTestId("page-rail-filters");
  expect(view.getByTestId("page-rail")).not.toBeNull();
  expect(view.queryByTestId("viz-rail-contents")).toBeNull();
});

test("PageHost: a page of four sections draws Contents", async () => {
  const view = host(run(sections(4)));
  await view.findByTestId("viz-rail-contents");
  expect(view.getByTestId("page-rail")).not.toBeNull();
});

test("PageHost: a page with one section and nothing else draws no Details", async () => {
  const view = host(run(sections(1)));
  await view.findByTestId("page-file");
  expect(view.queryByTestId("page-rail")).toBeNull();
});

test("PageHost: a document page with no rail keeps the reading column", async () => {
  const none = host(run(sections(1)));
  await none.findByTestId("page-file");
  expect(none.queryByTestId("page-rail")).toBeNull();
  expect(none.container.querySelector(".oxplow-reading-column")).not.toBeNull();
  cleanup();
  const some = host(run(sections(1), [filter]));
  await some.findByTestId("page-rail-filters");
  expect(some.getByTestId("page-rail-center").className).toContain("reading");
});

// A `sections` viz's subsections are sections, every one of them: Contents
// lists up to ten, but the page has them all.
test("contentsHas: a sections viz's rows count as sections", () => {
  // The parent's own heading is one section; each row is another.
  expect(contentsHas(withSections(0, 0), false)).toBe(false);
  expect(contentsHas(withSections(1, 0), false)).toBe(true);
  expect(contentsHas(withSections(14, 0), false)).toBe(true);
  expect(contentsHas(withSections(100, 1), false)).toBe(true);
});

test("contentsHas: a single titled section alone gets none", () => {
  expect(contentsHas(run(sections(1)), false)).toBe(false);
});

// A grouped `sections` viz: each distinct group is a section too.
test("contentsHas: a grouped sections viz counts its groups as well as its rows", () => {
  const grouped = (data: string[][]) =>
    fakeRun(
      fakePage(node.stack("s", [node.viz("pages", "many", { viz: "sections", columns: [{ key: "id" }], sections: { title: "id", group: "g" } } as never)]), {
        loaders: [decl("items"), decl("many")],
      }),
      { loaders: { items, many: loader(["id", "g"], data) } },
    );
  // One row, one group: two sections.
  expect(contentsHas(grouped([["a", "x"]]), false)).toBe(true);
  // One row with no group: one section.
  expect(contentsHas(grouped([["a", ""]]), false)).toBe(false);
});
