import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Renderer } from "../tauri-bridge/generated/bindings.js";
import { Page } from "./Page.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// "Open with": the nav bar offers the pages the daemon says can draw the
// shown URI when there are two or more; picking one patches the URI's view
// with `oxplow.renderer`, so the same URI stays in the tab.

const renderer = (page: string, title: string, over: Partial<Renderer> = {}): Renderer => ({ page, title, default: false, ...over });
const two = [renderer("oxplow-foundation/editor", "Editor", { default: true }), renderer("acme/blame", "Blame")];

afterEach(cleanup);

const FILE = "oxplow:oxplow/file?path=a.ts";

function shown(id: string, renderers: Renderer[] | undefined, patches: unknown[], viewParams: Record<string, string> = {}) {
  const nav = {
    goBack() {},
    goForward() {},
    canGoBack: false,
    canGoForward: false,
    ref: { id },
    renderers,
    viewParams,
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
  const view = shown(FILE, two, patches);
  await openWith(view);
  expect(view.getByTestId("menu-item-page-open-with-oxplow-foundation/editor").textContent).toContain("✓");
  expect(view.getByTestId("menu-item-page-open-with-acme/blame").textContent).not.toContain("✓");
  expect(view.getByTestId("menu-item-page-open-with-acme/blame").textContent).toContain("Blame");
  fireEvent.click(view.getByTestId("menu-item-page-open-with-acme/blame"));
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": "acme/blame" }]));
});

test("the renderer the view names is the one checked", async () => {
  const view = shown(FILE, two, [], { "oxplow.renderer": "acme/blame" });
  await openWith(view);
  expect(view.getByTestId("menu-item-page-open-with-acme/blame").textContent).toContain("✓");
  expect(view.getByTestId("menu-item-page-open-with-oxplow-foundation/editor").textContent).not.toContain("✓");
});

test("a URI with one renderer, none, or a list not yet asked offers nothing", async () => {
  for (const renderers of [[two[0]!], [], undefined]) {
    const view = shown(FILE, renderers, []);
    await new Promise((r) => setTimeout(r, 20));
    const menu = view.queryByTestId("page-nav-menu");
    if (menu) fireEvent.click(menu);
    expect(view.queryByTestId("menu-item-page-open-with")).toBeNull();
    cleanup();
  }
});

// The default's mode is "no renderer in the view" (tabs match on it), so
// picking the default drops the key rather than writing it.
test("picking the default renderer drops oxplow.renderer from the view", async () => {
  const patches: unknown[] = [];
  const view = shown(FILE, two, patches, { "oxplow.renderer": "acme/blame" });
  await openWith(view);
  fireEvent.click(view.getByTestId("menu-item-page-open-with-oxplow-foundation/editor"));
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": undefined }]));
});
