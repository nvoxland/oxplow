import { useResourcesLoader } from "./resources.js";
import { isWorkspaceDir } from "./workspaceFiles.js";
import { useOpenHandoffs } from "./handoffs.js";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import {
  closeThread,
  markThread,
  createThread,
  openAgentSession,
  renameAgentSession,
  renameNativeConversation,
  startAgentSession,
  stopAgentSession,
  getThreadState,
  type AgentStatus,
  type AgentStatusEntry,
  getCurrentStream,
  getWorkspaceContext,
  type GitOpKickoff,
  desktopBridge,
  listStreams,
  probeDaemon,
  renameThread,
  renameStream,
  selectThread,
  recordUsage,
  switchStream,
  runCommandInBackground,
  commandOpenAfter,
  type ThreadMark,
  type ThreadState,
  type SqlCell,
  type Stream,
  type WorkspaceContext,
} from "./api.js";
import { buildMenuBar, buildNativeMenuSnapshots, menuItemById, OPEN_RECENT_PREFIX } from "./menuBar.js";
import type { EditorNavigationTarget } from "./lsp.js";
import { Navigator } from "./components/Navigator.js";
import { StatusBar } from "./components/StatusBar.js";
import { TitleBar } from "./components/TitleBar.js";
import { showToast } from "./components/toastStore.js";
import { awaitGitOp, opErrorOf } from "./git-op.js";
import { UndoToastStack } from "./components/UndoToast.js";
import { identityOf, keyOf, paramOf, parseUri, typeOf } from "./uri/uri.js";
import { labelReadsRow, rendererFor, shellView, tabLabel } from "./pages/renderers.js";
import { PersonCommandConfirm } from "./components/PersonCommandConfirm.js";
import { RemoteConnectionBanner } from "./components/RemoteConnectionBanner.js";
import { subscribeUiError } from "./ui-error.js";
import { useBackendSubscriptions } from "./useBackendSubscriptions.js";
import { Menubar } from "./components/Menubar.js";
import { CenterTabs, type CenterTab } from "./components/CenterTabs/CenterTabs.js";
import type { DiffSpec } from "./components/Diff/DiffPane.js";
import { builtInPages } from "./pages/builtInPages.js";
import type { AppScope, SlotNav } from "./pages/appScope.js";
import { RailHud } from "./components/RailHud/RailHud.js";
import type { TabRef } from "./tabs/tabState.js";
import { navigationKey } from "./tabs/navigationKeys.js";
import { PageNavigationContext, type PageNavigation, type TabEditing } from "./tabs/PageNavigationContext.js";
import { removeBookmark, setBookmark, useBookmarks } from "./tabs/bookmarks.js";
import type { BookmarkScope } from "./tabs/bookmarks.js";
import { AgentSessionPage } from "./pages/AgentSessionPage.js";
import type { MenuItem } from "./menu.js";
import { InlinePromptStrip } from "./components/InlinePromptStrip.js";
import { reconcileSessionTabs } from "./tabs/sessionTabs.js";
import { newThreadOpening } from "./newThreadSession.js";
import { sessionLabel } from "./agentKinds.js";
import { renameHint, renameSession } from "./sessionRename.js";
import { useAgentHarnesses } from "./useAgentHarnesses.js";
import { readThreadSessions, useThreadSessions } from "./agentSessions.js";
import { rollUpAgentStatus, threadStatuses } from "./agentStatusRollup.js";
import { LensPage } from "./pages/LensPage.js";
import { insertIntoAgent } from "./agent-input-bus.js";
import { getPageDetailStore } from "./tabs/openPageDetail.js";
import { DeclaredPageView } from "./pages/DeclaredPageView.js";
import { kindForTabId } from "./pageKinds.js";
import { declaredPage, isAppRenderer, isBuiltInPage, type AppRendererName } from "./pages/declaredPages.js";
import { useExtensions } from "./extensionsStore.js";
import { PanelRunsProvider } from "./components/Panels/PanelRunsContext.js";
import { useAlerts } from "./components/Alerts/useAlerts.js";
import { useAlertToasts } from "./components/Alerts/useAlertToasts.js";
import { DomCommentLayer } from "./components/Comments/DomCommentLayer.js";
import { agentSessionRef, diskFilePath, fileOf, lensRef, refFromTabId, directoryRef, externalUrlRef, fileRef, gitCommitRef, pagePayload, recordedPageKind, isPage, diffRef, newSessionRef, newStreamRef, alertsRef, wikiPageRef, streamSettingsRef, threadSettingsRef, viewParamsOf, withViewParams, fileRefAt, resourceParamsOf, type PagePayload } from "./tabs/pageRefs.js";
import { requestNewThread } from "./new-thread-bus.js";
import { recordOpError, setOpErrorContext } from "./opErrors.js";
import { classifyExternalUrl } from "./external-url-allowlist.js";
import { installContextMenuSuppressor } from "./context-menu.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { settingsAreaEntries } from "./components/RailHud/sections.js";
import { NON_TRACKED_KINDS } from "./components/RailHud/history.js";
import { buildTabReport } from "./tabs/tabReport.js";
import type { PickerMode } from "./components/conversationRows.js";
import { useTabSessions } from "./tabs/useTabSessions.js";
import {
  activate,
  activeRef,
  back as sessionBack,
  bodies,
  close as sessionClose,
  current as currentEntry,
  evictions,
  forward as sessionForward,
  MAX_PAGE_TABS,
  navigate as sessionNavigate,
  open as sessionOpen,
  refsOf,
  reorder as sessionReorder,
  patchHistoryState as sessionPatchHistoryState,
  setStrip,
  slotShowing,
  stepOrigin,
  identityOf as pageIdentityOf,
  type Entry,
  type Origin,
  type Session,
  type Tab,
} from "./tabs/session.js";
import { openProject, createProject, listRecentProjects, shellAvailable } from "./api.js";
import { onRemoteReconnect, triggerRemoteResync } from "./api.js";
import type { CommandOutcome, ExtensionPage, RecentProjectView } from "./tauri-bridge/generated/bindings.js";
import { pickFolder } from "./tauri-bridge/nativeDialog.js";
import { WORKING } from "./revision.js";
import { isDirty } from "./editor/buffers.js";
import { buffers } from "./editor/openFiles.js";
import { watchShownAnswers } from "./shownAnswers.js";
import { recordPageVisit, reportTabs, resolveSymbol } from "./api.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { offerForShortcut } from "./keybindings.js";
import { commandOffers, type OfferDeps } from "./commandOffers.js";
import { setRefOfferHost } from "./components/refCommands.js";
import type { WhenContext } from "./when.js";
import { runLocally, startClientHost, type ClientCallContext, type ClientHandlers } from "./clientHost.js";
import { streamOfThread } from "./tabs/threadTabOps.js";
import { planNavigation } from "./tabs/navigation.js";
import { openRef as openRefAt, setWindowRefOpener } from "./tabs/openRef.js";
import { usePersonCommands } from "./personCommandsStore.js";
import { personCommands } from "./personCommands.js";
import { getUiClientId, logUi, setUiLogContext } from "./logger.js";

