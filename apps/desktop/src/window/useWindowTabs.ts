import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Stream, ThreadState } from "../api.js";
import type { AgentSessionRow } from "../agentSessions.js";
import { isDirty } from "../editor/buffers.js";
import { buffers } from "../editor/openFiles.js";
import type { PageAction } from "../kit/actions.js";
import { logUi } from "../logger.js";
import { newThreadOpening } from "../newThreadSession.js";
import { liveItemsOf } from "../tabs/liveOrigin.js";
import { agentSessionRef, diskFilePath, isPage, newSessionRef } from "../tabs/pageRefs.js";
import { typeOf } from "../uri/uri.js";
import type { TabEditing } from "../tabs/PageNavigationContext.js";
import {
  activate,
  activeRef,
  back as sessionBack,
  close as sessionClose,
  current as currentEntry,
  evictions,
  forward as sessionForward,
  MAX_PAGE_TABS,
  open as sessionOpen,
  refsOf,
  reorder as sessionReorder,
  setStrip,
  slotShowing,
  stepOrigin,
  type OriginItem,
} from "../tabs/session.js";
import { reconcileSessionTabs, storedSessionIsStale } from "../tabs/sessionTabs.js";
import type { TabRef } from "../tabs/tabState.js";
import { streamOfThread } from "../tabs/threadTabOps.js";
import { useTabSessions } from "../tabs/useTabSessions.js";

/** No tab chosen: the thread's home tab shows — its first agent session's,
 *  else the session picker if it's open, else its first tab (`homeTabOf`). */
export const HOME_TAB = "";

export interface WindowTabsOptions {
  /** The thread the window shows. */
  selectedThreadId: string | null;
  /** The stream it is in (a file's unsaved edits are kept by stream). */
  stream: Stream | null;
  /** Every stream's threads: which stream a thread's files belong to. */
  threadStates: Record<string, ThreadState>;
  /** The shown thread's open agent sessions; null until they are read. */
  selectedThreadSessions: AgentSessionRow[] | null;
}

/** The window's tabs: every thread's session of tabs (`tabs/session.ts`),
 *  what each tab registers (its title, unsaved edits, Save, Find, actions),
 *  the shown thread's active tab and the operations on its tabs, its agent
 *  sessions' tabs, a new thread's first tabs, the tab cap and the file
 *  buffers the tabs keep. Opening a page by its ref, which needs the
 *  window's openers, is not here. Shell-neutral: a shell draws its strip
 *  from it. */
