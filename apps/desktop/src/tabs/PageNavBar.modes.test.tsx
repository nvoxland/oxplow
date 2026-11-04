import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Renderer } from "../tauri-bridge/generated/bindings.js";
import { Page } from "./Page.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// A URI with several modes swaps between them from the nav bar in one
// click: two or three renderers are a segmented control, the one drawing
// pressed; more are one button labeled with the current mode that opens
// the list. Both are the same pick as the ☰'s View as (the default
// renderer drops the key). One renderer draws nothing.

const renderer = (page: string, title: string, over: Partial<Renderer> = {}): Renderer => ({ page, title, default: false, ...over });
const editor = renderer("oxplow-foundation/editor", "Editor", { default: true });
const two = [editor, renderer("oxplow-foundation/markdown", "Markdown")];
const four = [...two, renderer("oxplow-foundation/data-file", "Tree"), renderer("oxplow-foundation/file-viewer", "Text view")];

afterEach(cleanup);

function shown(renderers: Renderer[], patches: unknown[], viewParams: Record<string, string> = {}) {
  const nav = {
    goBack() {},
    goForward() {},
    canGoBack: false,
    canGoForward: false,
    ref: { id: "oxplow:oxplow/file?path=README.md" },
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

test("two or three modes are a segmented control; the one drawing is pressed and a press swaps", async () => {
  const patches: unknown[] = [];
  const view = shown(two, patches);
  const current = await waitFor(() => view.getByTestId("page-nav-mode-oxplow-foundation/editor"));
  expect(current.getAttribute("aria-pressed")).toBe("true");
  expect(current.textContent).toBe("Editor");
  const other = view.getByTestId("page-nav-mode-oxplow-foundation/markdown");
  expect(other.getAttribute("aria-pressed")).toBe("false");
  expect(view.queryByTestId("page-nav-modes")).toBeNull();
  fireEvent.click(other);
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": "oxplow-foundation/markdown" }]));
});

test("pressing the default mode drops the key, as View as does", async () => {
  const patches: unknown[] = [];
  const view = shown(two, patches, { "oxplow.renderer": "oxplow-foundation/markdown" });
  const markdown = await waitFor(() => view.getByTestId("page-nav-mode-oxplow-foundation/markdown"));
  expect(markdown.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(view.getByTestId("page-nav-mode-oxplow-foundation/editor"));
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": undefined }]));
});

test("four or more modes are one button named for the current mode that lists the rest", async () => {
  const patches: unknown[] = [];
  const view = shown(four, patches, { "oxplow.renderer": "oxplow-foundation/data-file" });
  const button = await waitFor(() => view.getByTestId("page-nav-modes"));
  expect(button.textContent).toContain("Tree");
  expect(view.queryByTestId("page-nav-mode-oxplow-foundation/editor")).toBeNull();
  fireEvent.click(button);
  const item = await waitFor(() => view.getByTestId("menu-item-page-mode-oxplow-foundation/markdown"));
  expect(view.getByTestId("menu-item-page-mode-oxplow-foundation/data-file").textContent).toContain("✓");
  fireEvent.click(item);
  await waitFor(() => expect(patches).toEqual([{ "oxplow.renderer": "oxplow-foundation/markdown" }]));
});

test("one mode draws no control", async () => {
  const view = shown([editor], []);
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("page-nav-modes")).toBeNull();
  expect(view.queryByTestId("page-nav-mode-oxplow-foundation/editor")).toBeNull();
});
