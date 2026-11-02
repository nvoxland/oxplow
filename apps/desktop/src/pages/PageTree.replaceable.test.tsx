import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { configValueResult, readsSettings } from "../configValuesTestSupport.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import type { PageRun, RunNode } from "../tauri-bridge/generated/bindings.js";

// A page file's node marked `replaceable: <target>` draws what the page run
// decided (`RunNode.replacedBy`, the run's embed): another extension's page
// in its place, badged, or oxplow's own.

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
let extensions: unknown[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  subscribeOxplowEvents: () => () => {},
  querySql: async (sql: string) =>
    readsSettings(sql) ? configValueResult([]) : { columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} },
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

/** A page whose `cards` node is replaceable (the Board), its own rows
 *  `own`; the run's tree as `tree` says. */
function host(tree: Partial<RunNode>): PageRun {
  const page = fakePage({ ...node.viz("cards", "own"), replaceable: "work_item.board" });
  const run = fakeRun(page, { loaders: { own: loader(["title"], [["oxplow's own row"]]) } });
  return { ...run, tree: { ...run.tree, ...tree } as RunNode };
}

/** The page another extension stands in with, as the run ran it. */
const lanes = fakeRun(fakePage(node.viz("lanes", "rows"), { id: "beta/lanes", extension: "beta", slug: "lanes" }), {
  loaders: { rows: loader(["lane"], [["Doing"]]) },
});

test("a node the run says another extension stands in for draws that run, saying so on hover and not in a chip", () => {
  const { container, getByTestId, queryByTestId } = render(<PageTree run={host({ replacedBy: "beta", embed: lanes })} runId="run-1" />);
  expect(queryByTestId("replacement-badge")).toBeNull();
  expect(getByTestId("replacement-work_item.board").getAttribute("title")).toContain("beta");
  // Drawn as an embed: its nodes under the replaced node's id.
  expect(container.querySelector('[data-node="cards/lanes"]')!.textContent).toContain("Doing");
  expect(container.textContent).not.toContain("oxplow's own row");
});

test("one whose page couldn't run shows oxplow's own, and says why", () => {
  const { getByTestId, container } = render(
    <PageTree run={host({ replacedBy: "beta", embed: null, error: "page beta/lanes is gone" })} runId="run-1" />,
  );
  expect(getByTestId("replacement-fallback").textContent).toContain("couldn't load: page beta/lanes is gone.");
  expect(container.textContent).toContain("oxplow's own row");
});

test("a replaceable node nobody stands in for is oxplow's own", async () => {
  const view = render(<PageTree run={host({})} runId="run-1" />);
  expect(view.queryByTestId("replacement-badge")).toBeNull();
  await waitFor(() => expect(view.container.textContent).toContain("oxplow's own row"));
});
