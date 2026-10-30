import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { leadOf, PageTree } from "./PageTree.js";
import { fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";

afterEach(cleanup);

const page = fakePage(
  node.stack("s", [
    node.viz("with", "about", { viz: "detail", columns: [{ key: "fact", label: "Fact" }], lead: "description" } as never, { title: "With" }),
    node.viz("without", "none", { viz: "detail", columns: [{ key: "fact", label: "Fact" }], lead: "description" } as never, { title: "Without" }),
  ]),
);
const run = fakeRun(page, {
  loaders: {
    about: loader(["description", "fact"], [["What it is.", "1"]]),
    none: loader(["description", "fact"], [["  ", "2"]]),
  },
});

test("a node's lead is a paragraph under its heading, above its facts; an empty one draws nothing", () => {
  const { container, queryByTestId } = render(<PageTree run={run} runId="run-1" />);
  const lead = queryByTestId("node-with-lead")!;
  expect(lead.textContent).toBe("What it is.");
  const node = container.querySelector('[data-node="with"]')!;
  // Under the heading, before the facts.
  expect(node.querySelector(".ox-section-title")!.compareDocumentPosition(lead) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(lead.compareDocumentPosition(node.querySelector(".ox-facts")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(queryByTestId("node-without-lead")).toBeNull();
});

test("leadOf: the first row's text, none when missing, null or blank", () => {
  const r = (columns: string[], rows: unknown[][], lead: string | null) => ({ config: { lead }, result: { columns, rows } });
  expect(leadOf(r(["d"], [["x"], ["y"]], "d"))).toBe("x");
  expect(leadOf(r(["d"], [[null]], "d"))).toBeNull();
  expect(leadOf(r(["d"], [[" "]], "d"))).toBeNull();
  expect(leadOf(r(["e"], [["x"]], "d"))).toBeNull();
  expect(leadOf(r(["d"], [], "d"))).toBeNull();
  expect(leadOf(r(["d"], [["x"]], null))).toBeNull();
});
