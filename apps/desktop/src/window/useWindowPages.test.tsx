import { afterEach, expect, test } from "bun:test";
import { cleanup, renderHook } from "@testing-library/react";
import { emptySession, navigate, open } from "../tabs/session.js";
import { agentSessionRef } from "../tabs/pageRefs.js";
import type { Session } from "../tabs/session.js";

// The shown thread's tabs as every shell draws them: each tab's current
// page, a Back page kept mounted but hidden, an agent session's tab pinned
// with its status, and a page's own title and unsaved edits on its tab.

const { useWindowPages } = await import("./useWindowPages.js");

afterEach(() => cleanup());

const first = { id: "oxplow:oxplow/page?page=e2e/item&uri=1" };
const second = { id: "oxplow:oxplow/page?page=e2e/item&uri=2" };
const agent = agentSessionRef("ses1");

function thread(): Session {
  let s = open(emptySession(), { ref: agent }, { pinned: true });
  s = open(s, { ref: first });
  return navigate(s, s.active!, { ref: second });
}

function setup(over: { pageTitles?: Record<string, string>; dirtyTabs?: Record<string, boolean> } = {}) {
  const noop = () => {};
  return renderHook(() =>
    useWindowPages({
      stream: null,
      selectedThread: { id: "thr1" } as never,
      selectedThreadSessions: [{ id: "ses1", threadId: "thr1", harness: "claude", displayName: "Fixing", title: "", nativeTitle: "" }] as never,
      extensions: null,
      sessionStatuses: { ses1: { status: "working" } } as never,
      harnesses: [],
      liveTerminals: new Set<string>(),
      offerDeps: { run: async () => null } as never,
      bookmarks: [],
      tabs: {
        sessions: { thr1: thread() },
        updateSession: noop,
        openTab: noop,
        closePageTab: noop,
        handleGoBack: noop,
        handleGoForward: noop,
        handleStepOrigin: noop,
        liveItems: {},
        pageTitles: over.pageTitles ?? {},
        setPageTitle: noop,
        dirtyTabs: over.dirtyTabs ?? {},
        tabActions: {},
        editingSetterFor: () => noop,
        actionsSetterFor: () => noop,
      },
      openers: { handleOpenPage: noop, handleNavigateInTab: noop, handleOpenDiffInTab: noop, openRefRef: { current: noop } },
      tabLabelByIdRef: { current: {} },
    }),
  );
}

test("each tab shows its current page, its Back page kept but hidden, the session's tab pinned with its status", () => {
  const tabs = setup().result.current.windowTabs;
  expect(tabs.map((t) => [t.id, !!t.hidden])).toEqual([
    [agent.id, false],
    [first.id, true],
    [second.id, false],
  ]);
  const session = tabs[0]!;
  expect(session.pinned).toBe(true);
  expect(session.agentStatus).toBe("working");
  expect(session.label).toBe("Fixing");
});

test("a page's own title and unsaved edits show on its tab", () => {
  const hook = setup({ pageTitles: { [second.id]: "Item two" }, dirtyTabs: { [second.id]: true } });
  const tab = hook.result.current.windowTabs.find((t) => t.id === second.id)!;
  expect(tab.label).toBe("Item two");
  expect(tab.dirty).toBe(true);
});
