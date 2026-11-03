import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// A right-click on a page's body that no row, editor or selection takes
// opens the page's own ☰ menu where the pointer is, so Add to Context (the
// whole page) and the rest are one click from anywhere on it. A child
// that owns its menu stops the event; selected text is the selection's.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({ ...realApi, refCommands: async () => [] }));
const { Page } = await import("./Page.js");

afterEach(cleanup);

function shown(children = <div data-testid="plain">body</div>) {
  const nav = {
    goBack() {},
    goForward() {},
    canGoBack: false,
    canGoForward: false,
    ref: { id: "oxplow:oxplow/page?page=alerts" },
    ask: { ref: "oxplow:oxplow/page?page=alerts", streamId: null },
  } as unknown as PageNavigation;
  return render(
    <PageNavigationContext.Provider value={nav}>
      <Page title="T">{children}</Page>
    </PageNavigationContext.Provider>,
  );
}

test("a right-click on the body opens the page's ☰ menu with Add to Context", async () => {
  const view = shown();
  fireEvent.contextMenu(view.getByTestId("plain"), { clientX: 40, clientY: 50 });
  const item = await waitFor(() => view.getByTestId("menu-item-page-add-to-context"));
  expect(item.textContent).toContain("Add to Context");
  expect(item.querySelector("[data-testid=menu-item-icon]")).not.toBeNull();
});

test("a child that owns its menu keeps the page's closed", async () => {
  const view = shown(
    <div data-testid="row" onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); }}>
      row
    </div>,
  );
  fireEvent.contextMenu(view.getByTestId("row"), { clientX: 40, clientY: 50 });
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("menu-item-page-add-to-context")).toBeNull();
});

test("a right-click on selected text is the selection's, not the page's", async () => {
  const view = shown();
  const text = view.getByTestId("plain").firstChild!;
  const range = document.createRange();
  range.selectNodeContents(text);
  window.getSelection()?.removeAllRanges();
  window.getSelection()?.addRange(range);
  fireEvent.contextMenu(view.getByTestId("plain"), { clientX: 40, clientY: 50 });
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("menu-item-page-add-to-context")).toBeNull();
  window.getSelection()?.removeAllRanges();
});
