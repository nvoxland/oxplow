import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import type { Stream } from "../tauri-bridge/index.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// Comments are a page's capability: a declared page with `comments: true`
// gets the nav bar's navigator on its URI and a body plain-text
// selections resolve to; one without gets neither. A body with markdown
// headings gets a Contents in its rail.

const targets: string[] = [];
const realApi = await import("../api.js");
const lens = {
  id: "x/note",
  extension: "x",
  slug: "note",
  title: "Note",
  description: "",
  query: "",
  viz: "markdown",
  markdown: { text: "body" },
  params: [{ name: "uri", label: null, default: null, options: [], type: "text", role: "resource", presets: [] }],
  columns: [],
  actions: [],
  parts: [],
  hidden: false,
  needs: [],
  origin: { extension: "x", source: null, running: "", deployment: null, bundled: false },
} as unknown as Lens;
mock.module("../api.js", () => ({
  ...realApi,
  runLens: async () =>
    ({
      lens,
      params: {},
      result: { columns: ["body"], rows: [["## First\n\nText.\n\n### Second\n\nMore."]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
    }) as unknown as LensRun,
  listCommentsForTarget: async (target: string) => {
    targets.push(target);
    return [{ comment: { id: "cmt1", target, quote: "Text", status: "open", orphaned: false, selectors_json: "[]" }, messages: [] }];
  },
  subscribeCommentEvents: () => () => {},
  onRemoteReconnect: realApi.onRemoteReconnect,
}));
const { LensPage } = await import("./LensPage.js");

afterEach(() => {
  cleanup();
  targets.length = 0;
});

const URI = "oxplow:oxplow/wiki?slug=notes";
const stream = { id: "str1" } as unknown as Stream;

function page(comments: boolean) {
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
      <LensPage lensId="x/note" params={{ uri: URI }} renders={URI} stream={stream} onOpenPage={() => {}} comments={comments} />
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

test("the body's markdown headings are the rail's Contents", async () => {
  const view = render(page(false));
  const contents = await waitFor(() => view.getByTestId("lens-rail-contents"));
  await waitFor(() => expect([...contents.querySelectorAll("button")].map((b) => [b.textContent, b.getAttribute("data-level")])).toEqual([
    ["First", "2"],
    ["Second", "3"],
  ]));
});