export function useWindowTabs({ selectedThreadId, stream, threadStates, selectedThreadSessions }: WindowTabsOptions) {
  // Every thread's center tabs: a session of entries per thread
  // (`tabs/session.ts`), restored on start and stored on every change.
  // The strip, the bodies, history, origins and the active tab all come
  // from it (`.context/pages-and-tabs.md` "Navigation model").
  const { sessions, update: updateSession, forget: forgetSession } = useTabSessions();
  // Per-tab page titles, keyed by tab id. Pages register their title via
  // PageNavigationContext.setTitle (the usePageTitle helper). Drives both
  // the tab strip label and the shared chrome header so the title lives in
  // exactly one place.
  const [pageTitles, setPageTitles] = useState<Record<string, string>>({});
  const setPageTitle = useCallback((tabId: string, title: string) => {
    setPageTitles((prev) => (prev[tabId] === title ? prev : { ...prev, [tabId]: title }));
  }, []);
  // Per-tab editing (`useTabEditing`), keyed by tab id: which tabs hold
  // unsaved edits (the ●, `pageDirty`, the tab cap) and their Saves. One
  // setter per tab, kept, so a body's registration doesn't re-run on
  // every render of the strip.
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});
  const tabSavesRef = useRef<Record<string, () => void>>({});
  const tabFindsRef = useRef<Record<string, () => void>>({});
  const editingSettersRef = useRef<Record<string, (editing: TabEditing | null) => void>>({});
  const editingSetterFor = useCallback((tabId: string) => {
    const known = editingSettersRef.current[tabId];
    if (known) return known;
    const set = (editing: TabEditing | null) => {
      if (editing) {
        tabSavesRef.current[tabId] = editing.save;
        if (editing.find) tabFindsRef.current[tabId] = editing.find;
        else delete tabFindsRef.current[tabId];
        setDirtyTabs((prev) => (prev[tabId] === editing.dirty ? prev : { ...prev, [tabId]: editing.dirty }));
      } else {
        delete tabSavesRef.current[tabId];
        delete tabFindsRef.current[tabId];
        setDirtyTabs((prev) => {
          if (!(tabId in prev)) return prev;
          const next = { ...prev };
          delete next[tabId];
          return next;
        });
      }
    };
    editingSettersRef.current[tabId] = set;
    return set;
  }, []);
  // Per-tab page actions (`useTabActions`): a page's actions are its tab's
  // right-click menu too. One setter per tab, kept.
  const [tabActions, setTabActionsState] = useState<Record<string, readonly PageAction[]>>({});
  const actionSettersRef = useRef<Record<string, (actions: readonly PageAction[] | null) => void>>({});
  const actionsSetterFor = useCallback((tabId: string) => {
    const known = actionSettersRef.current[tabId];
    if (known) return known;
    const set = (actions: readonly PageAction[] | null) =>
      setTabActionsState((prev) => {
        if (!actions) {
          if (!(tabId in prev)) return prev;
          const next = { ...prev };
          delete next[tabId];
          return next;
        }
        return { ...prev, [tabId]: actions };
      });
    actionSettersRef.current[tabId] = set;
    return set;
  }, []);

  // Each thread's tabs as the pages they show now, in strip order.
  const threadPageTabs = useMemo(
    () => Object.fromEntries(Object.entries(sessions).map(([thread, session]) => [thread, refsOf(session)])),
    [sessions],
  );
  // The shown thread's active tab's page id; `HOME_TAB` when none is
  // chosen (the thread's home tab shows — `effectiveCenterActive`).
  const centerActive = (selectedThreadId ? activeRef(sessions[selectedThreadId])?.id : null) ?? HOME_TAB;
  // The shown thread's tabs, and the tab it shows: the chosen one, else its
  // home tab.
  const pageTabsForActiveThread = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
  const availableCenterIds = useMemo(() => new Set(pageTabsForActiveThread.map((ref) => ref.id)), [pageTabsForActiveThread]);
  const effectiveCenterActive = availableCenterIds.has(centerActive)
    ? centerActive
    : homeTabOf(pageTabsForActiveThread);
  // The working-tree file the shown tab edits, if it does.
  const shownFilePath = diskFilePath(centerActive);
  /** Make the tab showing `id` active in the shown thread (`HOME_TAB`:
   *  none). An id no tab shows changes nothing. */
  const setCenterActive = useCallback(
    (next: string | ((prev: string) => string)) => {
      if (!selectedThreadId) return;
      updateSession(selectedThreadId, (session) => {
        const now = activeRef(session)?.id ?? HOME_TAB;
        const id = typeof next === "function" ? next(now) : next;
        if (id === now) return session;
        if (id === HOME_TAB) return activate(session, null);
        const slot = slotShowing(session, id);
        return slot ? activate(session, slot) : session;
      });
    },
    [selectedThreadId, updateSession],
  );
  /** Show `ref` in the shown thread: the tab showing it (its ref
   *  refreshed), else a new one; it becomes active. */
  const openTab = useCallback(
    (ref: TabRef, opts: { historyState?: Record<string, unknown> } = {}) => {
      if (!selectedThreadId) return;
      const entry = opts.historyState ? { ref, historyState: opts.historyState } : { ref };
      updateSession(selectedThreadId, (session) => sessionOpen(session, entry, { pinned: !!ref.pinned }));
    },
    [selectedThreadId, updateSession],
  );

  // A session just opened, to show once the window's list of sessions has it.
  const pendingSessionRef = useRef<string | null>(null);
  const [pendingSessionTick, setPendingSessionTick] = useState(0);
  useEffect(() => {
    const id = pendingSessionRef.current;
    if (id && selectedThreadSessions?.some((candidate) => candidate.id === id)) {
      pendingSessionRef.current = null;
      setCenterActive(agentSessionRef(id).id);
    }
  }, [selectedThreadSessions, pendingSessionTick]);

  const handleReorderCenterTabs = useCallback((orderedIds: string[]) => {
    // The session's tabs are the strip's order for every kind; pinned tabs
    // never move (the strip won't drag them), and `reconcileSessionTabs`
    // keeps them first.
    if (selectedThreadId) {
      updateSession(selectedThreadId, (session) =>
        sessionReorder(session, orderedIds.flatMap((id) => slotShowing(session, id) ?? [])),
      );
    }
  }, [selectedThreadId, updateSession]);

  // A thread's agent-session tabs follow its open sessions (their rows):
  // each open one's tab leads, a closed one's goes. A thread seen here for
  // the first time (no tab list yet — they persist) with no session opens
  // the session picker, where the person chooses which agent to start.
  // The thread whose first tabs are still being opened: the window isn't
  // ready for a navigate until they are, or they would push it aside.
  const [openingThread, setOpeningThread] = useState<string | null>(null);
  // The threads whose stored tabs were checked against their open sessions.
  const checkedStored = useRef(new Set<string>());
  useEffect(() => {
    if (!selectedThreadId || selectedThreadSessions === null) return;
    // Tabs the browser kept from before a project reset can name sessions the
    // thread never had (a new thread gets an old one's id): forget them once,
    // when the thread is first seen, and open it as a new thread.
    const stored = sessions[selectedThreadId];
    if (!checkedStored.current.has(selectedThreadId)) {
      checkedStored.current.add(selectedThreadId);
      if (stored !== undefined && storedSessionIsStale(stored, selectedThreadSessions)) {
        forgetSession(selectedThreadId);
        return;
      }
    }
    if (sessions[selectedThreadId] === undefined && selectedThreadSessions.length === 0) {
      const threadId = selectedThreadId;
      updateSession(threadId, (session) => session);
      setOpeningThread(threadId);
      void openNewThread(threadId).finally(() => setOpeningThread((now) => (now === threadId ? null : now)));
      return;
    }
    updateSession(selectedThreadId, (session) => setStrip(session, reconcileSessionTabs(refsOf(session), selectedThreadSessions)));
  }, [selectedThreadId, selectedThreadSessions, sessions, updateSession, forgetSession]);

  async function openNewThread(threadId: string) {
    try {
      const opening = await newThreadOpening();
      if (opening.tabs.length > 0) {
        updateSession(threadId, (session) =>
          refsOf(session).some((t) => isPage(t, "new-session")) ? session : setStrip(session, [...opening.tabs, ...refsOf(session)]),
        );
      }

    } catch (error) {
      // Couldn't read the choice or start the agent: ask instead.
      logUi("warn", "opening a new thread failed; showing the session picker", { error: String(error) });
      updateSession(threadId, (session) =>
        refsOf(session).some((t) => isPage(t, "new-session")) ? session : setStrip(session, [newSessionRef(), ...refsOf(session)]),
      );
    }
  }

  /** Close the shown thread's tab showing `id`: its title goes with it,
   *  and a file's buffer once no tab shows the file (`buffers.retainOnly`). */
  const closePageTab = useCallback((id: string) => {
    if (!selectedThreadId) return;
    updateSession(selectedThreadId, (session) => {
      const slot = slotShowing(session, id);
      return slot ? sessionClose(session, slot) : session;
    });
    setPageTitles((prev) => {
      if (!(id in prev)) return prev;
      const { [id]: _drop, ...rest } = prev;
      return rest;
    });
  }, [selectedThreadId, updateSession]);

  /** Show item `index` of the collection the tab was opened from, in
   *  place: history untouched. */
  // The pages a live origin (a viz node's link column) lists now, by tab slot.
  const [liveItems, setLiveItems] = useState<Record<string, { key: string; items: OriginItem[] }>>({});
  const liveItemsRef = useRef(liveItems);
  liveItemsRef.current = liveItems;
  const handleStepOrigin = useCallback((slot: string, index: number) => {
    if (!selectedThreadId) return;
    updateSession(selectedThreadId, (session) => stepOrigin(session, slot, index, liveItemsOf(liveItemsRef.current, slot, currentEntry(session.tabs.find((t) => t.slot === slot)!).from)));
  }, [selectedThreadId, updateSession]);

  const handleGoBack = useCallback((slot: string) => {
    if (selectedThreadId) updateSession(selectedThreadId, (session) => sessionBack(session, slot));
  }, [selectedThreadId, updateSession]);

  const handleGoForward = useCallback((slot: string) => {
    if (selectedThreadId) updateSession(selectedThreadId, (session) => sessionForward(session, slot));
  }, [selectedThreadId, updateSession]);

  // Enforce the page-tab cap on the shown thread: past MAX_PAGE_TABS the
  // least recently used tabs close (`evictions`). The active tab, pinned
  // ones and files with unsaved changes stay (a soft cap never discards
  // work). Eviction is closePageTab — the same cleanup a manual close does.
  useEffect(() => {
    if (!selectedThreadId) return;
    const session = sessions[selectedThreadId];
    if (!session) return;
    const dirty = (id: string) => {
      const path = diskFilePath(id);
      return !!dirtyTabs[id] || (path !== null && !!stream && isDirty(buffers.get(stream.id, path)));
    };
    const victims = evictions(session, MAX_PAGE_TABS, (t) => dirty(currentEntry(t).ref.id));
    for (const slot of victims) {
      const tab = session.tabs.find((t) => t.slot === slot);
      if (tab) closePageTab(currentEntry(tab).ref.id);
    }
  }, [sessions, selectedThreadId, stream, closePageTab, dirtyTabs]);

  // A file's buffer lives while a tab of any thread shows the file (any
  // step of its history): the rest close, their models disposed. Not
  // until every thread with tabs is placed in a stream, so no buffer is
  // dropped for a thread not yet loaded.
  useEffect(() => {
    const keep: { stream: string; path: string }[] = [];
    for (const [thread, session] of Object.entries(sessions)) {
      const files = session.tabs.flatMap((tab) => tab.entries.flatMap((entry) => diskFilePath(entry.ref.id) ?? []));
      if (files.length === 0) continue;
      const streamId = streamOfThread(threadStates, thread);
      if (!streamId) return;
      for (const path of files) keep.push({ stream: streamId, path });
    }
    buffers.retainOnly(keep);
  }, [sessions, threadStates]);

  return {
    sessions,
    updateSession,
    pageTitles,
    setPageTitle,
    dirtyTabs,
    tabSavesRef,
    tabFindsRef,
    editingSetterFor,
    tabActions,
    actionsSetterFor,
    threadPageTabs,
    centerActive,
    pageTabsForActiveThread,
    effectiveCenterActive,
    shownFilePath,
    setCenterActive,
    openTab,
    pendingSessionRef,
    setPendingSessionTick,
    handleReorderCenterTabs,
    openingThread,
    closePageTab,
    liveItems,
    setLiveItems,
    handleStepOrigin,
    handleGoBack,
    handleGoForward,
  };
}

/** The tab a thread falls back to: its first agent session's, else the
 *  session picker, else its first tab. */
function homeTabOf(tabs: TabRef[]): string {
  return (
    tabs.find((t) => typeOf(t.id) === "oxplow/agent_session")?.id ??
    tabs.find((t) => isPage(t, "new-session"))?.id ??
    tabs[0]?.id ??
    HOME_TAB
  );
}
