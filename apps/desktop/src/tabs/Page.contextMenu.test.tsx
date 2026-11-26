import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// A right-click on a page's body opens no menu of the page's: its ☰ in the
// nav bar and each row's ☰ hold everything (usability.md "Per-row actions"),
// which a touch screen can reach and a right-click can't.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
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

test("a right-click on the body opens no page menu", async () => {
  const view = shown();
  fireEvent.contextMenu(view.getByTestId("plain"), { clientX: 40, clientY: 50 });
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("menu-item-page-add-to-context")).toBeNull();
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
