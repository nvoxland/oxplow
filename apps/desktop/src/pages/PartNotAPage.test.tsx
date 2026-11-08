import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { PartNotAPage } from "./PartNotAPage.js";
import { hostsOf } from "./pageFiles.js";
import { fakePage, node } from "./testing/fakeRun.js";
import type { Extension } from "../tauri-bridge/generated/bindings.js";

afterEach(cleanup);

const section = fakePage(node.viz("rows", "rows"), { id: "acme/section", slug: "section", title: "Section", kind: "part" });
const kinds = fakePage(node.viz("rows", "rows"), { id: "acme/kinds", slug: "kinds", title: "Kinds", kind: "query" });
const host = fakePage(node.stack("root", [node.viz("rows", "rows"), node.embed("more", "acme/section")]), { id: "acme/host", slug: "host", title: "Host" });
const extensions = [
  {
    name: "acme",
    enabled: true,
    pageFiles: [host, section, kinds],
    panels: [{ id: "acme/side", title: "Side", body: "acme/section", collapsed: null, count: null, badge: null }],
    ui: { pageSlots: [{ host: "oxplow/work-item", slot: "sections", page: "acme/section" }], decorators: [], replacements: [] },
  },
] as unknown as Extension[];

// A part opened by its URI says what it is and where it is drawn; a query
// says it is rows other pages read.
test("a part's tab names the pages, panels and slots that draw it; a query's says what it is", () => {
  expect(hostsOf(extensions, "acme/section")).toEqual(["page `acme/host`", "panel `acme/side`", "page `oxplow/work-item` (slot `sections`)"]);
  const part = render(<PartNotAPage page={section} extensions={extensions} />);
  expect(part.getByTestId("page-not-a-page").textContent).toContain("is a part, drawn inside page `acme/host`, panel `acme/side`");
  cleanup();
  const query = render(<PartNotAPage page={kinds} extensions={extensions} />);
  expect(query.getByTestId("page-not-a-page").textContent).toContain("is a query: rows other pages read");
});
