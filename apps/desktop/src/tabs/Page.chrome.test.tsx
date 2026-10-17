import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { Page } from "./Page.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

afterEach(cleanup);

const nav = (over: Partial<PageNavigation> = {}): PageNavigation => ({
  navigate: () => {},
  goBack: () => {},
  goForward: () => {},
  canGoBack: true,
  canGoForward: false,
  ...over,
});

// The rail is the page's control panel, beside a body of either width.
test("the width and the rail are separate: a full-width page can have a rail", () => {
  const full = render(
    <Page title="T" rightRail={<p>controls</p>}>
      body
    </Page>,
  );
  expect(full.getByTestId("page-rail")).toBeTruthy();
  expect(full.getByTestId("page-rail-center").className).not.toContain("oxplow-reading-column");
  cleanup();
  const reading = render(
    <Page title="T" width="reading">
      body
    </Page>,
  );
  expect(reading.getByTestId("page-rail-center").className).toContain("oxplow-reading-column");
  expect(reading.queryByTestId("page-rail")).toBeNull();
  cleanup();
  const bare = render(<Page title="T">body</Page>);
  expect(bare.queryByTestId("page-rail-body")).toBeNull();
});

// The page says what it can do; the bar draws it, in every layout.
test("a page's actions are drawn in the nav bar, also beside a rail", () => {
  const ran: string[] = [];
  const { getByTestId } = render(
    <PageNavigationContext.Provider value={nav()}>
      <Page title="T" width="reading" rightRail={<p>rail</p>} actions={[{ id: "close", label: "Close", run: () => void ran.push("close") }]}>
        body
      </Page>
    </PageNavigationContext.Provider>,
  );
  const button = getByTestId("action-close");
  expect(getByTestId("page-rail").contains(button)).toBe(false);
  fireEvent.click(button);
  expect(ran).toEqual(["close"]);
});

// A pinned page is always itself: its bar has no history to step through.
test("a pinned page's bar has no back or forward", () => {
  const pinned = render(
    <PageNavigationContext.Provider value={nav({ pinned: true })}>
      <Page title="Session">body</Page>
    </PageNavigationContext.Provider>,
  );
  expect(pinned.queryByTestId("page-nav-back")).toBeNull();
  expect(pinned.getByTestId("page-nav-title").textContent).toBe("Session");
  cleanup();
  const plain = render(
    <PageNavigationContext.Provider value={nav()}>
      <Page title="Page">body</Page>
    </PageNavigationContext.Provider>,
  );
  expect(plain.getByTestId("page-nav-back")).toBeTruthy();
});

test("a chip that opens something is a button", () => {
  let opened = 0;
  const { getByRole } = render(<Page title="T" showNavBar={false} chips={[{ label: "Fresh", onClick: () => opened++ }]}>body</Page>);
  fireEvent.click(getByRole("button", { name: "Fresh" }));
  expect(opened).toBe(1);
});
