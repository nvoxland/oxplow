import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";
import { useCollapsedSet, useDraft, useViewList, useViewParam, useViewSelection } from "./useViewParam.js";

afterEach(cleanup);

type Patch = [Record<string, SqlCell | undefined>, { push?: boolean } | undefined];

function nav(viewParams: Record<string, SqlCell>, patches: Patch[], historyState: Record<string, unknown> = {}, states: Record<string, unknown>[] = []): PageNavigation {
  return {
    viewParams,
    patchViewParams: (patch, opts) => patches.push([patch, opts]),
    historyState,
    setHistoryState: (patch) => states.push(patch),
  } as unknown as PageNavigation;
}

const hooks: Record<string, unknown> = {};
function Probe() {
  hooks.range = useViewParam("range", "30d");
  hooks.days = useViewParam("days", 7);
  hooks.merges = useViewParam("merges", false);
  hooks.task = useViewSelection("task");
  hooks.authors = useViewList("authors");
  hooks.open = useCollapsedSet("open");
  hooks.draft = useDraft("draft", { title: "" });
  return null;
}
const set = (name: string, value: unknown) => act(() => (hooks[name] as [unknown, (v: unknown) => void])[1](value));
const value = (name: string) => (hooks[name] as [unknown])[0];

// A view param is a filter: what the page shows, in its tab's address; a
// change replaces the tab's entry and a value at its default leaves the
// address.
test("a view param reads its tab's value back in its own type, and writes a change as a filter", () => {
  const patches: Patch[] = [];
  render(
    <PageNavigationContext.Provider value={nav({ range: "7d", days: "14", merges: "true" }, patches)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect([value("range"), value("days"), value("merges")]).toEqual(["7d", 14, true]);
  set("range", "90d");
  set("days", 7);
  set("merges", false);
  expect(patches).toEqual([
    [{ range: "90d" }, undefined],
    [{ days: undefined }, undefined],
    [{ merges: undefined }, undefined],
  ]);
  // Its current value writes nothing.
  patches.length = 0;
  set("range", "7d");
  expect(patches).toEqual([]);
});

// "View all" is a step Back returns from, not a filter.
test("a view param said to push pushes every change", () => {
  const patches: Patch[] = [];
  function All() {
    hooks.all = useViewParam("all", false, { push: true });
    return null;
  }
  render(
    <PageNavigationContext.Provider value={nav({}, patches)}>
      <All />
    </PageNavigationContext.Provider>,
  );
  set("all", true);
  expect(patches).toEqual([[{ all: true }, { push: true }]]);
});

test("a view param missing or unreadable is its default", () => {
  render(
    <PageNavigationContext.Provider value={nav({ days: "soon" }, [])}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect([value("range"), value("days"), value("merges")]).toEqual(["30d", 7, false]);
});

// A selection is a pick: somewhere to come back from, so it pushes; a
// cleared one replaces.
test("a view selection pushes a pick and replaces a clear", () => {
  const patches: Patch[] = [];
  render(
    <PageNavigationContext.Provider value={nav({ task: "tsk1" }, patches)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(value("task")).toBe("tsk1");
  set("task", "tsk2");
  set("task", null);
  expect(patches).toEqual([
    [{ task: "tsk2" }, { push: true }],
    [{ task: undefined }, undefined],
  ]);
});

test("a view list is its values, comma-joined in the address", () => {
  const patches: Patch[] = [];
  render(
    <PageNavigationContext.Provider value={nav({ authors: "ann,bo" }, patches)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(value("authors")).toEqual(["ann", "bo"]);
  set("authors", ["bo"]);
  set("authors", []);
  expect(patches).toEqual([
    [{ authors: "bo" }, undefined],
    [{ authors: undefined }, undefined],
  ]);
});

// What only this step of the tab keeps: open groups and an unsaved draft.
test("a collapsed set and a draft live in the tab's history state", () => {
  const states: Record<string, unknown>[] = [];
  render(
    <PageNavigationContext.Provider value={nav({}, [], { open: ["a"], draft: { title: "half" } }, states)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  const open = hooks.open as ReturnType<typeof useCollapsedSet>;
  expect([open.has("a"), open.has("b")]).toEqual([true, false]);
  act(() => open.toggle("b"));
  expect(value("draft")).toEqual({ title: "half" });
  act(() => (hooks.draft as ReturnType<typeof useDraft>)[2]());
  expect(states).toEqual([{ open: ["a", "b"] }, { draft: { title: "" } }]);
});

test("a collapsed set starts from its initial ids and lists what it holds", () => {
  let seen: ReturnType<typeof useCollapsedSet> | null = null;
  function Tree() {
    seen = useCollapsedSet("dirs", [""]);
    return null;
  }
  render(<Tree />);
  expect(seen!.ids).toEqual([""]);
  expect(seen!.has("")).toBe(true);
  act(() => seen!.set(["", "src"]));
  expect(seen!.ids).toEqual(["", "src"]);
});

test("outside a tab, each is the page's own", () => {
  render(<Probe />);
  set("range", "90d");
  set("task", "tsk9");
  set("authors", ["ann"]);
  expect([value("range"), value("task"), value("authors")]).toEqual(["90d", "tsk9", ["ann"]]);
});
