import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { peekPendingCommentReveal, clearCommentReveal } from "../comment-reveal-bus.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { decl, fakePage, fakePageRuns, fakeRun, loader, node } from "./testing/fakeRun.js";

// A page file with `comments: true` gets what a lens page with it got: the
// nav bar's navigator on its URI, a body plain-text selections resolve
// to, `#comment=` revealed; its markdown headings are the rail's Contents.

const targets: string[] = [];
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listCommentsForTarget: async (target: string) => {
    targets.push(target);
    return [{ comment: { id: "cmt1", target, quote: "Text", status: "open", orphaned: false, selectors_json: "[]" }, messages: [] }];
  },
  subscribeCommentEvents: () => () => {},
  onRemoteReconnect: realApi.onRemoteReconnect,
}));
const { PageHost } = await import("./PageHost.js");

afterEach(() => {
  cleanup();
  targets.length = 0;
});

const URI = "oxplow:oxplow/wiki?slug=notes";

function page(comments: boolean, viewParams: Record<string, string> = {}) {
  const run = fakeRun(
    fakePage(node.viz("body", "body", { viz: "markdown", markdown: { text: "body" } as never }), { comments, loaders: [decl("body")] }),
    { loaders: { body: loader(["body"], [["## First\n\nText.\n\n### Second\n\nMore."]]) } },
  );
  const nav = {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    ref: { id: `${URI}#lines=1-2` },
    viewParams,
    setViewParams: () => {},
  } as unknown as PageNavigation;
  return (
    <PageNavigationContext.Provider value={nav}>
      <PageHost pageKey={{ page: "acme/page", params: { uri: URI }, streamId: "str1", threadId: null }} runs={fakePageRuns(run).runs} renders={URI} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>
  );
}

test("a page that takes comments steps through them on its URI, and its body is that URI's", async () => {
  const view = render(page(true));
  await waitFor(() => expect(view.getByTestId("lens-markdown")).toBeTruthy());
  await waitFor(() => expect(view.getByTestId("page-nav-comments-toggle")).toBeTruthy());
  expect(targets).toContain(URI);
  expect(view.container.querySelector(`[data-ref="${URI}"]`)).not.toBeNull();
});

test("a page that doesn't take them has neither", async () => {
  const view = render(page(false));
  await waitFor(() => expect(view.getByTestId("lens-markdown")).toBeTruthy());
  expect(view.queryByTestId("page-nav-comments-toggle")).toBeNull();
  expect(targets).toEqual([]);
  expect(view.container.querySelector(`[data-ref="${URI}"]`)).toBeNull();
});

test("the body's markdown headings are the rail's Contents, above About this page", async () => {
  const view = render(page(false));
  const contents = await waitFor(() => view.getByTestId("lens-rail-contents"));
  await waitFor(() => expect([...contents.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["First", "Second"]));
  expect(view.getByTestId("page-rail-about")).toBeTruthy();
});

test("`#comment=` reveals that comment on a page that takes comments, and only there", async () => {
  const taking = render(page(true, { comment: "cmt1" }));
  await waitFor(() => expect(taking.getByTestId("lens-markdown")).toBeTruthy());
  expect(peekPendingCommentReveal()).toBe("cmt1");
  clearCommentReveal("cmt1");
  cleanup();
  const not = render(page(false, { comment: "cmt2" }));
  await waitFor(() => expect(not.getByTestId("lens-markdown")).toBeTruthy());
  expect(peekPendingCommentReveal()).toBeNull();
});