/// Prompt for a folder and open it as an **existing** project.
/// `newWindow=false` replaces the current window (this process exits
/// once the new one spawns); `true` opens an additional independent
/// window. A folder that isn't a project yet is refused by
/// `open_project` — File ▸ New Project… is the door for that.
async function pickAndOpenProject(newWindow: boolean) {
  const selected = await pickFolder(newWindow ? "Open Project in New Window" : "Open Project");
  if (selected === null) return;
  try {
    await openProject(selected, newWindow);
  } catch (e) {
    recordOpError({
      label: "Open project",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}

/// Prompt for a folder and create a new project in it. Always opens in
/// a new window, so creating never closes the window the command ran
/// from; a folder that already is a project is refused by
/// `create_project`.
async function pickAndCreateProject() {
  const selected = await pickFolder("New Project");
  if (selected === null) return;
  try {
    await createProject(selected);
  } catch (e) {
    recordOpError({
      label: "New project",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}

/** No tab chosen: the thread's home tab shows — its first agent session's,
 *  else the session picker if it's open, else its first tab (`homeTabOf`). */
const HOME_TAB = "";

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

/** `slot`'s page in the view `view` makes of its current one (its id's
 *  `#`): the same page, so a filter replaces the entry and a pick
 *  (`push`) adds one Back returns from. */
function changeView(
  session: Session,
  slot: string,
  view: (now: Record<string, SqlCell>) => Record<string, SqlCell>,
  opts?: { push?: boolean },
): Session {
  const tab = session.tabs.find((t) => t.slot === slot);
  if (!tab) return session;
  const ref = currentEntry(tab).ref;
  return sessionNavigate(session, slot, { ref: withViewParams(ref, view(viewParamsOf(ref))) }, { replace: !opts?.push });
}

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [threadStates, setThreadStates] = useState<Record<string, ThreadState>>({});
  const harnesses = useAgentHarnesses();
  // Mirror of threadStates for subscription callbacks that need the
  // latest map without re-subscribing when it changes (see
  // useBackendSubscriptions). Kept current on every render.
  const threadStatesRef = useRef(threadStates);
  threadStatesRef.current = threadStates;

  // Each agent session's status (`sessionStatusKey`). A thread's dot is its
  // sessions' roll-up, and its question (the rail dot's tooltip) the one
  // the awaiting session asks.
  const [sessionStatuses, setSessionStatuses] = useState<Record<string, AgentStatusEntry>>({});
  // The agent session whose tab's Rename… strip is open.
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  // The conversation picker open over a session's terminal, and for what.
  const [conversationPicker, setConversationPicker] = useState<{ sessionId: string; mode: PickerMode } | null>(null);
  const openHandoffs = useOpenHandoffs();
  const { statuses: agentStatuses, questions: agentQuestions } = useMemo(
    () => threadStatuses(Object.values(sessionStatuses)),
    [sessionStatuses],
  );
  const [stream, setStream] = useState<Stream | null>(null);
  // Every thread's center tabs: a session of entries per thread
  // (`tabs/session.ts`), restored on start and stored on every change.
  // The strip, the bodies, history, origins and the active tab all come
  // from it (`.context/pages-and-tabs.md` "Navigation model").
  const { sessions, update: updateSession } = useTabSessions();
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
  const [error, setError] = useState<string | null>(null);
  // The error banner auto-dismisses; it's transient feedback (e.g. a
  // mistargeted file link), not a persistent state the user must clear.
  useEffect(() => {
    if (!error) return;
    const t = setTimeout(() => setError(null), 8000);
    return () => clearTimeout(t);
  }, [error]);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ vcsEnabled: false });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [quickOpenQuery, setQuickOpenQuery] = useState("");
  const daemonDownLogged = useRef(false);
  const daemonProbeState = useRef(INITIAL_DAEMON_PROBE_STATE);
  // macOS uses the native top-of-screen menu bar (driven by the
  // renderer through `setNativeMenu`, built in
  // src-tauri/.../commands/menu.rs); the in-window Menubar would
  // duplicate it, so it only renders off-Mac.
  const isMac = typeof navigator !== "undefined"
    && /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent || "");

  useEffect(() => {
    return subscribeUiError(({ label, message }) => {
      setError(`${label}: ${message}`);
    });
  }, []);

  // Top-level state hydration: streams, the current stream + its
  // threads, workspace context, and the selected thread's work state.
  // Run on mount and again on a remote-daemon WS reconnect (see
  // `onRemoteReconnect` below) so state goes live again after a drop
  // without a manual full-page reload.
  const loadInitialAppState = useCallback(() => {
    return Promise.all([listStreams(), getCurrentStream(), getWorkspaceContext()])
      .then(async ([allStreams, current, context]) => {
        const initialThreadState = await getThreadState(current.id);
        setStreams(allStreams);
        setStream(current);
        setThreadStates((prev) => ({ ...prev, [current.id]: initialThreadState }));
        setWorkspaceContext(context);
        // Prefetch thread state for the remaining streams so the
        // navigator can render every thread under every stream
        // immediately, not only after the user switches to that stream.
        const otherStreams = allStreams.filter((s) => s.id !== current.id);
        for (const s of otherStreams) {
          void getThreadState(s.id)
            .then((state) => setThreadStates((prev) => ({ ...prev, [s.id]: state })))
            .catch((e) => logUi("warn", "failed to prefetch thread state", { streamId: s.id, error: String(e) }));
        }
        setError(null);
        setDaemonUnavailable(false);
        logUi("info", "loaded initial app state", {
          streamCount: allStreams.length,
          currentStreamId: current.id,
          vcsEnabled: context.vcsEnabled,
        });
      })
      .catch((e) => {
        setError(String(e));
        setDaemonUnavailable(true);
        logUi("error", "failed to load initial app state", { error: String(e) });
      });
  }, []);

  useEffect(() => {
    void loadInitialAppState();
  }, [loadInitialAppState]);

  // Remote-daemon WS reconnect: re-hydrate the top-level stores so the
  // UI catches up on events missed while the socket was down. The WS
  // itself auto-re-subscribes; this covers the snapshot the client
  // holds. No manual reload — that's the whole point.
  useEffect(() => {
    return onRemoteReconnect(() => {
      logUi("info", "remote daemon reconnected, resyncing state");
      void loadInitialAppState();
    });
  }, [loadInitialAppState]);

  useEffect(() => {
    let cancelled = false;

    async function check() {
      const alive = await probeDaemon();
      if (cancelled) return;
      const decision = advanceDaemonProbeState(daemonProbeState.current, alive);
      daemonProbeState.current = decision.next;
      if (decision.refresh) {
        // Daemon came back after an HTTP-level outage. Resync the stores
        // in place instead of a full page reload (which drops unsaved
        // editor drafts). triggerRemoteResync re-runs the same reconnect
        // handlers a WS recovery would.
        logUi("info", "daemon recovered, resyncing ui");
        setDaemonUnavailable(false);
        triggerRemoteResync();
        return;
      }
      setDaemonUnavailable(decision.next.unavailable);
      if (decision.next.unavailable && !daemonDownLogged.current) {
        logUi("warn", "daemon probe failed");
        daemonDownLogged.current = true;
      }
      if (alive) {
        daemonDownLogged.current = false;
      }
    }

    check();
    const timer = window.setInterval(check, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  async function handleSwitch(id: string) {
    try {
      logUi("info", "switching stream", { streamId: id });
      const next = await switchStream(id);
      let nextThreadState = threadStates[next.id] ?? await getThreadState(next.id);
      // Invariant: a stream must always have a selected thread. If the
      // switched-to stream has no remembered selection, auto-pick its
      // first thread by sort order and persist that choice so subsequent
      // switches remember it.
      if (!nextThreadState.selectedThreadId && nextThreadState.threads.length > 0) {
        nextThreadState = await selectThread(next.id, nextThreadState.threads[0].id);
      }
      setThreadStates((prev) => ({ ...prev, [next.id]: nextThreadState }));
      setStream(next);
      setError(null);
      setDaemonUnavailable(false);
      logUi("info", "switched stream", { streamId: next.id, title: next.title });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to switch stream", { streamId: id, error: String(e) });
    }
  }

  async function handleRenameStreamById(streamId: string, newTitle: string) {
    const updated = await renameStream(streamId, newTitle);
    if (stream?.id === updated.id) setStream(updated);
    setStreams((prev) =>
      prev
        .map((candidate) => (candidate.id === updated.id ? updated : candidate))
        .sort((a, b) => a.created_at.localeCompare(b.created_at)),
    );
    setError(null);
  }

  async function handleRenameThreadById(threadId: string, newTitle: string) {
    if (!stream) return;
    try {
      await renameThread(stream.id, threadId, newTitle);
      const refreshed = await getThreadState(stream.id);
      setThreadStates((prev) => ({ ...prev, [stream.id]: refreshed }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleStreamCreated(next: Stream) {
    setStreams((prev) => {
      const others = prev.filter((stream) => stream.id !== next.id);
      return [...others, next].sort((a, b) => a.created_at.localeCompare(b.created_at));
    });
    setError(null);
    setDaemonUnavailable(false);
    logUi("info", "stream created in ui", { streamId: next.id, title: next.title, branch: next.branch });
    // Make the new stream current on the backend BEFORE we let the
    // TerminalPane mount and call open_terminal_session — that command
    // builds its session_key from `state.streams.current()`, so without
    // this hop the new thread's terminal would dedup onto the previous
    // stream/thread's PTY and show the wrong agent's transcript.
    try {
      await switchStream(next.id);
    } catch (e) {
      logUi("warn", "switch_stream after create failed", { streamId: next.id, error: String(e) });
    }
    try {
      const state = await getThreadState(next.id);
      setThreadStates((prev) => ({ ...prev, [next.id]: state }));
    } catch (e) {
      setError(String(e));
    }
    setStream(next);
  }

  /** Open the working-tree file `path`, its tab showing `ref` (the file in
   *  a view: `#lines=`; the plain file when none is given). Its buffer is
   *  read first, so a clicked path that isn't a file (a dotted word in
   *  prose, a stale link) opens no tab and says so; a directory opens its
   *  folder's page. With `ref` null only the buffer is read. */
  async function handleOpenFile(path: string, ref: TabRef | null = fileRef(path)) {
    if (!stream) return;
    // A trailing-slash link (terminal or agent output) is a folder: no
    // read is tried for it.
    if (path.endsWith("/") && (await isWorkspaceDir(stream.id, path))) {
      setError(null);
      handleOpenPageRef.current?.(directoryRef(path));
      return;
    }
    const buffer = await buffers.open(stream, path);
    if (buffer.state !== "ready") {
      if (await isWorkspaceDir(stream.id, path)) {
        setError(null);
        handleOpenPageRef.current?.(directoryRef(path));
        return;
      }
      if (buffer.state === "missing") {
        setError(`File not found: ${path}`);
        logUi("warn", "open file: target does not exist", { streamId: stream.id, path });
      } else {
        setError(buffer.error ?? `Couldn't open ${path}`);
        logUi("error", "failed to open file", { streamId: stream.id, path, error: buffer.error });
      }
      return;
    }
    if (!ref) return;
    openTab(ref);
    setError(null);
    void recordUsage({
      kind: "editor-file",
      key: path,
      event: "open",
      streamId: stream.id,
      threadId: selectedThread?.id ?? null,
    }).catch(() => {});
  }

  /** A `symbol:` ref opens its file at the symbol's line; one pinned to a
   *  version opens the file as it was then. A pin whose version is gone
   *  opens the current symbol and says so. */
  async function openSymbol(ref: string) {
    if (!stream) return;
    try {
      const at = await resolveSymbol(stream.id, ref);
      if (!at) {
        recordOpError({ label: "Open symbol", message: `${ref} isn't a symbol in this stream now.` });
        return;
      }
      if (at.versionGone) {
        recordOpError({ label: "Open symbol", message: `The version ${ref} pins is gone; showing the symbol as it is now.` });
      }
      if (at.revision) {
        handleOpenPage(fileRef(at.path, at.revision));
        return;
      }
      await handleNavigateToLocation({ path: at.path, line: at.line, column: at.col });
    } catch (e) {
      recordOpError({ label: "Open symbol", message: e instanceof Error ? e.message : String(e) });
    }
  }

  /** Open `target`'s file at its line: its tab in that view (`ref`, else
   *  `#lines=<line>`, where the editor lands). */
  async function handleNavigateToLocation(target: EditorNavigationTarget, ref?: TabRef) {
    await handleOpenFile(target.path, ref ?? fileRefAt(target.path, target.line));
  }

  /** Close the tab showing the working file `path`. Unsaved edits close
   *  with it, and a toast offers Undo for a while: the tab and its draft
   *  come back. */
  function handleCloseOpenFile(id: string, path: string) {
    if (!stream) return;
    const stashed = buffers.stash(stream.id, path);
    closePageTab(id);
    if (!stashed) return;
    const basename = path.split("/").pop() ?? path;
    showToast({
      message: `Closed "${basename}" with unsaved changes.`,
      actionLabel: "Undo",
      onUndo: () => {
        void buffers.restore(stashed).then(() => openTab(fileRef(path)));
      },
    });
  }

  async function handleSelectThread(streamId: string, threadId: string) {
    try {
      // Cross-stream selection: switch the active stream first so the
      // rest of the app (center tabs, file session, work panel) reframes
      // around the new stream before we apply the thread selection.
      // Doing this sequentially also avoids a race where handleSwitch's
      // own setThreadStates write (seeded from the prefetched state with
      // the OLD selectedThreadId) clobbers the selection we're about to
      // apply.
      if (stream && streamId !== stream.id) {
        await handleSwitch(streamId);
      }
      const next = await selectThread(streamId, threadId);
      setThreadStates((prev) => ({ ...prev, [streamId]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateThread(title: string) {
    if (!stream) return;
    try {
      // It opens on the session picker: a thread names no agent.
      const { state: next } = await createThread(stream.id, title);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMarkThread(threadId: string, mark: ThreadMark | null) {
    const owner = streams.find((s) => threadStates[s.id]?.threads.some((t) => t.id === threadId));
    if (!owner) return;
    try {
      const next = await markThread(owner.id, threadId, mark);
      setThreadStates((prev) => ({ ...prev, [owner.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCloseThread(threadId: string) {
    if (!stream) return;
    try {
      const next = await closeThread(stream.id, threadId);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }


  const currentThreadState = useMemo(
    () => (stream ? threadStates[stream.id] ?? { selectedThreadId: null, threads: [] } : { selectedThreadId: null, threads: [] }),
    [threadStates, stream],
  );
  const selectedThread = currentThreadState.threads.find((thread) => thread.id === currentThreadState.selectedThreadId) ?? null;
  // The selected thread's open agent sessions: its session tabs follow them.
  const selectedThreadSessions = useThreadSessions(selectedThread?.id ?? null);
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
  const selectedThreadId = selectedThread?.id ?? null;
  // Each thread's tabs as the pages they show now, in strip order.
  const threadPageTabs = useMemo(
    () => Object.fromEntries(Object.entries(sessions).map(([thread, session]) => [thread, refsOf(session)])),
    [sessions],
  );
  // The shown thread's active tab's page id; `HOME_TAB` when none is
  // chosen (the thread's home tab shows — `effectiveCenterActive`).
  const centerActive = (selectedThreadId ? activeRef(sessions[selectedThreadId])?.id : null) ?? HOME_TAB;
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

  // A failed operation from no named thread is the one on screen's.
  useEffect(() => {
    setOpErrorContext({ thread: selectedThreadId, stream: stream?.id ?? null });
  }, [selectedThreadId, stream?.id]);

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const threads = threadStates[s.id]?.threads ?? [];
      // The threads' roll-up: what you owe (an answer, a dead turn) shows
      // even from a collapsed rail, ahead of work in flight.
      out[s.id] = rollUpAgentStatus(threads.flatMap((t) => agentStatuses[t.id] ?? [])) ?? "waiting";
    }
    return out;
  }, [streams, threadStates, agentStatuses]);

  // Backend-event subscription wiring (workspace context, backlog, task
  // events, followup/thread/stream changes, config, agent status) lives
  // in this hook so App doesn't carry ~10 inline subscription effects.
  // Extensions' ref kinds (P8.D7): icons, labels, routes and wikilinks.
  useResourcesLoader();
  useBackendSubscriptions({
    threadStatesRef,
    setWorkspaceContext,
    setThreadStates,
    setStreams,
    setStream,
    setSessionStatuses,
  });

  // Imperative shortcut for opening the New-Task modal. When PlanPane is
  // mounted it registers its openCreateModal here; the menu handler can
  // call this ref directly instead of going through setState + useEffect.
  // Needed because menu clicks arrive as IPC messages (not "discrete user
  // input events"), so React doesn't auto-flush effects for them — the
  // useEffect chain can stall for 10+ seconds before committing. Direct
  // ref call inside flushSync sidesteps the scheduler entirely.
  // Forward ref so the forms and offers (declared above handleOpenPage) can
  // route through the same page-tab opener used by every other caller.
  // The ref is populated in a useEffect after handleOpenPage is defined.
  const handleOpenPageRef = useRef<((ref: TabRef) => void) | null>(null);
  // The window's context keys: what a command's `when` reads
  // (`oxplow_domain::when::CONTEXT_KEYS`; `.context/commands.md`).
  const whenContext = useMemo<WhenContext>(
    () => ({
      streamShown: !!stream,
      streamKind: stream?.kind ?? "",
      threadShown: !!selectedThread,
      fileShown: shownFilePath !== null,
      pageDirty: !!centerActive && !!dirtyTabs[centerActive],
      pageKind: centerActive ? kindForTabId(centerActive) : "",
      vcsEnabled: !!workspaceContext.vcsEnabled,
      shellAvailable: shellAvailable(),
      enabledAgents: harnesses
        .filter((h) => h.enabled)
        .map((h) => h.id)
        .join(","),
    }),
    [centerActive, dirtyTabs, harnesses, selectedThread, shownFilePath, stream, workspaceContext.vcsEnabled],
  );
  // Run a Git-menu mutation (pull/push) as a background task and surface
  // any failure the same way the Git Dashboard does: record an op-error
  // and offer a toast that opens its detail page. There's no page focus
  // here, so we route through handleOpenPageRef rather than onOpenPage.
  const runGitMenuOp = useCallback(
    async (
      label: string,
      command: string,
      action: () => Promise<GitOpKickoff>,
    ) => {
      const result = await awaitGitOp(await action());
      if (result.success) return;
      // Surfaces globally (toast + status-bar indicator) via recordOpError.
      recordOpError(opErrorOf(label, command, result));
    },
    [],
  );
  /** One of the window's own forms — a command's `ui.form` that isn't a
   *  tab id: what gathers New Thread's and Commit's input. */
  const openForm = useCallback((name: string) => {
    switch (name) {
      case "new-thread":
        // The Navigator owns thread creation (inline title + agent picker).
        if (stream) requestNewThread(stream.id);
        return;
      default:
        recordOpError({ label: "Open form", message: `the window has no form \`${name}\`` });
    }
  }, [stream, workspaceContext.vcsEnabled]);
  const [recentProjects, setRecentProjects] = useState<RecentProjectView[]>([]);
  // The native menu and its recent projects are the shell's: a browser
  // window has neither to ask for.
  useEffect(() => {
    if (!shellAvailable()) return;
    listRecentProjects()
      .then(setRecentProjects)
      .catch((e) => logUi("warn", "failed to load recent projects for menu", { error: String(e) }));
  }, []);
  // Cancel the native WKWebView context menu everywhere except inputs,
  // contenteditable, Monaco, and the terminal (see context-menu.ts).
  useEffect(() => installContextMenuSuppressor(), []);
  // What the command bus offers a person (Pull, New Task, …): search lists
  // them with the app's own commands, and a shortcut may run one.
  const personSpecs = usePersonCommands();
  const offerCtx = useMemo(
    () => ({ streamId: stream?.id ?? null, threadId: selectedThreadId ?? null, when: whenContext }),
    [stream?.id, selectedThreadId, whenContext],
  );
  const offerDeps = useMemo<OfferDeps>(
    () => ({
      openPage: (tabId) => {
        const ref = refFromTabId(tabId);
        if (ref) handleOpenPageRef.current?.(ref);
      },
      openForm,
      run: (label, id, input) => {
        // One the window hosts runs here: nothing goes to the daemon.
        const spec = personSpecs.find((s) => s.id === id);
        const local = spec ? runLocally(windowHandlersRef.current, spec, input) : null;
        if (local) {
          return local
            .then(({ result }): CommandOutcome => ({ result, audit_id: null, event_id: null, inverse: null }))
            .catch((e: unknown) => {
              recordOpError({ label, message: e instanceof Error ? e.message : String(e) });
              return null;
            });
        }
        return personCommands.run(label, id, input);
      },
      runInBackground: (label, id, input) =>
        void runGitMenuOp(label, id, () => runCommandInBackground(label, id, input)),
      openAfter: (id, result) =>
        commandOpenAfter(id, result).catch((e: unknown) => {
          recordOpError({ label: "Open what it made", message: e instanceof Error ? e.message : String(e) });
          return null;
        }),
      }),
    [personSpecs, runGitMenuOp, openForm],
  );
  const offers = useMemo(() => commandOffers(personSpecs, offerCtx, offerDeps), [personSpecs, offerCtx, offerDeps]);
  // A ref's menus (its page's, its rows') run offers the same way.
  useEffect(() => {
    setRefOfferHost({ ctx: offerCtx, deps: offerDeps });
    return () => setRefOfferHost(null);
  }, [offerCtx, offerDeps]);
  // The menu bar (File, Edit) and what search lists: the bus's offers,
  // and the shell's project commands.
  const menuGroups = useMemo(() => buildMenuBar(offers), [offers]);
  const nativeMenuSnapshots = useMemo(
    () => buildNativeMenuSnapshots(menuGroups, recentProjects),
    [menuGroups, recentProjects],
  );

  // The launcher (QuickOpen) is the single discovery surface — pages,
  // files, commands, and body search in one box — and has exactly one
  // shortcut: Cmd/Ctrl+P (`oxplow.window.quick_open`'s). The old
  // Cmd+K / Cmd+Shift+F aliases were removed (tsk59): one door is
  // clearer, and Cmd+P is the established dev quick-open reflex. Monaco
  // doesn't bind Cmd+P, so no capture-phase interception is needed — the
  // menu accelerator opens the launcher even when the editor is focused.

  useEffect(() => {
    // Runs in both Electron and browser modes. In Electron the native
    // menu's accelerator should also fire for the same command, but the
    // handler is idempotent (an offer's run → modal setters are no-ops
    // when the modal is already open) so a double-dispatch is harmless
    // — and not relying on the native menu means Cmd+Shift+N works even
    // when the menu snapshot is momentarily stale at startup.
    function handleKeyDown(event: KeyboardEvent) {
      // A command's `ui.shortcut`; typing in a field keeps it unless it
      // runs while typing (Save, Find, Quick Open do — a user mid-way
      // through a description shouldn't lose it to New Task's form).
      const offer = offerForShortcut(offers, event, isEditableTarget(event.target));
      if (offer) {
        event.preventDefault();
        offer.run();
        return;
      }
      // Back, Forward, and Up / Down the page's origin, on the active tab.
      const key = navigationKey(event, event.target);
      if (!key) return;
      const keys = navKeysRef.current;
      event.preventDefault();
      if (key === "back") keys.back();
      else if (key === "forward") keys.forward();
      else keys.step(key === "up" ? -1 : 1);
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [offers]);

  useEffect(() => {
    if (!shellAvailable()) return;
    void desktopBridge().setNativeMenu(nativeMenuSnapshots).catch((error) => {
      logUi("error", "failed to update native menu", { error: String(error) });
    });
  }, [nativeMenuSnapshots]);

  useEffect(() => {
    return desktopBridge().onMenuCommand((commandId: string) => {
      // Dynamic "Open Recent ▸ <project>" entries aren't menu items:
      // `oxplow.project.open`'s operation opens the trailing path in a
      // new window. A recent whose `.oxplow/` has since been deleted
      // errors rather than re-creating it.
      if (commandId.startsWith(OPEN_RECENT_PREFIX)) {
        const path = commandId.slice(OPEN_RECENT_PREFIX.length);
        void Promise.resolve(
          windowHandlersRef.current["projects.write"]?.open?.({ path, new_window: true }, { threadId: null, actor: "human", call: null }),
        ).catch((e: unknown) => {
          recordOpError({
            label: "Open project",
            message: e instanceof Error ? e.message : String(e),
          });
        });
        return;
      }
      const command = menuItemById(menuGroups, commandId);
      if (!command || !command.run) return;
      // React 18 only auto-flushes effects synchronously for discrete
      // user input events (click, keydown on webContents). IPC messages
      // from the main process don't qualify, so setState calls made in
      // this callback stay queued until the next real input event wakes
      // the scheduler — users reported menu dispatches stalling 10+
      // seconds. flushSync commits inside the callback. The commands
      // that open modals (plan.newTask, etc.) additionally go
      // through an imperative ref registered by the target pane so the
      // modal setState also commits here rather than via useEffect.
      const run = command.run;
      flushSync(() => { void run(); });
    });
  }, [menuGroups]);

  const pageTabsForActiveThread = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
  const availableCenterIds = useMemo(() => new Set(pageTabsForActiveThread.map((ref) => ref.id)), [pageTabsForActiveThread]);
  const effectiveCenterActive = availableCenterIds.has(centerActive)
    ? centerActive
    : homeTabOf(pageTabsForActiveThread);

  // Feed the stall watchdog (logger.ts) the coarse "what's on screen"
  // context so a `main thread stalled` WARN names the active page + the
  // open file, not just a duration.
  useEffect(() => {
    setUiLogContext({
      streamId: stream?.id ?? null,
      centerActive: effectiveCenterActive,
      filePath: shownFilePath,
    });
  }, [stream?.id, effectiveCenterActive, shownFilePath]);

  // Central page-visit recorder. Fires whenever the resolved active
  // tab changes — covers new-tab opens (any handler), switching to an
  // already-open tab, and snap-back when the active tab closes. Skips
  // the first commit so persisted-tab restoration on startup doesn't
  // flood History.
  //
  // The effect's dependency list is intentionally minimal: only the
  // active id + selected thread should re-fire it. The supporting
  // context (page tabs, file open-order, tasks titles, stream id)
  // is read through a ref so e.g. opening a SECOND tab doesn't
  // re-record a visit for the FIRST (still-active) one.
  // tabLabelByIdRef is the single source of "what's this tab called
  // right now" — populated by the centerTabs builder after it has
  // applied any pageTitles override. The history recorder reads from
  // this so RailHud History shows the *same* string as the tab strip
  // for every kind.
  const tabLabelByIdRef = useRef<Record<string, string>>({});
  const visitContextRef = useRef<{
    pageTabs: TabRef[];
    streamId: string | null;
  }>({ pageTabs: [], streamId: null });
  visitContextRef.current = {
    pageTabs: pageTabsForActiveThread,
    streamId: stream?.id ?? null,
  };
  const hydratedHistoryRef = useRef(false);
  // What was last recorded: a page whose view changed (a filter, a row
  // picked) is the same visit, so Recents lists the page once.
  const lastVisitRef = useRef<string | null>(null);
  useEffect(() => {
    const shownRef = visitContextRef.current.pageTabs.find((t) => t.id === effectiveCenterActive);
    const visit = `${selectedThreadId}|${pageIdentityOf(shownRef?.id ?? effectiveCenterActive)}`;
    const same = lastVisitRef.current === visit;
    lastVisitRef.current = visit;
    if (!hydratedHistoryRef.current) {
      hydratedHistoryRef.current = true;
      return;
    }
    if (!selectedThreadId || same) return;
    const ctx = visitContextRef.current;
    const ref = ctx.pageTabs.find((t) => t.id === effectiveCenterActive) ?? null;
    if (!ref || NON_TRACKED_KINDS.has(recordedPageKind(ref))) return;
    // Defer briefly so the freshly-activated page's `usePageTitle`
    // effect fires and the centerTabs memo applies it before we
    // snapshot the label. tabLabelByIdRef is what the tab strip
    // currently displays — reading from it keeps the two surfaces
    // consistent; there is no separate fallback derivation.
    const timer = setTimeout(() => {
      const ctxAfter = visitContextRef.current;
      const label = tabLabelByIdRef.current[effectiveCenterActive] ?? ref.id;
      void recordPageVisit({
        refKind: recordedPageKind(ref),
        // A visit is of the page, whatever its view.
        refId: pageIdentityOf(ref.id),
        label,
        streamId: ctxAfter.streamId,
        threadId: selectedThreadId,
      });
    }, 250);
    return () => clearTimeout(timer);
  }, [effectiveCenterActive, selectedThreadId]);

  // Report what this window shows — every thread's tabs and the one
  // selected in each, the thread it shows, its focus and visibility, and
  // the shown page's published detail (a lens's params) — so an agent's
  // `list_tabs` / `get_open_page` see what the person sees. Best-effort
  // and debounced; see tabs/tabReport.ts and tabs/openPageDetail.ts.
  const [pageDetailTick, setPageDetailTick] = useState(0);
  useEffect(() => getPageDetailStore().subscribe(() => setPageDetailTick((t) => t + 1)), []);
  const [windowFocused, setWindowFocused] = useState(() => document.hasFocus());
  const [windowVisible, setWindowVisible] = useState(() => document.visibilityState === "visible");
  useEffect(() => {
    const focus = () => setWindowFocused(true);
    const blur = () => setWindowFocused(false);
    const visibility = () => setWindowVisible(document.visibilityState === "visible");
    window.addEventListener("focus", focus);
    window.addEventListener("blur", blur);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      window.removeEventListener("focus", focus);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
  useEffect(() => {
    const timer = setTimeout(() => {
      const ctx = visitContextRef.current;
      const shown = ctx.pageTabs.find((t) => t.id === effectiveCenterActive) ?? null;
      const report = buildTabReport({
        client: getUiClientId(),
        focused: windowFocused,
        visible: windowVisible,
        shownThread: selectedThreadId ?? null,
        sessions,
        titleOf: (tab) => tabLabelByIdRef.current[tab.id] ?? pageTitles[tab.id],
        detail: shown ? getPageDetailStore().get(shown.id) : null,
      });
      void reportTabs(report).catch(() => {});
    }, 300);
    return () => clearTimeout(timer);
  }, [effectiveCenterActive, selectedThreadId, sessions, pageTitles, pageDetailTick, windowFocused, windowVisible]);

  const closeDiffTab = (id: string) => closePageTab(id);


  const handleOpenWiki = useCallback((slug: string) => {
    const tid = selectedThread?.id ?? null;
    if (!tid) return;
    openTab(wikiPageRef(slug));
    const sid = stream?.id ?? null;
    if (sid) {
      void recordUsage({
        kind: "wiki",
        key: slug,
        event: "open",
        streamId: sid,
        threadId: tid,
      }).catch(() => {});
    }
  }, [stream?.id, selectedThread?.id, openTab]);

  /**
   * Open an http(s) URL as an in-app sandboxed external-url tab.
   * Validates through the scheme allowlist; rejected URLs are routed to
   * the OS browser via window.open (which the main process turns into a
   * shell.openExternal call) so the user still gets to follow the link
   * even if it can't be embedded.
   */
  const handleOpenExternalUrl = useCallback((rawUrl: string) => {
    const verdict = classifyExternalUrl(rawUrl);
    if (!verdict.ok) {
      window.open(rawUrl, "_blank", "noopener,noreferrer");
      return;
    }
    handleOpenPageRef.current?.(externalUrlRef(verdict.url));
  }, []);

  /** Open the commit page for a wikilink-resolved commit SHA. */
  const handleOpenCommit = useCallback((sha: string) => {
    if (!sha) return;
    handleOpenPageRef.current?.(gitCommitRef(sha));
  }, []);

  const handleReorderCenterTabs = useCallback((orderedIds: string[]) => {
    // The session's tabs are the strip's order for every kind; pinned tabs
    // never move (the strip won't drag them), and `reconcileSessionTabs`
    // keeps them first.
    if (selectedThread?.id) {
      updateSession(selectedThread.id, (session) =>
        sessionReorder(session, orderedIds.flatMap((id) => slotShowing(session, id) ?? [])),
      );
    }
  }, [stream, selectedThread?.id, updateSession]);

  const agentThreadStatus: AgentStatus = selectedThread ? agentStatuses[selectedThread.id] ?? "waiting" : "waiting";

  // A thread's agent-session tabs follow its open sessions (their rows):
  // each open one's tab leads, a closed one's goes. A thread seen here for
  // the first time (no tab list yet — they persist) with no session opens
  // the way the person's `newThreadSession` says: the session picker, no
  // session, or that agent's session.
  useEffect(() => {
    if (!selectedThreadId || selectedThreadSessions === null) return;
    if (sessions[selectedThreadId] === undefined && selectedThreadSessions.length === 0) {
      const threadId = selectedThreadId;
      updateSession(threadId, (session) => session);
      void openNewThread(threadId);
      return;
    }
    updateSession(selectedThreadId, (session) => setStrip(session, reconcileSessionTabs(refsOf(session), selectedThreadSessions)));
  }, [selectedThreadId, selectedThreadSessions, sessions, updateSession]);

  async function openNewThread(threadId: string) {
    try {
      const opening = await newThreadOpening();
      if (opening.tabs.length > 0) {
        updateSession(threadId, (session) =>
          refsOf(session).some((t) => isPage(t, "new-session")) ? session : setStrip(session, [...opening.tabs, ...refsOf(session)]),
        );
      }
      if (opening.start) {
        const opened = await openAgentSession(threadId, opening.start.harness);
        const ref = agentSessionRef(opened.id);
        updateSession(threadId, (session) => sessionOpen(session, { ref }, { pinned: true }));
      }
    } catch (error) {
      // Couldn't read the choice or start the agent: ask instead.
      logUi("warn", "opening a new thread failed; showing the session picker", { error: String(error) });
      updateSession(threadId, (session) =>
        refsOf(session).some((t) => isPage(t, "new-session")) ? session : setStrip(session, [newSessionRef(), ...refsOf(session)]),
      );
    }
  }

  const bookmarks = useBookmarks(selectedThreadId, stream?.id ?? null);

  const handleOpenPage = useCallback((ref: TabRef) => {
    // Every kind not handled below opens as a per-thread page tab.
    const openAsPageTab = () => openTab(ref);
    // Page-visit recording lives in the central activation effect
    // below — it fires whenever `effectiveCenterActive` resolves to a
    // new TabRef, regardless of which handler caused the activation.
    switch (typeOf(ref.id)) {
      case "oxplow/agent_session": {
        // Its tab is there while it's open on this thread
        // (`reconcileSessionTabs`). A closed session, or another thread's,
        // has none: its transcript opens instead, with Reopen on it.
        const sessionId = paramOf(ref.id, "id");
        // The transcript lens takes the session's bare id (`ses3`).
        const transcript = () =>
          handleOpenPageRef.current?.(
            lensRef("oxplow-foundation/session-transcript", { session: sessionId ?? keyOf(ref.id) ?? ref.id }),
          );
        if (sessionId && selectedThreadSessions?.some((candidate) => candidate.id === sessionId)) {
          setCenterActive(ref.id);
        } else if (sessionId && selectedThread) {
          // A session opened a moment ago (a form's `open_after`) isn't in
          // the window's list yet: read the model, and show its tab once the
          // list has it; a closed one, or another thread's, shows its transcript.
          void readThreadSessions(selectedThread.id).then(({ sessions: open }) => {
            if (open.some((candidate) => candidate.id === sessionId)) {
              pendingSessionRef.current = sessionId;
              setPendingSessionTick((tick) => tick + 1);
            }
            else transcript();
          });
        } else {
          transcript();
        }
        return;
      }
      case "oxplow/symbol":
        void openSymbol(identityOf(ref.id) ?? ref.id);
        return;
      case "oxplow/file": {
        const payload = fileOf(ref);
        if (!payload?.path) return;
        const version = payload.version ?? WORKING;
        // The working tree's file is read first (a path that isn't one
        // opens no tab); the line it lands on is its view (`#lines=`).
        if (version === WORKING) {
          void handleOpenFile(payload.path, ref);
          return;
        }
        // A file at a revision reads itself.
        openTab(ref);
        return;
      }
      case "page":
        // A page opened again in another view (a diff at another line) is
        // the tab already showing it, in that view.
        openAsPageTab();
        return;
      default:
        openAsPageTab();
        return;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handleOpenFile, handleOpenWiki, openTab, selectedThreadId, selectedThreadSessions, setCenterActive, stream?.id]);

  /**
   * Open the ref `ref` (of any kind) where its kind's page says
   * (`tabs/openRef.ts`, by way of `refOpener`): a tab through `open`, the
   * thread or stream it names, or an error. The one path every link to a
   * ref takes — a page's links (`PageNavigation.openRef`), the rail and
   * palette (the window's opener), a search hit. Kept in a ref so the
   * page contexts built in a memo always call the latest.
   */
  const openRefRef = useRef<(ref: string, how: import("./tabs/openRef.js").OpenHow, open: import("./tabs/openRef.js").RefHost["open"], closePage?: () => void) => void>(() => {});
  openRefRef.current = (ref, how, open, closePage) => {
    void openRefAt(ref, how, {
      open,
      closePage,
      showThread: async (thread) => {
        const streamId = streamOfThread(threadStatesRef.current, thread);
        if (!streamId) return false;
        await handleSelectThread(streamId, thread);
        return true;
      },
      showStream: async (id) => {
        if (!threadStatesRef.current[id]) {
          // A stream just made (New Stream) isn't in the window's state yet:
          // read it and take it in as the create flow always did.
          const made = (await listStreams()).find((candidate) => candidate.id === id);
          if (!made) return false;
          await handleStreamCreated(made);
          return true;
        }
        if (stream?.id !== id) await handleSwitch(id);
        return true;
      },
      failed: (message) => recordOpError({ label: "Open a reference", message }),
    });
  };
  useEffect(() => {
    setWindowRefOpener((ref, how) => openRefRef.current(ref, how, (tab) => handleOpenPageRef.current?.(tab)));
    return () => setWindowRefOpener(null);
  }, []);

  /** Navigate to a unified-search hit: wherever its ref opens. */
  const openSearchHit = useCallback((hit: import("./api.js").SearchHit) => {
    openRefRef.current(hit.uri, { newTab: true }, (tab) => handleOpenPage(tab));
  }, [handleOpenPage]);

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

  /** The collection a page is opened from, as an entry keeps it. */
  const originOf = (origin: Origin | undefined, ref: TabRef): Origin | undefined => {
    if (!origin) return undefined;
    const at = origin.items.findIndex((item) => item.ref.id === ref.id);
    const index = at !== -1 ? at : origin.index;
    if (index < 0 || index >= origin.items.length) return undefined;
    return { ...origin, index };
  };

  /**
   * Browser-style in-tab navigation: the tab showing `currentTabId` goes
   * to `ref`, pushed (Back returns) or in its place with `replace`. A tab
   * id no tab shows opens `ref` in a new tab.
   */
  const handleNavigateInTab = useCallback((
    currentTabId: string,
    ref: TabRef,
    origin?: Origin,
    replace = false,
  ) => {
    if (!selectedThreadId) return;
    const slot = slotShowing(sessions[selectedThreadId] ?? { tabs: [], active: null, mru: [], next: 1 }, currentTabId);
    // A symbol isn't a page (it opens its file at its line), and an agent
    // session has its own tab: replacing, the page they replace closes.
    const resource = typeOf(ref.id);
    if (resource === "oxplow/symbol" || resource === "oxplow/agent_session" || !slot) {
      if (replace && slot) closePageTab(currentTabId);
      handleOpenPage(ref);
      return;
    }
    updateSession(selectedThreadId, (session) =>
      sessionNavigate(session, slot, { ref, from: originOf(origin, ref) }, { replace }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [closePageTab, handleOpenPage, selectedThreadId, sessions, updateSession]);

  /** Open a diff in the tab showing `currentTabId`; Back returns to the
   *  page it came from. */
  const handleOpenDiffInTab = useCallback((currentTabId: string, spec: DiffSpec, origin?: Origin) => {
    handleNavigateInTab(currentTabId, diffRef(spec), origin);
  }, [handleNavigateInTab]);

  /** Show item `index` of the collection the tab was opened from, in
   *  place: history untouched. */
  const handleStepOrigin = useCallback((slot: string, index: number) => {
    if (!selectedThreadId) return;
    updateSession(selectedThreadId, (session) => stepOrigin(session, slot, index));
  }, [selectedThreadId, updateSession]);

  const handleGoBack = useCallback((slot: string) => {
    if (selectedThreadId) updateSession(selectedThreadId, (session) => sessionBack(session, slot));
  }, [selectedThreadId, updateSession]);

  const handleGoForward = useCallback((slot: string) => {
    if (selectedThreadId) updateSession(selectedThreadId, (session) => sessionForward(session, slot));
  }, [selectedThreadId, updateSession]);

  // What the navigation keys do now: the active tab's Back, Forward and
  // step through its origin. Read at the key press, so the handler below
  // needn't rebind as tabs move.
  const navKeysRef = useRef({ back: () => {}, forward: () => {}, step: (_by: number) => {} });
  navKeysRef.current = (() => {
    const session = selectedThreadId ? sessions[selectedThreadId] : undefined;
    const tab = session?.tabs.find((t) => t.slot === session.active);
    if (!tab) return { back: () => {}, forward: () => {}, step: () => {} };
    const origin = currentEntry(tab).from;
    return {
      back: () => handleGoBack(tab.slot),
      forward: () => handleGoForward(tab.slot),
      step: (by: number) => {
        if (origin) handleStepOrigin(tab.slot, origin.index + by);
      },
    };
  })();

  // The window as a command host (`clientHost.ts`): what only it can do,
  // for a command the daemon runs over it — an agent's `oxplow.tab.*`, in
  // the agent's own thread (never switching the thread or stream shown).
  const windowHandlers = useMemo<ClientHandlers>(() => {
    const threadOf = (ctx: ClientCallContext) => {
      const thread = ctx.threadId ?? selectedThreadId;
      if (!thread) throw new Error("no thread is shown");
      return thread;
    };
    const refOf = (input: unknown) => {
      const id = (input as { uri?: unknown } | null)?.uri;
      if (typeof id !== "string") throw new Error("`uri` is a page's URI");
      const ref = refFromTabId(id);
      if (!ref) throw new Error(`\`${id}\` isn't a page's ref`);
      return ref;
    };
    const open = async (input: unknown, ctx: ClientCallContext, focus: boolean) => {
      const thread = threadOf(ctx);
      const ref = refOf(input);
      if (thread === selectedThreadId && focus) {
        handleOpenPageRef.current?.(ref);
        return { uri: ref.id, focused: true };
      }
      // A working-tree file is read first: a path that isn't a file
      // opens nothing.
      const path = diskFilePath(ref.id);
      const fileStream = streams.find((candidate) => candidate.id === streamOfThread(threadStates, thread));
      if (path !== null && fileStream) {
        const buffer = await buffers.open(fileStream, path);
        if (buffer.state === "missing") throw new Error(`\`${path}\` isn't a file in the working tree`);
        if (buffer.state === "error") throw new Error(buffer.error ?? `couldn't read \`${path}\``);
      }
      updateSession(thread, (session) => {
        const opened = sessionOpen(session, { ref });
        // Opened without focus, it waits behind what the thread shows.
        return focus ? opened : { ...opened, active: session.active, mru: [...session.mru, ...opened.mru.filter((m) => !session.mru.includes(m))] };
      });
      return { uri: ref.id, focused: focus };
    };
    return {
      "editor.write": {
        // The file `uri` names, else the one the thread shows.
        save: async (input, ctx) => {
          const thread = threadOf(ctx);
          const id = (input as { uri?: unknown } | null)?.uri;
          const shown = thread === selectedThreadId ? centerActive : activeRef(sessions[thread])?.id;
          const tab = typeof id === "string" ? id : shown;
          const path = tab ? diskFilePath(tab) : null;
          // The page shown that isn't a file saves through its own Save
          // (`useTabEditing`): a wiki page's draft, say.
          const ownSave = typeof id !== "string" && thread === selectedThreadId && tab ? tabSavesRef.current[tab] : undefined;
          if (path === null && ownSave) {
            ownSave();
            return { saved: tab };
          }
          if (path === null) throw new Error(typeof id === "string" ? `\`${id}\` isn't a file in the working tree` : "nothing to save on the page shown");
          const streamId = streamOfThread(threadStates, thread);
          if (!streamId) throw new Error(`no stream has thread \`${thread}\``);
          await buffers.save(streamId, path, ctx.call);
          return { saved: path };
        },
      },
      "window.show": {
        find: () => {
          // The page shown finds in what it edits (`useTabEditing`).
          const find = tabFindsRef.current[centerActive];
          if (!find) throw new Error("nothing to find in on the page shown");
          find();
          return null;
        },
        quick_open: (input) => {
          if (!stream) throw new Error("no stream is shown");
          const query = (input as { query?: unknown } | null)?.query;
          setQuickOpenQuery(typeof query === "string" ? query : "");
          setQuickOpenVisible(true);
          return null;
        },
        // Takes the person to a ref (`tabs/navigation.ts`): the page opens
        // and is focused in the thread it belongs to, then the window
        // switches to that stream and thread.
        navigate: async (input) => {
          const plan = await planNavigation(input, threadStates, stream?.id ?? null);
          const { replace, new_tab: newTab } = (input ?? {}) as { replace?: unknown; new_tab?: unknown };
          if (plan.page) {
            if (!plan.threadId) throw new Error(`stream \`${plan.streamId}\` has no thread`);
            // As a browser follows a link: the shown tab goes there (Back
            // returns, or not with `replace`); `new_tab` gives it a tab of
            // its own.
            const shownTab = activeRef(sessions[plan.threadId])?.id;
            if (newTab !== true && shownTab && plan.threadId === selectedThreadId) {
              handleNavigateInTab(shownTab, plan.page, undefined, replace === true);
            } else {
              await open({ uri: plan.page.id }, { threadId: plan.threadId, actor: "human", call: null }, true);
            }
          }
          if (stream?.id !== plan.streamId) await handleSwitch(plan.streamId);
          if (plan.selectThread && plan.threadId) await handleSelectThread(plan.streamId, plan.threadId);
          return { opened: plan.page?.id ?? null, stream: plan.streamId };
        },
      },
      // It fills the shown thread's agent input, unsent.
      "agent_input.write": {
        draft: (input) => {
          const text = (input as { text?: unknown } | null)?.text;
          if (typeof text !== "string") throw new Error("`text` is the draft");
          insertIntoAgent(text);
          return null;
        },
      },
      // The app shell's: projects and windows.
      "projects.write": {
        create: async () => {
          await pickAndCreateProject();
          return null;
        },
        open: async (input) => {
          const { path, new_window } = (input ?? {}) as { path?: unknown; new_window?: unknown };
          if (typeof path === "string") await openProject(path, new_window === true);
          else await pickAndOpenProject(new_window === true);
          return null;
        },
      },
      "tabs.write": {
        open: (input, ctx) => open(input, ctx, false),
        focus: (input, ctx) => open(input, ctx, true),
        close: (input, ctx) => {
          const thread = threadOf(ctx);
          const ref = refOf(input);
          if (thread === selectedThreadId) {
            closePageTab(ref.id);
          } else {
            updateSession(thread, (session) => {
              const slot = slotShowing(session, ref.id);
              return slot ? sessionClose(session, slot) : session;
            });
          }
          return { uri: ref.id, closed: true };
        },
      },
    };
    // getFileSession / mutateFileSession are plain functions over the
    // session state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedThreadId, threadStates, closePageTab, updateSession, centerActive, sessions, stream, streams, setCenterActive, handleNavigateInTab]);
  const windowHandlersRef = useRef(windowHandlers);
  windowHandlersRef.current = windowHandlers;
  useEffect(() => startClientHost(() => windowHandlersRef.current), []);

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

  // What an agent shows in the shown thread (`oxplow_show_lens`) opens as
  // a tab of it, behind what the person is on: the Answers panel counts
  // it, and the person's terminal keeps the keys.
  useEffect(() => {
    if (!selectedThreadId) return;
    const thread = selectedThreadId;
    return watchShownAnswers(thread, (uri) => {
      updateSession(thread, (session) => {
        const opened = sessionOpen(session, { ref: { id: uri } });
        return { ...opened, active: session.active, mru: [...session.mru, ...opened.mru.filter((m) => !session.mru.includes(m))] };
      });
    });
  }, [selectedThreadId, updateSession]);

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

  // Keep the forward ref in sync with the latest handleOpenPage. Used by
  // the forms and offers (declared above handleOpenPage) so menu/keyboard
  // dispatches route through the same page-tab opener.
  useEffect(() => {
    handleOpenPageRef.current = handleOpenPage;
  }, [handleOpenPage]);

  // The terminal link provider hands us absolute paths (resolved against
  // stream.worktree_path); the open-file path takes a workspace-relative
  // one, so trim the worktree prefix when present.
  const openTerminalPath = useCallback(
    (absPath: string, line?: number, column?: number) => {
      if (!stream) return;
      const wt = stream.worktree_path.endsWith("/") ? stream.worktree_path.slice(0, -1) : stream.worktree_path;
      const rel = absPath.startsWith(wt + "/") ? absPath.slice(wt.length + 1) : absPath;
      if (typeof line === "number" && line > 0) {
        void handleNavigateToLocation({ path: rel, line, column: column ?? 1 });
      } else {
        void handleOpenFile(rel);
      }
    },
    // handleNavigateToLocation / handleOpenFile are plain functions over state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [stream],
  );

  const extensions = useExtensions();
  // Every enabled extension's pages: the routes and the renderers.
  const declaredPages = useMemo(
    () => (extensions ?? []).filter((e) => e.enabled).flatMap((e) => e.pages),
    [extensions],
  );
  // What renders a URI no page is for: foundation's `resource`.
  const fallbackRenderer = useMemo(
    () => declaredPages.find((p) => p.body.kind === "builtIn" && p.body.name === "resource") ?? null,
    [declaredPages],
  );
  const centerTabs: CenterTab[] = useMemo(() => {
    const tabs: CenterTab[] = [];
    // Every tab is a session tab (`tabs/session.ts`): files, diffs, agent
    // sessions and pages alike. A tab keeps a body mounted per page in its
    // history (`bodies`) so Back shows a page as it was; only the current
    // one is in the strip. A file's content and unsaved edits are its
    // buffer's (`editor/openFiles.ts`), its place the session's.
    const pageTabStartIdx = tabs.length;
    const threadSession = selectedThreadId ? sessions[selectedThreadId] : undefined;
    // Tab ids must be unique across the rendered list — the active-tab
    // lookup and the close button key on them — so a page shows once.
    const renderedTabIds = new Set<string>();
    // For each rendered tab: its session tab, its entry's ref, and whether
    // it is that tab's current entry.
    const placed: { tab: Tab; ref: TabRef; entry: Entry; current: boolean }[] = [];
    // One renderer per page kind. `Record<PageKind, …>` makes the table
    // exhaustive: a kind without a renderer (or a renderer for a kind that
    // no longer exists) is a compile error, not a blank tab.
    // The features a session's harness declares (`agent_harness`).
    const harnessFeatures = (harness: string): string[] => harnesses.find((h) => h.id === harness)?.features ?? [];
    // What core offers the built-in pages (`pages/builtInPages.tsx`).
    const scope: AppScope = {
      stream,
      streams,
      setStreams,
      threads: currentThreadState.threads,
      setThreadStates,
      thread: selectedThread,
      threadId: selectedThreadId,
      agentStatus: agentThreadStatus,
      harnesses,
      effectiveCenterActive,
      setCenterActive,
      vcsEnabled: workspaceContext.vcsEnabled,
      selectedFilePath: shownFilePath,
      handleOpenPage,
      handleNavigateInTab,
      closePageTab,
      handleOpenFile,
      handleNavigateToLocation,
      handleStreamCreated,
    };
    // The built-in renderers whose bodies draw on the window's own state;
    // the rest are `builtInPages`. Each reads what it shows from its URI.
    const appRenderers: Record<AppRendererName, (ref: TabRef, nav: SlotNav) => CenterTab | null> = {
      "agent-session": (ref) => {
        const sessionId = paramOf(ref.id, "id") ?? "";
        const session = selectedThreadSessions?.find((candidate) => candidate.id === sessionId) ?? null;
        const status = sessionStatuses[sessionId];
        const label = session ? session.displayName || sessionLabel(harnesses, session) : "Agent";
        // What can be done to the session: its tab's right-click menu and its
        // page's actions in the nav bar.
        const sessionMenu: MenuItem[] = [
            {
              id: "agent-session.rename",
              label: "Rename…",
              enabled: !!session,
              run: () => setRenamingSessionId(sessionId),
            },
            // Its harness's saved conversations: switch to one, or fork one.
            ...(session && harnessFeatures(session.harness).includes("conversation_list")
              ? [
                  {
                    id: "agent-session.switch-conversation",
                    label: "Switch Conversation…",
                    enabled: true,
                    run: () => setConversationPicker({ sessionId, mode: "resume" }),
                  },
                  ...(harnessFeatures(session.harness).includes("conversation_fork")
                    ? [
                        {
                          id: "agent-session.fork-conversation",
                          label: "Fork Conversation…",
                          enabled: true,
                          run: () => setConversationPicker({ sessionId, mode: "fork" as const }),
                        },
                      ]
                    : []),
                ]
              : []),
            // A terminal agent's process: stopping keeps the slot and the
            // saved conversation, and it stays stopped until started.
            ...(session
              ? [
                  session.stoppedByUser
                    ? {
                        id: "agent-session.start",
                        label: "Start Agent",
                        enabled: true,
                        run: () => void startAgentSession(sessionId).catch(() => undefined),
                      }
                    : {
                        id: "agent-session.stop",
                        label: "Stop Agent",
                        enabled: true,
                        run: () => void stopAgentSession(sessionId).catch(() => undefined),
                      },
                ]
              : []),
          ];
        return {
          id: ref.id,
          label,
          closable: true,
          pinned: true,
          agentStatus: status?.status ?? "waiting",
          question: status?.question,
          icon: session ? harnesses.find((h) => h.id === session.harness)?.icon : undefined,
          contextMenu: sessionMenu,
          render: () => (
            <AgentSessionPage
              thread={selectedThread}
              session={session}
              title={label}
              harnessIcon={session ? harnesses.find((h) => h.id === session.harness)?.icon : undefined}
              actions={sessionMenu.map((m) => ({ id: m.id, label: m.label, disabled: !m.enabled, run: () => m.run?.() }))}
              stream={stream}
              visible={effectiveCenterActive === ref.id}
              onOpenFile={openTerminalPath}
              canChooseConversation={!!session && harnessFeatures(session.harness).includes("conversation_list")}
              picker={conversationPicker?.sessionId === sessionId ? conversationPicker.mode : null}
              onPicker={(mode) => setConversationPicker(mode ? { sessionId, mode } : null)}
            />
          ),
        };
      },
      lens: (ref, nav) => {
        const lensId = paramOf(ref.id, "lens") ?? "";
        return {
          id: ref.id,
          label: lensId,
          closable: true,
          render: () => (
            <LensPage lensId={lensId} params={resourceParamsOf(ref)} stream={stream} onOpenPage={nav.navOpen} />
          ),
        };
      },
    };
    // A route (`oxplow:oxplow/page?page=…`): what its declaration says draws
    // it — a lens, a custom component, or (oxplow's own, until the kit can
    // draw it) a built-in page.
    const routeTab = (ref: TabRef, nav: SlotNav): CenterTab | null => {
        const page = pagePayload(ref)!;
        const declared = declaredPage(extensions, page);
        const builtIn =
          declared?.body.kind === "builtIn"
            ? declared.body.name
            : extensions === null && page.extension === null && isBuiltInPage(page.page)
              ? page.page
              : null;
        if (builtIn && isBuiltInPage(builtIn)) {
          const tab = builtInPages[builtIn]({ ref, params: page.params, nav, scope });
          return tab ? { ...tab, label: tab.label || (declared ? tabLabel(declared, page.params) : page.page) } : null;
        }
        return {
          id: ref.id,
          label: declared ? tabLabel(declared, page.params) : page.page,
          closable: true,
          render: () => (
            <DeclaredPageView page={page} declared={declared} stream={stream} threadId={selectedThread?.id ?? null} onOpenPage={nav.navOpen} />
          ),
        };
    };
    // What draws a tab: a route's declaration, else the page that renders
    // its URI (`pages/renderers.ts`: its resource's, as the URI's view and
    // the declarations pick), else foundation's fallback, `resource`.
    const renderTab = (ref: TabRef, nav: SlotNav): CenterTab | null => {
      if (typeOf(ref.id) === "oxplow/page") return routeTab(ref, nav);
      // A symbol opens its file at its line (openSymbol); it's never a tab.
      if (typeOf(ref.id) === "oxplow/symbol") return null;
      // Until the extensions load, nothing says what renders it.
      if (extensions === null) return { id: ref.id, label: "", closable: true, render: () => null };
      const page = rendererFor(ref.id, declaredPages, viewParamsOf(ref)) ?? fallbackRenderer;
      if (!page) return null;
      const tab = renderBody(page, ref, nav);
      return tab ? { ...tab, label: tab.label || tabLabel(page, parseUri(ref.id)?.params ?? {}), ...(page.pinned ? { pinned: true } : {}) } : null;
    };
    const renderBody = (page: ExtensionPage, ref: TabRef, nav: SlotNav): CenterTab | null => {
      switch (page.body.kind) {
        case "builtIn": {
          const name = page.body.name;
          if (isAppRenderer(name)) return appRenderers[name](ref, nav);
          if (!isBuiltInPage(name)) return null;
          return builtInPages[name]({ ref, params: parseUri(ref.id)?.params ?? {}, nav, scope });
        }
        case "lens": {
          const lens = page.body.lens;
          return {
            id: ref.id,
            label: tabLabel(page, parseUri(ref.id)?.params ?? {}),
            closable: true,
            // A renderer's lens is run with the URI it shows.
            render: () => (
              <LensPage lensId={lens} title={tabLabel(page, parseUri(ref.id)?.params ?? {})} titleFrom={labelReadsRow(page) ? (row) => tabLabel(page, parseUri(ref.id)?.params ?? {}, row) : undefined} params={{ uri: identityOf(ref.id) ?? ref.id }} renders={ref.id} stream={stream} onOpenPage={nav.navOpen} comments={page.comments} threadId={selectedThread?.id ?? null} />
            ),
          };
        }
        case "component": {
          const shown: PagePayload = { extension: page.extension, page: page.id, params: { uri: identityOf(ref.id) ?? ref.id } };
          return {
            id: ref.id,
            label: tabLabel(page, parseUri(ref.id)?.params ?? {}),
            closable: true,
            render: () => (
              <DeclaredPageView page={shown} declared={page} stream={stream} threadId={selectedThread?.id ?? null} onOpenPage={nav.navOpen} />
            ),
          };
        }
      }
    };
    for (const sessionTab of threadSession?.tabs ?? []) {
      // Navigation from any of the tab's bodies (a hidden back page too)
      // moves the tab.
      const currentId = currentEntry(sessionTab).ref.id;
      const navOpen = (newRef: TabRef, opts?: import("./tabs/PageNavigationContext.js").NavigateOptions) => {
        if (opts?.newTab) handleOpenPage(newRef);
        else handleNavigateInTab(currentId, newRef, opts?.origin, opts?.replace);
      };
      const navOpenFile = (path: string, opts?: { newTab?: boolean }) => {
        if (opts?.newTab) handleOpenPage(fileRef(path));
        else handleNavigateInTab(currentId, fileRef(path));
      };
      // A diff in this tab: Back returns to the page it came from.
      const navOpenDiff = (spec: DiffSpec, origin?: Origin) => handleOpenDiffInTab(currentId, spec, origin);
      const slotNav: SlotNav = { slotId: currentId, navOpen, navOpenFile, navOpenDiff };
      for (const body of bodies(sessionTab)) {
        const ref = body.entry.ref;
        if (renderedTabIds.has(ref.id)) continue;
        renderedTabIds.add(ref.id);
        const tab = renderTab(ref, slotNav);
        if (!tab) continue;
        tabs.push({ ...tab, key: body.key, ...(body.current ? {} : { hidden: true }) });
        placed.push({ tab: sessionTab, ref, entry: body.entry, current: body.current });
      }
    }
    // Wrap each page-tab render with PageNavigationContext so descendants
    // (BacklinksList, RouteLink, in-page cross-references) can navigate
    // in-tab and the Page chrome mounts its nav bar.
    for (let i = pageTabStartIdx; i < tabs.length; i++) {
      const tab = tabs[i]!;
      const { tab: sessionTab, ref, entry, current: isCurrent } = placed[i - pageTabStartIdx]!;
      const slot = sessionTab.slot;
      const tabId = tab.id;
      const currentId = currentEntry(sessionTab).ref.id;
      const origin: Origin | null = isCurrent ? (currentEntry(sessionTab).from ?? null) : null;
      const innerRender = tab.render;
      const bookmarkScope = bookmarks.find((b) => b.ref.id === ref.id)?.scope ?? null;
      const registeredTitle = pageTitles[tabId];
      const navValue: PageNavigation = {
        navigate: (newRef, opts) => {
          // A new tab with history state (a compare's two texts) is its
          // own tab; without, the tab already showing the page, or a new one.
          if (opts?.newTab && opts.state) openTab(newRef, { historyState: opts.state });
          else if (opts?.newTab) handleOpenPage(newRef);
          else handleNavigateInTab(currentId, newRef, opts?.origin, opts?.replace);
        },
        openRef: (target, how) =>
          openRefRef.current(
            target,
            how ?? {},
            (opened, h) => {
              if (h.newTab) handleOpenPage(opened);
              else handleNavigateInTab(currentId, opened, h.origin, h.replace);
            },
            () => closePageTab(currentId),
          ),
        goBack: () => handleGoBack(slot),
        goForward: () => handleGoForward(slot),
        close: () => closePageTab(currentId),
        canGoBack: sessionTab.at > 0,
        canGoForward: sessionTab.at < sessionTab.entries.length - 1,
        pinned: !!tab.pinned,
        origin,
        stepOrigin: origin ? (index: number) => handleStepOrigin(slot, index) : undefined,
        setTitle: (t: string) => setPageTitle(tabId, t),
        title: registeredTitle,
        setEditing: isCurrent ? editingSetterFor(tabId) : undefined,
        ask: parseUri(ref.id) ? { ref: ref.id } : undefined,
        bookmark: {
          scope: bookmarkScope,
          toggle: (scope: BookmarkScope) => {
            const viewer = { threadId: selectedThreadId, streamId: stream?.id ?? null };
            const write = bookmarkScope === scope
              ? removeBookmark(viewer, ref.id)
              : setBookmark(viewer, ref, registeredTitle ?? tab.label, scope);
            write.catch((e: unknown) =>
              recordOpError({ label: "Bookmark", message: e instanceof Error ? e.message : String(e) }));
          },
        },
        ref,
        viewParams: viewParamsOf(ref),
        // Only the page the tab shows changes the tab; a hidden back page
        // drawing an older entry can't.
        setViewParams: isCurrent && selectedThreadId
          ? (params: Record<string, SqlCell>, opts?: { push?: boolean }) => {
              const thread = selectedThreadId;
              // A page writes its own view; the shell's keys (the renderer
              // pick) stay.
              updateSession(thread, (session) => changeView(session, slot, (now) => ({ ...shellView(now), ...params }), opts));
            }
          : undefined,
        patchViewParams: isCurrent && selectedThreadId
          ? (patch: Record<string, SqlCell | undefined>, opts?: { push?: boolean }) => {
              const thread = selectedThreadId;
              updateSession(thread, (session) =>
                changeView(session, slot, (now) => {
                  const next: Record<string, SqlCell> = { ...now };
                  for (const [k, v] of Object.entries(patch)) {
                    if (v === undefined) delete next[k];
                    else next[k] = v;
                  }
                  return next;
                }, opts),
              );
            }
          : undefined,
        historyState: entry.historyState && typeof entry.historyState === "object" && !Array.isArray(entry.historyState) ? (entry.historyState as Record<string, unknown>) : undefined,
        setHistoryState: isCurrent && selectedThreadId
          ? (patch: Record<string, unknown>) => {
              const thread = selectedThreadId;
              updateSession(thread, (session) => sessionPatchHistoryState(session, slot, patch));
            }
          : undefined,
      };
      if (registeredTitle && registeredTitle !== tab.label) {
        tab.label = registeredTitle;
      }
      if (isCurrent && dirtyTabs[tabId]) tab.dirty = true;
      tab.render = () => (
        <PageNavigationContext.Provider value={navValue}>
          {innerRender()}
        </PageNavigationContext.Provider>
      );
    }
    // Snapshot the resolved labels for the history recorder so RailHud
    // History shows the same text the tab strip shows.
    const labelMap: Record<string, string> = {};
    for (const t of tabs) labelMap[t.id] = t.label;
    tabLabelByIdRef.current = labelMap;
    return tabs;
  }, [
    selectedThread,
    agentThreadStatus,
    selectedThreadSessions,
    sessionStatuses,
    harnesses,
    openTerminalPath,
    effectiveCenterActive,
    stream,
    handleOpenWiki,
    handleOpenCommit,
    handleOpenExternalUrl,
    selectedThreadId,
    sessions,
    updateSession,
    handleOpenPage,
    handleNavigateInTab,
    handleOpenDiffInTab,
    handleGoBack,
    handleGoForward,
    handleStepOrigin,
    closePageTab,
    pageTitles,
    setPageTitle,
    dirtyTabs,
    editingSetterFor,
    openTab,
    bookmarks,
    workspaceContext.vcsEnabled,
    shownFilePath,
    centerActive,
    extensions,
  ]);

  return (
    // One owner of every extension panel's runs, read by the rail, the
    // status bar's bell and the Alerts page.
    <PanelRunsProvider streamId={stream?.id ?? null} threadId={selectedThreadId}>
    <AlertToasts onReview={(lens) => handleOpenPage(lens ? lensRef(lens) : alertsRef())} />
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden" }}>
      {/* macOS uses an Overlay titlebar (transparent, hidden title), so the
          webview reaches the top edge: the title bar is the window's top,
          its empty space drags it and its start leaves room for the
          floating traffic lights. Elsewhere it sits under the menu bar. */}
      {!isMac ? <Menubar groups={menuGroups} /> : null}
      <TitleBar
        stream={stream}
        thread={selectedThread ? { id: selectedThread.id, title: selectedThread.title } : null}
        vcsEnabled={workspaceContext.vcsEnabled}
        leftInset={isMac ? 78 : 10}
        onOpenSearch={() => setQuickOpenVisible(true)}
      />
      <div style={{ borderBottom: error ? "1px solid var(--border)" : undefined, flexShrink: 0 }}>
        {error ? (
          <div
            onClick={() => setError(null)}
            title="Click to dismiss"
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 8,
              padding: "2px 12px",
              background: "var(--bg-2)",
              color: "#ff6b6b",
              fontSize: 11,
              minHeight: 22,
              borderBottom: "1px solid var(--border)",
              cursor: "pointer",
            }}
          >
            <span>{error}</span>
            <span aria-hidden style={{ opacity: 0.7, paddingLeft: 8 }}>
              ✕
            </span>
          </div>
        ) : null}
      </div>
      <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <Navigator
          streams={streams}
          currentStreamId={stream?.id ?? null}
          threadStates={threadStates}
          streamStatuses={streamStatuses}
          agentStatuses={agentStatuses}
          agentQuestions={agentQuestions}
          openHandoffs={openHandoffs}
          onSwitchStream={handleSwitch}
          onSelectThread={handleSelectThread}
          onCreateThread={async (streamId, title) => {
            if (streamId !== stream?.id) await handleSwitch(streamId);
            await handleCreateThread(title);
          }}
          onOpenNewStreamPage={() => handleOpenPage(newStreamRef())}
          onRenameStream={handleRenameStreamById}
          onRenameThread={handleRenameThreadById}
          onMarkThread={handleMarkThread}
          onCloseThread={handleCloseThread}
          onOpenStreamSettings={(streamId) => handleOpenPage(streamSettingsRef(streamId))}
          onOpenThreadSettings={(threadId) => handleOpenPage(threadSettingsRef(threadId))}
          vcsEnabled={workspaceContext.vcsEnabled}
        />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0, background: "var(--surface-chrome)" }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <RailHud
          streamId={stream?.id ?? null}
          onOpenPage={handleOpenPage}
        />
        <div style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
          minWidth: 0,
          overflow: "hidden",
          // The main content area floats as a panel on the chrome: the
          // left side is curved (revealing chrome), the whole right side
          // stays flush/square against the window edge. The border is
          // drawn on the panel itself so the corners stay closed
          // regardless of tab state.
          marginLeft: 6,
          border: "1px solid var(--border-strong)",
          borderTopLeftRadius: 8,
          borderBottomLeftRadius: 8,
        }}>
          {stream ? (
            <CenterTabs
              tabs={centerTabs}
              activeId={effectiveCenterActive}
              onActivate={(id) => setCenterActive(id)}
              onClose={(id) => {
                const path = diskFilePath(id);
                if (path !== null) handleCloseOpenFile(id, path);
                else if (id.startsWith("diff:")) closeDiffTab(id);
                else if (typeOf(id) === "oxplow/agent_session") {
                  // Closing a session's tab closes the session at once, its
                  // toast offering Undo (it reopens on the saved
                  // conversation); its tab goes when its row closes.
                  const ref = refFromTabId(id);
                  const sessionId = ref ? paramOf(ref.id, "id") : null;
                  if (sessionId) {
                    const session = selectedThreadSessions?.find((c) => c.id === sessionId);
                    const name = session ? session.displayName || sessionLabel(harnesses, session) : "session";
                    void personCommands.run(`Close ${name}`, "oxplow.agent_session.close", {
                      session: sessionId,
                    });
                  }
                }
                else closePageTab(id);
              }}
              onReorder={handleReorderCenterTabs}
              header={renamingSessionId ? (() => {
                const renaming = selectedThreadSessions?.find((c) => c.id === renamingSessionId);
                const harness = renaming ? harnesses.find((h) => h.id === renaming.harness) : undefined;
                const harnessTitle = harness?.title ?? renaming?.harness ?? "the agent";
                const hint = renameHint({
                  title: renaming?.title ?? "",
                  nativeTitle: renaming?.nativeTitle ?? "",
                  displayName: renaming?.displayName ?? "",
                  harnessTitle,
                  nativeRename: !!harness?.features.includes("native_rename"),
                });
                return (
                  <InlinePromptStrip
                    testId="rename-session"
                    message={hint.message}
                    confirmLabel="Rename"
                    fields={[{ key: "title", initialValue: renaming?.title ?? "", placeholder: hint.placeholder, optional: true }]}
                    toggle={hint.toggleLabel ? { key: "native", label: hint.toggleLabel } : undefined}
                    onCancel={() => setRenamingSessionId(null)}
                    onSubmit={(values) => {
                      const sessionId = renamingSessionId;
                      setRenamingSessionId(null);
                      void renameSession({
                        alias: values.title ?? "",
                        native: values.native === "true",
                        harnessTitle,
                        setAlias: (alias) => renameAgentSession(sessionId, alias),
                        setNative: (name) => renameNativeConversation(sessionId, name),
                      }).catch((e: unknown) =>
                        recordOpError({ label: "Rename session", message: e instanceof Error ? e.message : String(e) }),
                      );
                    }}
                  />
                );
              })() : undefined}
            />
          ) : <div style={{ padding: 12 }}>loading…</div>}
          {/* Generic comment layer for every plain-DOM page. Mounted once
              here (not per page): captureSelection only fires inside a
              data-ref-* region and skips editor/terminal surfaces, so a
              single instance serves the tasks list, commit pages, finding
              pages, etc. without per-page wiring. */}
          {stream ? (
            <DomCommentLayer
              streamId={stream.id}
              threadId={currentThreadState.selectedThreadId ?? null}
            />
          ) : null}
        </div>
        </div>
        <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "flex-end",
          gap: 8,
          padding: "4px 10px",
          background: "var(--bg-2)",
          flexShrink: 0,
          minHeight: 26,
          // Gap above the bottom rail, matching the other panel gaps; the
          // chrome behind shows through.
          marginTop: 6,
        }}
      >
        <StatusBar onOpenPage={handleOpenPage} />
      </div>
        </div>
      </div>
      <QuickOpenOverlay
        open={quickOpenVisible}
        initialQuery={quickOpenQuery}
        stream={stream}
        threadId={selectedThreadId}
        selectedFilePath={shownFilePath}
        pages={settingsAreaEntries()}
        offers={offers}
        onClose={() => setQuickOpenVisible(false)}
        onOpenFile={(path) => {
          void handleOpenFile(path);
        }}
        onOpenPage={(ref) => {
          handleOpenPage(ref);
        }}
        onOpenSearchHit={openSearchHit}
      />
      {daemonUnavailable ? <DaemonDownDialog /> : null}
      <UndoToastStack />
      <PersonCommandConfirm />
      <RemoteConnectionBanner />
    </div>
    </PanelRunsProvider>
  );
}

/** A toast as each new thing needs the person; Review opens Alerts (a badge's, its lens). */
function AlertToasts({ onReview }: { onReview(lens?: string): void }) {
  const { items } = useAlerts();
  useAlertToasts(items, onReview);
  return null;
}

/**
 * Fill empty diff sides with a readable placeholder so the Monaco diff view
 * doesn't just show blank text with no explanation. State flags from the
 * snapshot store tell us why content is missing.
 */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    // Checkbox / button / radio inputs shouldn't block the shortcut — they
    // don't swallow typed characters the way a text field does.
    return type === "text" || type === "search" || type === "email" || type === "url" || type === "password" || type === "" || type === "tel";
  }
  return false;
}

function DaemonDownDialog() {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0, 0, 0, 0.65)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
        padding: 24,
      }}
    >
      <div
        style={{
          width: "min(520px, 100%)",
          background: "var(--bg-2)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 20,
          boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 12px 40px rgba(0, 0, 0, 0.4)",
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>Backend daemon disconnected</div>
        <div style={{ color: "var(--muted)", lineHeight: 1.5, marginBottom: 16 }}>
          The backend daemon was killed or is no longer reachable. Stream switching, terminal panes, and hook
          updates will not keep working until the daemon is started again.
        </div>
        <button type="button"
          onClick={() => window.location.reload()}
          style={{
            background: "var(--accent)",
            color: "#fff",
            border: "none",
            padding: "8px 14px",
            borderRadius: 4,
            cursor: "pointer",
            fontFamily: "inherit",
          }}
        >
          Reload after restart
        </button>
      </div>
    </div>
  );
}
