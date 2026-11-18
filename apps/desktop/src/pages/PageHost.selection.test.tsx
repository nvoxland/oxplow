import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ContextWhat } from "../contextBasket.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { decl, fakePage, fakePageRuns, fakeRun, loader, node } from "./testing/fakeRun.js";

// Any page's text takes Add to Context over a fresh selection, on the
// page's URI (its tab's identity, without the view part); its markdown
// headings are the rail's Contents.

const added: ContextWhat[] = [];
const realBasket = { ...(await import("../contextBasket.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../contextBasket.js", () => ({
  ...realBasket,
  addToContext: async (what: ContextWhat) => {
    added.push(what);
  },
}));
const { PageHost } = await import("./PageHost.js");

afterEach(() => {
  cleanup();
  added.length = 0;
  window.getSelection()?.removeAllRanges();
});

const URI = "oxplow:oxplow/knowledge?id=notes&store=oxwiki";

function page() {
  const run = fakeRun(
    fakePage(node.viz("body", "body", { viz: "markdown", markdown: { text: "body" } as never }), { loaders: [decl("body")] }),
    { loaders: { body: loader(["body"], [["## First\n\nText.\n\n### Second\n\nMore."]]) } },
  );
  const nav = {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref: { id: `${URI}#lines=1-2` },
    viewParams: {},
    setViewParams: () => {},
  } as unknown as PageNavigation;
  return (
    <PageNavigationContext.Provider value={nav}>
      <PageHost pageKey={{ page: "acme/page", params: { uri: URI }, streamId: "str1", threadId: null }} runs={fakePageRuns(run).runs} renders={URI} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>
  );
}

test("a selection in a page's text offers Add to Context on the page's URI", async () => {
  const view = render(page());
  const body = await waitFor(() => view.getByTestId("viz-markdown"));
  const para = await waitFor(() => [...body.querySelectorAll("p")].find((p) => p.textContent === "Text.")!);
  const range = document.createRange();
  range.selectNodeContents(para);
  window.getSelection()!.removeAllRanges();
  window.getSelection()!.addRange(range);
  await act(async () => {
    fireEvent.mouseUp(para);
    await new Promise((r) => setTimeout(r, 5));
  });
  fireEvent.mouseDown(await waitFor(() => view.getByTestId("selection-context-button")));
  expect(added).toEqual([{ uri: URI, text: "Text." }]);
});

test("the body's markdown headings are the rail's Contents", async () => {
  const view = render(page());
  const contents = await waitFor(() => view.getByTestId("viz-rail-contents"));
  await waitFor(() => expect([...contents.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["First", "Second"]));
});

// `mock.module` outlives this file: put the real module back for the files that run after it.
afterAll(() => {
  mock.module("../contextBasket.js", () => realBasket);
});
