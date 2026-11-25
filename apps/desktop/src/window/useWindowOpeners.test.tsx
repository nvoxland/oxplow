import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

// The window's openers over its tabs: a page opens a tab, a working-tree
// file opens only once its buffer reads (a missing one says so), and a
// page opened in a tab is a step Back returns from.

const realOpenFiles = { ...(await import("../editor/openFiles.js")) }; // a copy: `mock.module` patches a loaded module in place
const missing = new Set<string>();
mock.module("../editor/openFiles.js", () => ({
  ...realOpenFiles,
  buffers: {
    ...realOpenFiles.buffers,
    open: async (_stream: unknown, path: string) =>
      missing.has(path) ? { state: "missing" } : { state: "ready" },
  },
}));
mock.module("../workspaceFiles.js", () => ({ isWorkspaceDir: async () => false }));
const realApiForTabs = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../api.js", () => ({
  ...realApiForTabs,
  // This device has no stored tabs yet; saves are kept nowhere.
  listTabSessions: async () => [],
  saveTabSession: async () => 1,
  forgetTabSession: async () => {},
}));
const { useWindowTabs } = await import("./useWindowTabs.js");
const { useWindowOpeners } = await import("./useWindowOpeners.js");

const stream = { id: "str1", title: "s", branch: "main", created_at: "1", worktree_path: "/w" } as never;
const thread = { id: "thr1", title: "t" } as never;
const page = (n: number) => ({ id: `oxplow:oxplow/page?page=e2e/item&uri=${n}` });

function setup() {
  const errors: Array<string | null> = [];
  const hook = renderHook(() => {
    const tabs = useWindowTabs({ selectedThreadId: "thr1", stream, threadStates: {}, selectedThreadSessions: null });
    const openers = useWindowOpeners({
      stream,
      selectedThread: thread,
      selectedThreadSessions: null,
      setError: (e) => errors.push(e),
      streams: {
        threadStatesRef: { current: {} },
        selectThread: async () => {},
        streamCreated: async () => {},
        switchStream: async () => {},
      },
      tabs,
    });
    return { tabs, openers };
  });
  return { hook, errors };
}

beforeEach(() => {
  missing.clear();
});
afterEach(() => cleanup());

test("a page opens its tab, and the opener ref reaches the same opener", async () => {
  const { hook } = setup();
  act(() => hook.result.current.openers.handleOpenPage(page(1)));
  expect(hook.result.current.tabs.centerActive).toBe(page(1).id);
  await waitFor(() => expect(hook.result.current.openers.handleOpenPageRef.current).not.toBeNull());
  act(() => hook.result.current.openers.handleOpenPageRef.current!(page(2)));
  expect(hook.result.current.tabs.centerActive).toBe(page(2).id);
});

test("a working-tree file opens once its buffer reads; a missing one opens nothing and says so", async () => {
  const { hook, errors } = setup();
  missing.add("gone.txt");
  await act(() => hook.result.current.openers.handleOpenFile("gone.txt"));
  expect(hook.result.current.tabs.threadPageTabs.thr1 ?? []).toEqual([]);
  expect(errors).toContain("File not found: gone.txt");
  await act(() => hook.result.current.openers.handleOpenFile("here.txt"));
  expect(hook.result.current.tabs.threadPageTabs.thr1.map((t) => t.id)).toEqual(["oxplow:oxplow/file?path=here.txt"]);
});

test("a page opened in a tab is a step Back returns from", () => {
  const { hook } = setup();
  act(() => hook.result.current.openers.handleOpenPage(page(1)));
  act(() => hook.result.current.openers.handleNavigateInTab(page(1).id, page(2)));
  expect(hook.result.current.tabs.threadPageTabs.thr1.map((t) => t.id)).toEqual([page(2).id]);
  const slot = hook.result.current.tabs.sessions.thr1.active!;
  act(() => hook.result.current.tabs.handleGoBack(slot));
  expect(hook.result.current.tabs.centerActive).toBe(page(1).id);
});
