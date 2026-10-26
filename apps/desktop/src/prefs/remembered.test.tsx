import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { extPageRef, pageTabRef, wikiPageRef } from "../tabs/pageRefs.js";
import type { UiPreference } from "../tauri-bridge/generated/bindings.js";
import {
  createRememberedStore,
  pageMemoryScope,
  RememberedContext,
  useAppPreference,
  usePageMemory,
  usePanelMemory,
} from "./remembered.js";

afterEach(cleanup);

function backend(stored: UiPreference[] = []) {
  const saves: [string, string, string | null][] = [];
  return {
    saves,
    load: async () => stored,
    save: async (scope: string, key: string, valueJson: string | null) => void saves.push([scope, key, valueJson]),
  };
}

test("a page's memory is scoped by the page, whatever it shows and however", () => {
  expect(pageMemoryScope(pageTabRef("tasks", { stream: "str1" }, { state: "open" }))).toBe("oxplow:oxplow/page?page=tasks");
  expect(pageMemoryScope(extPageRef("acme", "pulls"))).toBe("oxplow:oxplow/page?page=acme/pulls");
  expect(pageMemoryScope(pageTabRef("x/y", { stream_id: 2 }, { range: "7d" }))).toBe("oxplow:oxplow/page?page=x/y");
  // An entity page's is its resource's: every wiki page shares one.
  expect(pageMemoryScope(wikiPageRef("home"))).toBe("resource:oxplow/wiki");
});

test("the store reads what was kept, writes a change once after a pause, and ignores an unchanged one", async () => {
  const b = backend([{ scope: "app", key: "railWidth", valueJson: "320" }]);
  const store = createRememberedStore(b, 10);
  await store.ready;
  expect(store.get("app", "railWidth")).toBe(320);
  store.set("app", "railWidth", 320);
  store.set("app", "railWidth", 400);
  store.set("app", "railWidth", 410);
  expect(store.get("app", "railWidth")).toBe(410);
  await waitFor(() => expect(b.saves).toEqual([["app", "railWidth", "410"]]));
  store.set("app", "railWidth", undefined);
  await waitFor(() => expect(b.saves.at(-1)).toEqual(["app", "railWidth", null]));
});

function Wide() {
  const [w, setW] = useAppPreference("railWidth", 260);
  return (
    <button type="button" data-testid="w" onClick={() => setW(w + 10)}>
      {w}
    </button>
  );
}

function Mode() {
  const [mode, setMode] = usePageMemory("mergeMode", "merge");
  return (
    <button type="button" data-testid="mode" onClick={() => setMode("rebase")}>
      {mode}
    </button>
  );
}

function Choice() {
  const [v, setV] = usePanelMemory("core:work", "show", "open");
  return (
    <button type="button" data-testid="choice" onClick={() => setV("all")}>
      {v}
    </button>
  );
}

const inTab = (ref: PageNavigation["ref"], children: ReactNode) => (
  <PageNavigationContext.Provider value={{ ref } as PageNavigation}>{children}</PageNavigationContext.Provider>
);

// Every tab of a page shares its memory; another page has its own.
test("page memory is shared by every tab of a page, and app preferences by every page", async () => {
  const b = backend();
  const store = createRememberedStore(b, 10);
  const view = render(
    <RememberedContext.Provider value={store}>
      {inTab(pageTabRef("git-dashboard", { branch: "main" }), <Mode />)}
      {inTab(pageTabRef("git-dashboard", { branch: "topic" }), <div data-testid="other"><Mode /></div>)}
      {inTab(pageTabRef("tasks"), <div data-testid="tasks"><Mode /></div>)}
      <Wide />
      <Choice />
    </RememberedContext.Provider>,
  );
  await act(async () => store.ready);
  fireEvent.click(view.getAllByTestId("mode")[0]!);
  expect(view.getAllByTestId("mode").map((m) => m.textContent)).toEqual(["rebase", "rebase", "merge"]);
  fireEvent.click(view.getByTestId("w"));
  expect(view.getByTestId("w").textContent).toBe("270");
  fireEvent.click(view.getByTestId("choice"));
  await waitFor(() =>
    expect(b.saves.sort()).toEqual([
      ["app", "railWidth", "270"],
      ["oxplow:oxplow/page?page=git-dashboard", "mergeMode", '"rebase"'],
      ["oxplow:oxplow/panel?panel=core:work", "show", '"all"'],
    ]),
  );
});

test("outside a tab, page memory is the component's own", () => {
  const store = createRememberedStore(backend(), 10);
  const view = render(
    <RememberedContext.Provider value={store}>
      <Mode />
    </RememberedContext.Provider>,
  );
  fireEvent.click(view.getByTestId("mode"));
  expect(view.getByTestId("mode").textContent).toBe("rebase");
  expect(store.get("oxplow:oxplow/page?page=tasks", "mergeMode")).toBeUndefined();
});
