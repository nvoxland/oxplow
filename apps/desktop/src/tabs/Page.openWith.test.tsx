import { afterAll, afterEach, expect, mock, test } from "bun:test";
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
// Page files (`pages/*.yaml`): their id is `<extension>/<slug>`.
let files: object[] = [];
const file = (extension: string, slug: string, more: object = {}) => ({
  id: `${extension}/${slug}`,
  extension,
  slug,
  title: slug,
  for: ["oxplow/file"],
  accepts: {},
  default: false,
  actions: [],
  params: [],
  view: { id: "v", kind: "markdown", text: "" },
  rail: null,
  ...more,
});

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [{ name: "ext", enabled: true, pages, pageFiles: files, ui: { slots: [], commands: [], decorators: [] } }],
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

/** The Open With submenu's items, once the ☰ has it. */
async function openWith(view: ReturnType<typeof render>) {
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-menu")));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-page-open-with")));
}

test("a URI with two renderers offers Open with; picking one patches the view with oxplow.renderer", async () => {
  const patches: unknown[] = [];
  const view = shown(FILE, patches);
  await openWith(view);
  expect(view.getByTestId("menu-item-page-open-with-oxplow-foundation/editor").textContent).toContain("✓");
  expect(view.getByTestId("menu-item-page-open-with-acme/blame").textContent).not.toContain("✓");
  fireEvent.click(view.getByTestId("menu-item-page-open-with-acme/blame"));
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": "acme/blame" }]));
});

/** Open the ☰ (when there is one) once the extensions have loaded. */
async function menuItems(view: ReturnType<typeof render>) {
  await new Promise((r) => setTimeout(r, 20));
  const menu = view.queryByTestId("page-nav-menu");
  if (menu) fireEvent.click(menu);
}

test("a URI with one renderer, or none, offers nothing", async () => {
  const patches: unknown[] = [];
  const one = shown("oxplow:oxplow/wiki?slug=a", patches);
  await menuItems(one);
  expect(one.queryByTestId("menu-item-page-open-with")).toBeNull();
  cleanup();
  pages = [renderer("oxplow-foundation", "editor", { default: true })];
  const solo = shown(FILE, patches);
  await menuItems(solo);
  expect(solo.queryByTestId("menu-item-page-open-with")).toBeNull();
  pages = [renderer("oxplow-foundation", "editor", { default: true }), renderer("acme", "blame")];
});

test("page files' renderers are offered, the one drawing it checked", async () => {
  pages = [];
  files = [file("oxplow-foundation", "editor", { default: true }), file("acme", "blame", { title: "Blame" })];
  try {
    const patches: unknown[] = [];
    const view = shown(FILE, patches);
    await openWith(view);
    expect(view.getByTestId("menu-item-page-open-with-oxplow-foundation/editor").textContent).toContain("✓");
    expect(view.getByTestId("menu-item-page-open-with-acme/blame").textContent).toContain("Blame");
  } finally {
    files = [];
    pages = [renderer("oxplow-foundation", "editor", { default: true }), renderer("acme", "blame")];
  }
});

// `mock.module` is process-wide: put the real module back so the files that
// run after this one don't see this file's extensions.
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});
