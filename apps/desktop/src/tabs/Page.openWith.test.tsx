import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// "Open with": the nav bar offers the pages that render the shown URI when
// there are two or more; picking one patches the URI's view with
// `oxplow.renderer`, so the same URI stays in the tab.

const renderer = (extension: string, id: string, more: object = {}) => ({
  id,
  extension,
  pageRef: `oxplow:oxplow/page?page=${extension}/${id}`,
  title: id,
  icon: null,
  category: null,
  keywords: null,
  params: [],
  count: null,
  for: ["oxplow/file"],
  accepts: {},
  default: false,
  pinned: false,
  body: { kind: "builtIn", name: id },
  ...more,
});
let pages = [renderer("oxplow-foundation", "editor", { default: true }), renderer("acme", "blame")];

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [{ name: "ext", enabled: true, pages, ui: { slots: [], commands: [], decorators: [] } }],
}));
const { Page } = await import("./Page.js");

afterEach(cleanup);

const FILE = "oxplow:oxplow/file?path=a.ts";

function shown(id: string, patches: unknown[]) {
  const nav = {
    goBack() {},
    goForward() {},
    canGoBack: false,
    canGoForward: false,
    ref: { id },
    viewParams: {},
    patchViewParams: (patch: unknown) => patches.push(patch),
  } as unknown as PageNavigation;
  return render(
    <PageNavigationContext.Provider value={nav}>
      <Page title="T">body</Page>
    </PageNavigationContext.Provider>,
  );
}

test("a URI with two renderers offers Open with; picking one patches the view with oxplow.renderer", async () => {
  const patches: unknown[] = [];
  const view = shown(FILE, patches);
  const select = (await waitFor(() => view.getByTestId("page-nav-open-with"))) as HTMLSelectElement;
  expect(select.value).toBe("oxplow-foundation/editor");
  fireEvent.change(select, { target: { value: "acme/blame" } });
  expect(patches).toEqual([{ "oxplow.renderer": "acme/blame" }]);
});

test("a URI with one renderer, or none, offers nothing", async () => {
  const patches: unknown[] = [];
  const one = shown("oxplow:oxplow/wiki?slug=a", patches);
  // Let the extensions load before asserting absence.
  await new Promise((r) => setTimeout(r, 20));
  expect(one.queryByTestId("page-nav-open-with")).toBeNull();
  cleanup();
  pages = [renderer("oxplow-foundation", "editor", { default: true })];
  const solo = shown(FILE, patches);
  await new Promise((r) => setTimeout(r, 20));
  expect(solo.queryByTestId("page-nav-open-with")).toBeNull();
  pages = [renderer("oxplow-foundation", "editor", { default: true }), renderer("acme", "blame")];
});
