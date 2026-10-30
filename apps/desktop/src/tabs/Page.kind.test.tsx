import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { setResources, type Resource } from "../resources.js";
import { Page } from "./Page.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// The nav bar says what kind of thing a tab shows: its URI's resource's
// label, before the title. A page that is no resource's (a route) says none.

afterEach(cleanup);

const resource = (name: string, label: string): Resource => ({
  resource: name,
  extension: "oxplow-foundation",
  label,
  params: {},
  additional: false,
  resolve: null,
  title: null,
  searchable: null,
  icon: null,
  wikilink: null,
});

function shown(id: string) {
  setResources([resource("oxplow/measure", "Measure"), resource("oxplow/page", "Page")]);
  const nav = { goBack() {}, goForward() {}, canGoBack: false, canGoForward: false, ref: { id } } as unknown as PageNavigation;
  return render(
    <PageNavigationContext.Provider value={nav}>
      <Page title="Static-analysis hit">body</Page>
    </PageNavigationContext.Provider>,
  );
}

test("a resource's page names its kind before the title", () => {
  const view = shown("oxplow:oxplow/measure?key=oxplow.lint_hit");
  expect(view.getByTestId("page-nav-kind").textContent).toBe("Measure");
  expect(view.getByTestId("page-nav-title").textContent).toBe("Static-analysis hit");
});

test("a page route and an unknown resource name no kind", () => {
  expect(shown("oxplow:oxplow/page?page=extensions").queryByTestId("page-nav-kind")).toBeNull();
  cleanup();
  expect(shown("oxplow:acme/unknown?id=1").queryByTestId("page-nav-kind")).toBeNull();
});
