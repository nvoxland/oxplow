import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { useHostedView, type HostedView } from "./hostedView.js";

afterEach(cleanup);

function nav(viewParams: Record<string, SqlCell>, historyState: Record<string, unknown>, states: Record<string, unknown>[]): PageNavigation {
  return { viewParams, historyState, setHistoryState: (p) => states.push(p) } as unknown as PageNavigation;
}

let seen: HostedView | null = null;
function Probe(props: Parameters<typeof useHostedView>[0]) {
  seen = useHostedView(props);
  return null;
}

// A replacement shares the keys its target's contract lists, and keeps
// its own history state apart from its page's.
test("a hosted view is the listed keys of its page's view, and its own history state", () => {
  const states: Record<string, unknown>[] = [];
  const writes: Record<string, string | null>[] = [];
  render(
    <PageNavigationContext.Provider
      value={nav({ q: "fix", days: 7, sha: "abc" }, { picked: 1, "component:vcs.history.graph": { scroll: 4 } }, states)}
    >
      <Probe keys={["q", "author", "days"]} scope="vcs.history.graph" write={(p) => writes.push(p)} />
    </PageNavigationContext.Provider>,
  );
  expect(seen!.viewParams).toEqual({ q: "fix", days: "7" });
  expect(seen!.historyState).toEqual({ scroll: 4 });
  act(() => seen!.setViewParams({ q: null, author: "ann" }));
  expect(writes).toEqual([{ q: null, author: "ann" }]);
  act(() => seen!.setHistoryState({ open: ["a"] }));
  expect(states).toEqual([{ "component:vcs.history.graph": { scroll: 4, open: ["a"] } }]);
});

test("a key outside a hosted view is refused, and nothing is written", () => {
  const writes: Record<string, string | null>[] = [];
  render(
    <PageNavigationContext.Provider value={nav({}, {}, [])}>
      <Probe keys={["q", "author", "branch"]} scope="vcs.history.graph" write={(p) => writes.push(p)} />
    </PageNavigationContext.Provider>,
  );
  expect(() => seen!.setViewParams({ q: "x", sha: "abc" })).toThrow(
    "`sha` isn't in vcs.history.graph's view (q, author, branch).",
  );
  expect(writes).toEqual([]);
});

// By default a change merges into the tab's entry, a null dropping a key.
test("a hosted view writes by patching the tab's view", () => {
  const patches: unknown[] = [];
  const n = { ...nav({}, {}, []), patchViewParams: (p: unknown) => patches.push(p) } as unknown as PageNavigation;
  render(
    <PageNavigationContext.Provider value={n}>
      <Probe keys={["q", "author"]} scope="vcs.history.graph" />
    </PageNavigationContext.Provider>,
  );
  act(() => seen!.setViewParams({ q: "fix", author: null }));
  expect(patches).toEqual([{ q: "fix", author: undefined }]);
});

// A component that is the page has the whole entry.
test("a page's view is all of its params and the whole history state", () => {
  const states: Record<string, unknown>[] = [];
  render(
    <PageNavigationContext.Provider value={nav({ from: "2026-10-01" }, { scroll: 3 }, states)}>
      <Probe keys="all" scope={null} write={() => {}} />
    </PageNavigationContext.Provider>,
  );
  expect(seen!.viewParams).toEqual({ from: "2026-10-01" });
  expect(seen!.historyState).toEqual({ scroll: 3 });
  act(() => seen!.setHistoryState({ scroll: 9 }));
  expect(states).toEqual([{ scroll: 9 }]);
});

test("outside a tab, or with no keys, there is no view", () => {
  render(<Probe keys={["q"]} scope="x" write={() => {}} />);
  expect(seen).toBeNull();
  render(
    <PageNavigationContext.Provider value={nav({ q: "a" }, {}, [])}>
      <Probe keys={[]} scope="x" write={() => {}} />
    </PageNavigationContext.Provider>,
  );
  expect(seen).toBeNull();
});
