import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

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

// A page with no rail has the same gutter as one with a rail.
test("a page with no rail has the same side padding as one with a rail", () => {
  const withRail = render(<Page title="T" rightRail={<p>controls</p>}>body</Page>);
  const railPad = withRail.getByTestId("page-rail-body").style.padding;
  cleanup();
  const bare = render(<Page title="T">body</Page>);
  expect(railPad).not.toBe("");
  expect(bare.getByTestId("page-body").style.padding).toBe(railPad);
});

// The page says what it can do; the bar draws it, in every layout.
test("a page's actions are drawn in the nav bar, also beside a rail: the primary one a button, the rest in the ☰", async () => {
  const ran: string[] = [];
  const { getByTestId, queryByTestId } = render(
    <PageNavigationContext.Provider value={nav()}>
      <Page
        title="T"
        width="reading"
        rightRail={<p>rail</p>}
        actions={[
          { id: "save", label: "Save", primary: true, run: () => void ran.push("save") },
          { id: "close", label: "Close", run: () => void ran.push("close") },
        ]}
      >
        body
      </Page>
    </PageNavigationContext.Provider>,
  );
  const save = getByTestId("action-save");
  expect(getByTestId("page-rail").contains(save)).toBe(false);
  fireEvent.click(save);
  expect(queryByTestId("action-close")).toBeNull();
  fireEvent.click(getByTestId("page-nav-menu"));
  fireEvent.click(getByTestId("menu-item-action-close"));
  await waitFor(() => expect(ran).toEqual(["save", "close"]));
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
  const { getByRole } = render(<Page title="T" chips={[{ label: "Fresh", onClick: () => opened++ }]}>body</Page>);
  fireEvent.click(getByRole("button", { name: "Fresh" }));
  expect(opened).toBe(1);
});

// The title is the nav bar's, always: a scrolling page's body has no heading.
test("the title is in the nav bar and the body draws no heading", () => {
  const { getByTestId, container } = render(
    <PageNavigationContext.Provider value={nav()}>
      <Page title="A rather long page title" titleIcon={<i data-testid="ico" />}>body</Page>
    </PageNavigationContext.Provider>,
  );
  const title = getByTestId("page-nav-title");
  expect(title.textContent).toBe("A rather long page title");
  expect(title.getAttribute("title")).toBe("A rather long page title");
  expect(getByTestId("page-nav-title-icon").contains(getByTestId("ico"))).toBe(true);
  expect(container.querySelector("h1")).toBeNull();
});

// A long rail scrolls inside the page's visible height (the scroll body's,
// not the window's, which includes the app's chrome above the page), and
// its title band is opaque, so what scrolls under it doesn't show through.
test("the side rail is as tall as the page body shows, under an opaque title", () => {
  const real = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class {
    constructor(private readonly cb: ResizeObserverCallback) {}
    observe() {
      this.cb([{ contentRect: { width: 1200, height: 500 } } as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  try {
    const { getByTestId } = render(
      <Page title="T" rightRail={<p>controls</p>}>
        body
      </Page>,
    );
    expect(getByTestId("page-rail").style.maxHeight).toBe("500px");
    expect(getByTestId("page-rail-title").style.background).toBe("var(--surface-chrome)");
  } finally {
    globalThis.ResizeObserver = real;
  }
});
