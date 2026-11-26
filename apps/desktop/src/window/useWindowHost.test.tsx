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
let queued: Array<{ threadId: string; uri: string }> = [];
const reconnects = new Set<() => void>();
mock.module("../api.js", () => ({
  ...realApi,
  listTabSessions: async () => [],
  takeQueuedTabs: async () => queued.splice(0),
  onRemoteReconnect: (fn: () => void) => {
    reconnects.add(fn);
    return () => reconnects.delete(fn);
  },
  saveTabSession: async () => ({ saved: true, revision: 1 }),
  forgetTabSession: async () => {},
  reportTabs: async (report: { shown_thread: string | null }) => {
    reports.push(report);
    return { missingRuns: [] };
  },
}));
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
          tabLabelByIdRef: { current: {} },
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
  queued = [{ threadId: "thr2", uri: page(2).id }];
  const hook = setup({ extensions: [] });
  act(() => hook.result.current.tabs.openTab(page(1)));
  await waitFor(() => expect(hook.result.current.tabs.threadPageTabs.thr2?.map((t) => t.id)).toEqual([page(2).id]));
  // Opened without focus: the shown tab stays.
  expect(hook.result.current.tabs.centerActive).toBe(page(1).id);
  queued.push({ threadId: "thr2", uri: page(3).id });
  await act(async () => {
    for (const fn of reconnects) fn();
  });
  await waitFor(() => expect(hook.result.current.tabs.threadPageTabs.thr2.map((t) => t.id)).toEqual([page(2).id, page(3).id]));
});
