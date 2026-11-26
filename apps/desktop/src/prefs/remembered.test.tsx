import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { extPageRef, pageTabRef, knowledgePageRef } from "../tabs/pageRefs.js";
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
  // An entity page's is its resource's: every knowledge page shares one.
  expect(pageMemoryScope(knowledgePageRef("oxwiki", "home"))).toBe("resource:oxplow/knowledge");
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

// A value another device (or a forgetting) changed arrives when the store is told to read again.
test("the store reads again when told to, keeping what is waiting to be saved over what comes back", async () => {
  let stored: UiPreference[] = [{ scope: "app", key: "railWidth", valueJson: "320" }];
  let reload = () => {};
  const saves: [string, string, string | null][] = [];
  const store = createRememberedStore(
    {
      load: async () => stored,
      save: async (scope, key, valueJson) => void saves.push([scope, key, valueJson]),
      watch: (run) => {
        reload = run;
        return () => {};
      },
    },
    10,
  );
  await store.ready;
  let heard = 0;
  store.subscribe(() => (heard += 1));
  stored = [
    { scope: "app", key: "railWidth", valueJson: "500" },
    { scope: "app", key: "dockHeight", valueJson: "200" },
  ];
  reload();
  await waitFor(() => expect(store.get("app", "railWidth")).toBe(500));
  expect(store.get("app", "dockHeight")).toBe(200);
  // What the other side forgot is forgotten here.
  stored = [{ scope: "app", key: "railWidth", valueJson: "500" }];
  reload();
  await waitFor(() => expect(store.get("app", "dockHeight")).toBeUndefined());
  // A change waiting to be saved wins over the reloaded value.
  store.set("app", "railWidth", 700);
  stored = [{ scope: "app", key: "railWidth", valueJson: "100" }];
  reload();
  await new Promise((r) => setTimeout(r, 0));
  expect(store.get("app", "railWidth")).toBe(700);
  await waitFor(() => expect(saves).toEqual([["app", "railWidth", "700"]]));
  // Nothing changed: nobody is told.
  stored = [{ scope: "app", key: "railWidth", valueJson: "700" }];
  const before = heard;
  reload();
  await new Promise((r) => setTimeout(r, 0));
  expect(heard).toBe(before);
});

// A read that a newer one replaces doesn't count as loaded: `ready` waits
// for the values to be there, so a component never reads its memory as
// empty and writes its default over it.
test("ready waits for a read that is applied, not the first one", async () => {
  const answers: Array<(rows: UiPreference[]) => void> = [];
  let watch: (() => void) | null = null;
  const store = createRememberedStore({
    load: () => new Promise<UiPreference[]>((resolve) => answers.push(resolve)),
    save: async () => {},
    watch: (reload) => {
      watch = reload;
      return () => {};
    },
  });
  let ready = false;
  void store.ready.then(() => (ready = true));
  watch!(); // the socket opened: a second read replaces the first
  answers[0]!([{ scope: "component:acme/x", key: "k", valueJson: "1" }]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(ready).toBe(false);
  answers[1]!([{ scope: "component:acme/x", key: "k", valueJson: "2" }]);
  await store.ready;
  expect(store.get("component:acme/x", "k")).toBe(2);
});
