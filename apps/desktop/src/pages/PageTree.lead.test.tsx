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

// A lead is Markdown unless it says `format: text`: a commit message's
// paragraphs, lists and `code` read as such; a text lead keeps its `*` and
// its line breaks as written.
test("a lead draws as Markdown by default, and as text with its line breaks when it says so", () => {
  const message = "Add the option\n\n- `chartable: true` on a table\n- a view\nsaved as one";
  const formatted = fakePage(
    node.stack("s", [
      node.viz("md", "msg", { viz: "detail", columns: [{ key: "fact" }], lead: "description" } as never, { title: "Md" }),
      node.viz("txt", "msg", { viz: "detail", columns: [{ key: "fact" }], lead: "description", leadFormat: "text" } as never, { title: "Txt" }),
    ]),
  );
  const r = fakeRun(formatted, { loaders: { msg: loader(["description", "fact"], [[message, "1"]]) } });
  const { getByTestId } = render(<PageTree run={r} runId="run-1" />);
  const md = getByTestId("node-md-lead");
  expect(md.querySelectorAll("li")).toHaveLength(2);
  expect(md.querySelector("code")!.textContent).toBe("chartable: true");
  expect(md.querySelectorAll("p").length).toBeGreaterThan(0);
  const txt = getByTestId("node-txt-lead");
  expect(txt.querySelector("li")).toBeNull();
  expect(txt.textContent).toBe(message);
  expect(txt.style.whiteSpace).toBe("pre-line");
});
