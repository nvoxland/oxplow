import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { PageNavigationContext, useTabEditing, type PageNavigation, type TabEditing } from "./PageNavigationContext.js";

// A page body that edits says so to its tab: whether it holds unsaved
// edits and how to save them. The shell draws the ● and runs Save.

afterEach(cleanup);

function Body({ dirty, saved }: { dirty: boolean; saved: string[] }) {
  useTabEditing({ dirty, save: () => saved.push(dirty ? "dirty" : "clean") });
  return null;
}

function host(registered: (TabEditing | null)[]): PageNavigation {
  return {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    setEditing: (e) => registered.push(e),
  };
}

test("a body registers whether it is dirty, its latest save, and clears it when it goes", () => {
  const registered: (TabEditing | null)[] = [];
  const saved: string[] = [];
  const nav = host(registered);
  const { rerender, unmount } = render(
    <PageNavigationContext.Provider value={nav}>
      <Body dirty={false} saved={saved} />
    </PageNavigationContext.Provider>,
  );
  expect(registered.at(-1)?.dirty).toBe(false);
  rerender(
    <PageNavigationContext.Provider value={nav}>
      <Body dirty saved={saved} />
    </PageNavigationContext.Provider>,
  );
  expect(registered.at(-1)?.dirty).toBe(true);
  registered.at(-1)!.save();
  expect(saved).toEqual(["dirty"]);
  unmount();
  expect(registered.at(-1)).toBeNull();
});

test("outside a tab it does nothing", () => {
  const saved: string[] = [];
  expect(() => render(<Body dirty saved={saved} />)).not.toThrow();
});
