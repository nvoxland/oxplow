import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

// The window as the daemon's host: it takes calls only once it is ready,
// an agent's tab open lands in the agent's thread without moving the shown
// one, and the window tells the daemon which thread it shows.

const realHost = { ...(await import("../clientHost.js")) }; // a copy: `mock.module` patches a loaded module in place
let started = 0;
mock.module("../clientHost.js", () => ({
  ...realHost,
  startClientHost: () => {
    started += 1;
    return () => {};
  },
}));
const realApi = { ...(await import("../api.js")) };
const reports: Array<{ shown_thread: string | null }> = [];
let queued: Array<{ id: number; threadId: string; uri: string }> = [];
const forgotten: number[] = [];
const saved: string[] = [];
const eventListeners = new Set<(event: unknown) => void>();
const reconnects = new Set<() => void>();
mock.module("../api.js", () => ({
  ...realApi,
  listTabSessions: async () => [],
  // A read keeps the rows until they are forgotten.
  listQueuedTabs: async () => queued.filter((tab) => !forgotten.includes(tab.id)),
  forgetQueuedTabs: async (ids: number[]) => void forgotten.push(...ids),
  subscribeOxplowEvents: (fn: (event: unknown) => void) => {
    eventListeners.add(fn);
    return () => eventListeners.delete(fn);
  },
  onRemoteReconnect: (fn: () => void) => {
    reconnects.add(fn);
    return () => reconnects.delete(fn);
  },
  saveTabSession: async (thread: string) => {
    saved.push(thread);
    return { saved: true, revision: 1 };
  },
  forgetTabSession: async () => {},
  reportTabs: async (report: { shown_thread: string | null }) => {
    reports.push(report);
    return { missingRuns: [] };
  },
}));
const { getUiClientId } = await import("../uiClientId.js");
const { useWindowTabs } = await import("./useWindowTabs.js");
const { useWindowHost } = await import("./useWindowHost.js");

const stream = { id: "str1", title: "s", branch: "main", created_at: "1", worktree_path: "/w" } as never;
const page = (n: number) => ({ id: `oxplow:oxplow/page?page=e2e/item&uri=${n}` });

function setup(ready: { extensions: unknown[] | null }) {
  return renderHook(
    ({ extensions }: { extensions: unknown[] | null }) => {
      const tabs = useWindowTabs({ selectedThreadId: "thr1", stream, threadStates: {}, selectedThreadSessions: null });
      const host = useWindowHost({
        stream,
        streams: [stream],
        threadStates: { str1: { selectedThreadId: "thr1", threads: [{ id: "thr1" }, { id: "thr2" }] } } as never,
        selectedThreadId: "thr1",
        selectedThreadSessions: [],
        extensions: extensions as never,
        tabs,
        openers: { handleOpenPageRef: { current: (ref) => tabs.openTab(ref) }, handleNavigateInTab: () => {} },
        switchStream: async () => {},
        selectThread: async () => {},
        view: {
          effectiveCenterActive: tabs.centerActive,
          visitContextRef: { current: { pageTabs: [] } } as never,
        },
        openQuickOpen: () => {},
      });
      return { tabs, host };
    },
    { initialProps: ready },
  );
}

beforeEach(() => {
  started = 0;
  reports.length = 0;
  queued = [];
  forgotten.length = 0;
  saved.length = 0;
});
afterEach(() => cleanup());

test("the window hosts the daemon's calls only once it is ready", async () => {
  const hook = setup({ extensions: null });
  // The shown thread's first tabs are there once it has a tab list.
  act(() => hook.result.current.tabs.openTab(page(1)));
  expect(started).toBe(0);
  hook.rerender({ extensions: [] });
  await waitFor(() => expect(started).toBe(1));
});

test("an agent's tab open lands in its own thread and leaves the shown tab", async () => {
  const hook = setup({ extensions: [] });
  act(() => hook.result.current.tabs.openTab(page(1)));
  const open = hook.result.current.host.windowHandlersRef.current["tabs.write"]!.open!;
  await act(async () => {
    await open({ uri: page(2).id }, { threadId: "thr2", actor: "agent", call: null });
  });
  expect(hook.result.current.tabs.threadPageTabs.thr2.map((t) => t.id)).toEqual([page(2).id]);
  expect(hook.result.current.tabs.centerActive).toBe(page(1).id);
});

test("the window reports the thread it shows", async () => {
  setup({ extensions: [] });
  await waitFor(() => expect(reports.at(-1)?.shown_thread).toBe("thr1"));
});

test("the tabs agents opened while the device was away open in their threads, now and after a reconnect", async () => {
  queued = [{ id: 1, threadId: "thr2", uri: page(2).id }];
  const hook = setup({ extensions: [] });
  act(() => hook.result.current.tabs.openTab(page(1)));
  await waitFor(() => expect(hook.result.current.tabs.threadPageTabs.thr2?.map((t) => t.id)).toEqual([page(2).id]));
  // Opened without focus: the shown tab stays.
  expect(hook.result.current.tabs.centerActive).toBe(page(1).id);
  // The row goes only after the thread's tabs were stored.
  await waitFor(() => expect(forgotten).toEqual([1]));
  expect(saved).toContain("thr2");
  queued.push({ id: 2, threadId: "thr2", uri: page(3).id });
  await act(async () => {
    for (const fn of reconnects) fn();
  });
  await waitFor(() => expect(hook.result.current.tabs.threadPageTabs.thr2.map((t) => t.id)).toEqual([page(2).id, page(3).id]));
  await waitFor(() => expect(forgotten).toEqual([1, 2]));
});

test("a tab queued just after the window connected is taken when the daemon says so", async () => {
  const hook = setup({ extensions: [] });
  act(() => hook.result.current.tabs.openTab(page(1)));
  await waitFor(() => expect(started).toBe(1));
  queued = [{ id: 7, threadId: "thr2", uri: page(4).id }];
  await act(async () => {
    // Another window's event is not ours.
    for (const fn of eventListeners) fn({ kind: "tabsQueued", client: "con-someone-else" });
  });
  await new Promise((r) => setTimeout(r, 20));
  expect(hook.result.current.tabs.threadPageTabs.thr2).toBeUndefined();
  await act(async () => {
    for (const fn of eventListeners) fn({ kind: "tabsQueued", client: getUiClientId() });
  });
  await waitFor(() => expect(hook.result.current.tabs.threadPageTabs.thr2?.map((t) => t.id)).toEqual([page(4).id]));
  await waitFor(() => expect(forgotten).toEqual([7]));
});
