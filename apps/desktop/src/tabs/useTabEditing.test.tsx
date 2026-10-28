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

// A page's actions are its tab's right-click menu too: registered while
// it shows them, the latest run, cleared when it goes.
test("a body's actions reach its tab, the latest one runs, and they clear when it goes", async () => {
  const { useTabActions } = await import("./PageNavigationContext.js");
  const registered: (readonly { id: string; run(): void }[] | null)[] = [];
  const nav = { ...host([]), setTabActions: (a: readonly { id: string; run(): void }[] | null) => registered.push(a) } as PageNavigation;
  const ran: string[] = [];
  function Actions({ tag }: { tag: string }) {
    useTabActions([{ id: "rename", label: "Rename…", run: () => ran.push(tag) }]);
    return null;
  }
  const { rerender, unmount } = render(
    <PageNavigationContext.Provider value={nav}>
      <Actions tag="first" />
    </PageNavigationContext.Provider>,
  );
  rerender(
    <PageNavigationContext.Provider value={nav}>
      <Actions tag="second" />
    </PageNavigationContext.Provider>,
  );
  registered.at(-1)![0]!.run();
  expect(ran).toEqual(["second"]);
  unmount();
  expect(registered.at(-1)).toBeNull();
});
