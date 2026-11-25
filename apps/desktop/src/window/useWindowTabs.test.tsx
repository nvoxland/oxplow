import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

// The shown thread's tabs: opening, switching and closing a tab, the tab
// cap, and a new thread's first tabs.

const opening = { tabs: [{ id: "oxplow:oxplow/page?page=new-session" }] };
let openings = 0;
mock.module("../newThreadSession.js", () => ({
  newThreadOpening: async () => {
    openings += 1;
    return opening;
  },
}));
const realApiForTabs = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../api.js", () => ({
  ...realApiForTabs,
  // This device has no stored tabs yet; saves are kept nowhere.
  listTabSessions: async () => [],
  saveTabSession: async () => 1,
  forgetTabSession: async () => {},
}));
const { HOME_TAB, useWindowTabs } = await import("./useWindowTabs.js");
const { MAX_PAGE_TABS } = await import("../tabs/session.js");

const page = (n: number) => ({ id: `oxplow:oxplow/page?page=e2e/item&uri=${n}` });
type Sessions = Parameters<typeof useWindowTabs>[0]["selectedThreadSessions"];

function setup(selectedThreadSessions: Sessions) {
  return renderHook(() =>
    useWindowTabs({ selectedThreadId: "thr1", stream: null, threadStates: {}, selectedThreadSessions }),
  );
}

beforeEach(() => {
  openings = 0;
});
afterEach(() => cleanup());

test("an opened tab becomes active, another can be chosen, and a closed one goes with its title", () => {
  const hook = setup(null);
  expect(hook.result.current.centerActive).toBe(HOME_TAB);
  act(() => hook.result.current.openTab(page(1)));
  act(() => hook.result.current.openTab(page(2)));
  expect(hook.result.current.centerActive).toBe(page(2).id);
  act(() => hook.result.current.setCenterActive(page(1).id));
  expect(hook.result.current.centerActive).toBe(page(1).id);
  act(() => hook.result.current.setPageTitle(page(1).id, "One"));
  act(() => hook.result.current.closePageTab(page(1).id));
  expect(hook.result.current.threadPageTabs.thr1.map((t) => t.id)).toEqual([page(2).id]);
  expect(hook.result.current.pageTitles[page(1).id]).toBeUndefined();
});

test("past the tab cap the least recently used page tab closes", () => {
  const hook = setup(null);
  for (let n = 1; n <= MAX_PAGE_TABS + 1; n += 1) act(() => hook.result.current.openTab(page(n)));
  const ids = hook.result.current.threadPageTabs.thr1.map((t) => t.id);
  expect(ids).toHaveLength(MAX_PAGE_TABS);
  expect(ids).not.toContain(page(1).id);
});

test("a thread first seen with no agent session opens a new thread's first tabs", async () => {
  const hook = setup([]);
  await waitFor(() => expect(hook.result.current.threadPageTabs.thr1?.map((t) => t.id)).toEqual([opening.tabs[0].id]));
  expect(openings).toBe(1);
  expect(hook.result.current.openingThread).toBeNull();
});

test("with no tab chosen the thread's home tab shows: its first agent session's, else its first tab", () => {
  const hook = setup(null);
  act(() => hook.result.current.openTab(page(1)));
  act(() => hook.result.current.openTab({ id: "oxplow:oxplow/agent_session?id=ses1", pinned: true }));
  act(() => hook.result.current.setCenterActive(HOME_TAB));
  expect(hook.result.current.centerActive).toBe(HOME_TAB);
  expect(hook.result.current.effectiveCenterActive).toBe("oxplow:oxplow/agent_session?id=ses1");
});
