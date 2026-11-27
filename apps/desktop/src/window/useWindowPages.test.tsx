import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, renderHook } from "@testing-library/react";
import { current, emptySession, navigate, open, setTitle } from "../tabs/session.js";
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

let updates: Array<[string, Session]> = [];
const thread1 = thread();

function setup(
  over: {
    pageTitles?: Record<string, string>;
    dirtyTabs?: Record<string, boolean>;
    extensions?: unknown;
    sessions?: () => Record<string, Session>;
    update?: (thread: string, f: (s: Session) => Session) => void;
  } = {},
) {
  const noop = () => {};
  return renderHook(({ name }: { name: string }) =>
    useWindowPages({
      stream: null,
      selectedThread: { id: "thr1" } as never,
      selectedThreadSessions: [{ id: "ses1", threadId: "thr1", harness: "claude", displayName: name, title: "", nativeTitle: "" }] as never,
      extensions: (over.extensions ?? null) as never,
      sessionStatuses: { ses1: { status: "working" } } as never,
      harnesses: [],
      liveTerminals: new Set<string>(),
      offerDeps: { run: async () => null } as never,
      bookmarks: [],
      tabs: {
        sessions: over.sessions ? over.sessions() : { thr1: thread() },
        updateSession:
          over.update ?? ((thread: string, f: (s: Session) => Session) => void updates.push([thread, f(thread1)])),
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
    { initialProps: { name: "Fixing" } },
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

// Agents read the stored tabs: the strip's label is kept in its own field of
// the entry, never in the page's registered title (which the strip prefers).
describe("the strip's label, kept for agents", () => {
  /** The thread's session held as the window holds it: each update applies
   *  to the latest, and a render reads what was stored. */
  function held(over: Parameters<typeof setup>[0] = {}) {
    let stored = thread();
    let writes = 0;
    const hook = setup({
      ...over,
      sessions: () => ({ thr1: stored }),
      update: (_t, f) => {
        const next = f(stored);
        if (next !== stored) writes++;
        stored = next;
      },
    });
    return { hook, stored: () => stored, writes: () => writes };
  }
  const labelOf = (s: Session, id: string) => s.tabs.map((t) => current(t)).find((e) => e.ref.id === id)?.label;

  test("each tab's strip label is stored once, as the entry's label", () => {
    const h = held({ pageTitles: { [second.id]: "Item two" }, extensions: [] });
    expect(labelOf(h.stored(), second.id)).toBe("Item two");
    expect(current(h.stored().tabs.find((t) => current(t).ref.id === second.id)!).title).toBeUndefined();
    const before = h.writes();
    h.hook.rerender({ name: "Fixing" });
    h.hook.rerender({ name: "Fixing" });
    expect(h.writes()).toBe(before);
  });

  test("nothing is stored while the extensions are not loaded: labels are placeholders", () => {
    const h = held({ extensions: null });
    expect(h.writes()).toBe(0);
  });

  test("an agent session's tab follows the session's name, not a stored label", () => {
    const h = held({ extensions: [] });
    expect(labelOf(h.stored(), agent.id)).toBe("Fixing");
    h.hook.rerender({ name: "Renamed" });
    expect(labelOf(h.stored(), agent.id)).toBe("Renamed");
  });

  test("the page's registered title stays the page's: the label never replaces it", () => {
    const h = held({ extensions: [] });
    const withTitle = setTitle(h.stored(), h.stored().tabs[0]!.slot, "Registered");
    expect(current(withTitle.tabs[0]!).title).toBe("Registered");
  });
});
