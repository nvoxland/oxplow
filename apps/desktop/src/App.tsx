import { useRefKindsLoader } from "./refKinds.js";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { flushSync } from "react-dom";
import {
  closeThread,
  createThread,
  getThreadState,
  getConfig,
  createWorkspaceDirectory,
  type AgentStatus,
  createWorkspaceFile,
  deleteWorkspacePath,
  getCurrentStream,
  getWorkspaceContext,
  type GitOpKickoff,
  desktopBridge,
  listStreams,
  listWorkspaceEntries,
  probeDaemon,
  readWorkspaceFile,
  renameWorkspacePath,
  renameThread,
  renameStream,
  subscribeWorkspaceEvents,
  openExternalUrl,
  setGenerated,
  selectThread,
  promoteThread,
  recordUsage,
  reorderThreads,
  switchStream,
  runCommand,
  runCommandForCall,
  runCommandInBackground,
  type ThreadState,
  type AgentKind,
  type SqlCell,
  type Stream,
  type WorkspaceContext,
} from "./api.js";
import {
  applyItemChange,
  createWorkItem,
  deleteWorkItem,
  moveWorkItem,
  readWorkList,
  reorderWorkItems,
  type ItemChange,
  type NewWorkItem,
  type WorkList,
} from "./workItems.js";
import { useWorkListProfile } from "./useWorkListProfile.js";
import { workItemLabel } from "./workItemRef.js";
import {
  closeOpenFile,
  createEmptyFileSession,
  enforceOpenFileLimit,
  markFileSaved,
  openFileInSession,
  removeOpenFiles,
  renameOpenFilePaths,
  reorderOpenFiles,
  selectOpenFile,
  setLoadedFileContent,
  setOpenFileLoading,
  updateFileDraft,
  type FileSessionState,
} from "./editor-session.js";
import { buildMenuBar, buildNativeMenuSnapshots, menuItemById, OPEN_RECENT_PREFIX } from "./menuBar.js";
import { externalFileSyncAction } from "./external-file-sync.js";
import type { EditorNavigationTarget } from "./lsp.js";
import { Navigator } from "./components/Navigator.js";
import { StatusBar } from "./components/StatusBar.js";
import { TitleBar } from "./components/TitleBar.js";
import { showToast } from "./components/toastStore.js";
import { awaitGitOp, opErrorOf } from "./git-op.js";
import { UndoToastStack } from "./components/UndoToast.js";
import { parseRef } from "./refs/ref.js";
import { PersonCommandConfirm } from "./components/PersonCommandConfirm.js";
import { RemoteConnectionBanner } from "./components/RemoteConnectionBanner.js";
import { subscribeUiError } from "./ui-error.js";
import { useBackendSubscriptions } from "./useBackendSubscriptions.js";
import { useFileSessions } from "./useFileSessions.js";
import { Menubar } from "./components/Menubar.js";
import { CenterTabs, type CenterTab } from "./components/CenterTabs/CenterTabs.js";
import type { DiffSpec } from "./components/Diff/DiffPane.js";
import { DiffPage } from "./pages/DiffPage.js";
import { DuplicateBlockPage } from "./pages/DuplicateBlockPage.js";
import { FileViewerPage } from "./pages/FileViewerPage.js";
import { RailHud } from "./components/RailHud/RailHud.js";
import type { PageKind, TabRef } from "./tabs/tabState.js";
import { PageNavigationContext } from "./tabs/PageNavigationContext.js";
import { clearPageSnapshot } from "./tabs/usePageSnapshot.js";
import { planCloseOrGoBack } from "./tabs/closeOrGoBack.js";
import { removeBookmark, setBookmark, useBookmarks } from "./tabs/bookmarks.js";
import type { BookmarkScope } from "./tabs/bookmarks.js";
import { SettingsPage } from "./pages/SettingsPage.js";
import { LocalHistoryDashboardPage } from "./pages/LocalHistoryDashboardPage.js";
import { DiffViewPage } from "./pages/DiffViewPage.js";
import { GitHistoryPage } from "./pages/GitHistoryPage.js";
import { GitDashboardPage } from "./pages/GitDashboardPage.js";
import { UncommittedChangesPage } from "./pages/UncommittedChangesPage.js";
import { AgentPage } from "./pages/AgentPage.js";
import { agentLabel, sessionLabel } from "./agentKinds.js";
import { useThreadSessions } from "./agentSessions.js";
import { TerminalPage } from "./pages/TerminalPage.js";
import { HookEventsPage } from "./pages/HookEventsPage.js";
import { FilesPage } from "./pages/FilesPage.js";
import { DirectoryPage } from "./pages/DirectoryPage.js";
import { WikiIndexPage } from "./pages/WikiIndexPage.js";
import { TasksPage } from "./pages/TasksPage.js";
import { DoneWorkPage } from "./pages/DoneWorkPage.js";
import { BacklogPage } from "./pages/BacklogPage.js";
import { CommentsInboxPage } from "./pages/CommentsInboxPage.js";
import { MetricDetailPage } from "./pages/MetricDetailPage.js";
import { MetricsPage } from "./pages/MetricsPage.js";
import { CustomDashboardPage } from "./pages/CustomDashboardPage.js";
import { DashboardsIndexPage } from "./pages/DashboardsIndexPage.js";
import { LensPage } from "./pages/LensPage.js";
import { insertIntoAgent } from "./agent-input-bus.js";
import { getPageDetailStore } from "./tabs/openPageDetail.js";
import { ExploreDataPage } from "./pages/ExploreDataPage.js";
import { CatalogPage } from "./pages/CatalogPage.js";
import { BoardPage } from "./pages/BoardPage.js";
import { ProblemsPage } from "./pages/ProblemsPage.js";
import { ExtensionPageView } from "./pages/ExtensionPageView.js";
import { SymbolsPage } from "./pages/SymbolsPage.js";
import { resolveSymbol } from "./codeIntel.js";
import { ClosedThreadsPage } from "./pages/ClosedThreadsPage.js";
import { ExternalUrlPage } from "./pages/ExternalUrlPage.js";
import { WorkItemPage } from "./pages/WorkItemPage.js";
import { WikiPage } from "./pages/WikiPage.js";
import { WikiFreshnessPage } from "./pages/WikiFreshnessPage.js";
import { DashboardPage } from "./pages/DashboardPage.js";
import { StreamSettingsPage } from "./pages/StreamSettingsPage.js";
import { ThreadSettingsPage } from "./pages/ThreadSettingsPage.js";
import { NewStreamPage } from "./pages/NewStreamPage.js";
import { NewTaskPage } from "./pages/NewTaskPage.js";
import { GitCommitPage } from "./pages/GitCommitPage.js";
import { AlertsPage } from "./pages/AlertsPage.js";
import { PanelRunsProvider } from "./components/Panels/PanelRunsContext.js";
import { useAlerts } from "./components/Alerts/useAlerts.js";
import { useAlertToasts } from "./components/Alerts/useAlertToasts.js";
import { DomCommentLayer } from "./components/Comments/DomCommentLayer.js";
import { AGENT_TAB_ID, computeDiffId, diskFilePath, pageKindOf, refFromTabId, closedThreadsRef, commentsRef, dashboardsRef, directoryRef, effortDiffRef, externalUrlRef, fileRef, gitCommitRef, gitDashboardRef, indexRef, newStreamRef, newTaskRef, alertsRef, searchHitTarget, uncommittedChangesRef, wikiPageRef, streamSettingsRef, threadSettingsRef, workItemTabRef, type DiffViewPayload } from "./tabs/pageRefs.js";
import { requestNewThread } from "./new-thread-bus.js";
import { getOpErrorsStore, recordOpError } from "./components/opErrorsStore.js";
import { classifyExternalUrl } from "./external-url-allowlist.js";
import { installContextMenuSuppressor } from "./context-menu.js";
import { TerminalPane } from "./components/TerminalPane.js";
import { FilePage } from "./pages/FilePage.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { computePagesDirectory } from "./components/RailHud/sections.js";
import { NON_TRACKED_KINDS } from "./components/RailHud/history.js";
import { resolveActiveTabRef } from "./tabs/resolveActiveTabRef.js";
import { useThreadPageTabs } from "./tabs/useThreadPageTabs.js";
import { dropFromMru, MAX_PAGE_TABS, selectLruEvictions, touchMru } from "./tabs/tabLru.js";
import {
  readPersistedCenterActive,
  readPersistedFileSessionPaths,
  writePersistedCenterActive,
  writePersistedFileSessionPaths,
} from "./tabs/pageTabsPersistence.js";
import { forgetPage, generatedPaths, recordPageVisit, recordUserInterrupt, reportOpenPage } from "./api.js";
import { openProject, createProject, listRecentProjects, shellAvailable } from "./api.js";
import { onRemoteReconnect, triggerRemoteResync } from "./api.js";
import type { CommandOutcome, RecentProjectView } from "./tauri-bridge/generated/bindings.js";
import { pickFolder } from "./tauri-bridge/nativeDialog.js";
import { WORKING, shortRevisionLabel } from "./revision.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { offerForShortcut } from "./keybindings.js";
import { commandOffers, type OfferDeps } from "./commandOffers.js";
import { setRefOfferHost } from "./components/refCommands.js";
import { runLocally, startClientHost, type ClientCallContext, type ClientCallRef, type ClientHandlers } from "./clientHost.js";
import { streamOfThread, withTab, withoutTab } from "./tabs/threadTabOps.js";
import { usePersonCommands } from "./personCommandsStore.js";
import { personCommands } from "./personCommands.js";
import { logUi, setUiLogContext } from "./logger.js";

// Cap on concurrent file tabs in the center. Intellij uses ~10 by default;
// when this is exceeded, the oldest-touched tab without unsaved changes is
// closed automatically via enforceOpenFileLimit. Dirty tabs stay pinned.
const MAX_OPEN_FILE_TABS = 10;

/**
 * Take a caller-supplied siblings record, snap its `index` to the
 * position whose `ref.id` matches the destination, and drop the
 * record entirely if there's no match (a stale list shouldn't drive
 * prev/next on a page that isn't actually in it).
 */
function resolveSiblings(
  siblings: import("./tabs/PageNavigationContext.js").NavSiblings | undefined,
  ref: TabRef,
): import("./tabs/PageNavigationContext.js").NavSiblings | null {
  if (!siblings || siblings.entries.length === 0) return null;
  const matchIdx = siblings.entries.findIndex((e) => e.ref.id === ref.id);
  if (matchIdx < 0) {
    // Caller passed `siblings` but the destination isn't in the list.
    // If the supplied index points at a valid slot, trust it; otherwise
    // drop. This guards against silently rendering "1 of N" with the
    // wrong page.
    if (siblings.index < 0 || siblings.index >= siblings.entries.length) return null;
    return siblings;
  }
  return { entries: siblings.entries, index: matchIdx };
}

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

/** True when `path` is a directory in the workspace. `listWorkspaceEntries`
 * does a `read_dir`, which succeeds (even for an empty dir) only for a
 * directory and errors for a file or missing path. */
async function isWorkspaceDir(streamId: string, path: string): Promise<boolean> {
  try {
    await listWorkspaceEntries(streamId, path);
    return true;
  } catch {
    return false;
  }
}

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [threadStates, setThreadStates] = useState<Record<string, ThreadState>>({});
  const [enabledAgents, setEnabledAgents] = useState<AgentKind[]>(["claude"]);
  // Mirror of threadStates for subscription callbacks that need the
  // latest map without re-subscribing when it changes (see
  // useBackendSubscriptions). Kept current on every render.
  const threadStatesRef = useRef(threadStates);
  threadStatesRef.current = threadStates;

  useEffect(() => {
    let cancelled = false;
    void getConfig()
      .then((config) => {
        if (!cancelled && config.agents?.length) setEnabledAgents(config.agents);
      })
      .catch((e) => logUi("warn", "failed to load project config", { error: String(e) }));
    return () => {
      cancelled = true;
    };
  }, []);
  const [threadWorkStates, setThreadWorkStates] = useState<Record<string, WorkList>>({});
  const threadWorkStatesRef = useRef(threadWorkStates);
  threadWorkStatesRef.current = threadWorkStates;
  const [backlogState, setBacklogState] = useState<WorkList | null>(null);
  // The active work list: what it can do and its own fields.
  const workListProfile = useWorkListProfile();
  const [agentStatuses, setAgentStatuses] = useState<Record<string, AgentStatus>>({});
  // Parallel to agentStatuses: the question each thread is waiting on, set
  // only while that thread's status is "awaiting". Feeds the rail dot's
  // tooltip so a thread parked on your answer says WHAT it's asking.
  const [agentQuestions, setAgentQuestions] = useState<Record<string, string | undefined>>({});
  const [stream, setStream] = useState<Stream | null>(null);
  // Per-thread active center tab. The map is the source of truth; `centerActive`
  // and `setCenterActive` below are derived helpers so existing handler code
  // keeps working unchanged. Each thread remembers its last active tab so
  // switching threads restores it. A thread with none yet takes the
  // last-active tab persisted across restarts (`readPersistedCenterActive`).
  // …and the rest of the tab layout (per-thread tab lists, per-tab
  // back/forward history, diff-spec registry) lives in the
  // useThreadPageTabs hook below, which also owns its persistence.
  const {
    threadCenterActive,
    setThreadCenterActive,
    threadPageMru,
    setThreadPageMru,
    threadPageTabs,
    setThreadPageTabs,
    threadPageHistory,
    setThreadPageHistory,
    diffTabs,
    setDiffTabs,
  } = useThreadPageTabs();
  // Per-tab page titles, keyed by tab id. Pages register their title via
  // PageNavigationContext.setTitle (the usePageTitle helper). Drives both
  // the tab strip label and the shared chrome header so the title lives in
  // exactly one place.
  const [pageTitles, setPageTitles] = useState<Record<string, string>>({});
  const setPageTitle = useCallback((tabId: string, title: string) => {
    setPageTitles((prev) => (prev[tabId] === title ? prev : { ...prev, [tabId]: title }));
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
  const { fileSessions, setFileSessions, getFileSession, mutateFileSession } = useFileSessions();
  const restoredStreamsRef = useRef<Set<string>>(new Set());
  const centerActiveValidatedRef = useRef(false);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ vcsEnabled: false });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const [commitFilesRequest, setCommitFilesRequest] = useState(0);
  const [generated, setGeneratedState] = useState<{ exclude: string[]; include: string[] }>({
    exclude: [],
    include: [],
  });
  const opErrorsStore = getOpErrorsStore();
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
        const initialThread = initialThreadState.threads.find((thread) => thread.id === initialThreadState.selectedThreadId);
        if (initialThread) {
          const initialWork = await readWorkList(initialThread.id);
          setThreadWorkStates((prev) => ({ ...prev, [initialThread.id]: initialWork }));
        }
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
      // switched-to stream has no remembered selection, auto-pick the
      // active thread (or the first thread by sort order) and persist
      // that choice so subsequent switches remember it.
      if (!nextThreadState.selectedThreadId && nextThreadState.threads.length > 0) {
        const fallback =
          nextThreadState.threads.find((t) => t.id === nextThreadState.activeThreadId)
          ?? nextThreadState.threads[0];
        nextThreadState = await selectThread(next.id, fallback.id);
      }
      const nextThread = nextThreadState.threads.find((thread) => thread.id === nextThreadState.selectedThreadId);
      if (nextThread && !threadWorkStates[nextThread.id]) {
        const nextWork = await readWorkList(nextThread.id);
        setThreadWorkStates((prev) => ({ ...prev, [nextThread.id]: nextWork }));
      }
      setThreadStates((prev) => ({ ...prev, [next.id]: nextThreadState }));
      setStream(next);
      const nextSession = getFileSession(next.id);
      // Seed the new thread's center-active only if we don't already have a
      // remembered value for it. Per-thread persistence means returning to a
      // thread restores its prior tab; only initial entry uses the file-session
      // selected path as a heuristic.
      if (nextThread) {
        const seeded = nextSession.selectedPath ? fileRef(nextSession.selectedPath).id : AGENT_TAB_ID;
        setThreadCenterActive((prev) => (
          prev[nextThread.id] !== undefined ? prev : { ...prev, [nextThread.id]: seeded }
        ));
      }
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
      const thread = state.threads.find((candidate) => candidate.id === state.selectedThreadId);
      if (thread) {
        const seeded = AGENT_TAB_ID;
        setThreadCenterActive((prev) => (
          prev[thread.id] !== undefined ? prev : { ...prev, [thread.id]: seeded }
        ));
        void readWorkList(thread.id).then((work) => {
          setThreadWorkStates((prev) => ({ ...prev, [thread.id]: work }));
        });
      }
    } catch (e) {
      setError(String(e));
    }
    setStream(next);
  }

  async function handleOpenFile(path: string) {
    if (!stream) return;
    // A path that resolves to a directory (e.g. a trailing-slash link from
    // terminal/agent output) opens the Files tree rooted there, not the
    // editor. Fast-path the common dir-link form (trailing slash) so no
    // existence probe runs on the common file-open path.
    if (path.endsWith("/") && (await isWorkspaceDir(stream.id, path))) {
      setError(null);
      handleOpenPageRef.current?.(directoryRef(path));
      return;
    }

    // Add the file to the unified per-thread page tab list and make it active.
    // File tabs share the same chrome / back-forward substrate as every other
    // page kind; fileSessions owns content + dirty state, threadPageTabs owns
    // membership.
    const activateFileTab = () => {
      if (selectedThreadId) {
        const ref = fileRef(path);
        setThreadPageTabs((prev) => {
          const existing = prev[selectedThreadId] ?? [];
          if (existing.some((t) => t.id === ref.id)) return prev;
          return { ...prev, [selectedThreadId]: [...existing, ref] };
        });
      }
      setCenterActive(fileRef(path).id);
      setError(null);
      void recordUsage({
        kind: "editor-file",
        key: path,
        event: "open",
        streamId: stream.id,
        threadId: selectedThread?.id ?? null,
      }).catch(() => {});
    };

    const existing = getFileSession(stream.id).files[path];
    // Already open and loaded — just select it, no re-read.
    if (existing && !existing.isLoading) {
      mutateFileSession(stream.id, (base) =>
        enforceOpenFileLimit(selectOpenFile(base, path), MAX_OPEN_FILE_TABS),
      );
      activateFileTab();
      return;
    }

    // Read FIRST, then open the tab — so a clicked path that isn't a real file
    // (a dotted identifier in prose, a stale link) opens no tab and doesn't
    // hijack the active tab; it just shows a dismissible message.
    let file;
    try {
      logUi("debug", "open file: readWorkspaceFile start", { streamId: stream.id, path });
      file = await readWorkspaceFile(stream.id, path);
      logUi("debug", "open file: readWorkspaceFile end", {
        streamId: stream.id,
        path,
        size: file.content.length,
        lineCount: file.content.split("\n").length,
      });
    } catch (e) {
      // The clicked path is actually a directory (no trailing slash): open the
      // Files tree rooted there.
      if (await isWorkspaceDir(stream.id, path)) {
        setError(null);
        handleOpenPageRef.current?.(directoryRef(path));
        return;
      }
      const msg = String(e);
      // A simply-missing file (e.g. a stale terminal link or a dotted word in
      // prose) is benign — friendly message, no tab opened, active untouched.
      const notFound = /no such file|not found|os error 2/i.test(msg);
      if (notFound) {
        setError(`File not found: ${path}`);
        logUi("warn", "open file: target does not exist", { streamId: stream.id, path });
      } else {
        setError(msg);
        logUi("error", "failed to open file", { streamId: stream.id, path, error: msg });
      }
      return;
    }

    // Exists — open the tab, load its content, activate.
    mutateFileSession(stream.id, (base) => {
      const opened = existing
        ? selectOpenFile(base, path)
        : openFileInSession(base, path, "", false);
      return enforceOpenFileLimit(opened, MAX_OPEN_FILE_TABS);
    });
    mutateFileSession(stream.id, (s) => setLoadedFileContent(s, path, file.content));
    activateFileTab();
    logUi("info", "opened file", { streamId: stream.id, path });
  }

  /** A `symbol:` ref opens its file at the symbol's line (`v_symbol`). */
  async function openSymbol(ref: string) {
    try {
      const at = await resolveSymbol(ref);
      if (!at) {
        recordOpError({ label: "Open symbol", message: `${ref} isn't a known symbol now (the file changed, or its language server isn't running).` });
        return;
      }
      await handleNavigateToLocation({ path: at.path, line: at.line, column: at.col });
    } catch (e) {
      recordOpError({ label: "Open symbol", message: e instanceof Error ? e.message : String(e) });
    }
  }

  async function handleNavigateToLocation(target: EditorNavigationTarget) {
    await handleOpenFile(target.path);
    setEditorNavigationTarget(target);
    setCenterActive(fileRef(target.path).id);
  }

  function handleEditorChange(value: string) {
    if (!stream) return;
    const session = getFileSession(stream.id);
    if (!session.selectedPath) return;
    mutateFileSession(stream.id, (s) => updateFileDraft(s, session.selectedPath!, value));
  }

  /** Save `path`'s unsaved changes in `streamId`'s session; throws what
   *  went wrong. */
  /** Save `path` — for the daemon's `call` when answering one (an
   *  agent's Save, written as the agent), else as the person. */
  async function saveFile(streamId: string, path: string, call: ClientCallRef | null = null) {
    const current = getFileSession(streamId).files[path];
    if (!current) throw new Error(`\`${path}\` isn't open`);
    if (current.isLoading) throw new Error(`\`${path}\` is still loading`);
    mutateFileSession(streamId, (s) => setOpenFileLoading(s, path, true));
    try {
      // An audited write that logs `file.saved` (its content isn't kept
      // in the record).
      const content = current.draftContent;
      const input = { stream: streamId, path, content };
      await (call ? runCommandForCall(call, "oxplow.file.save", input) : runCommand("oxplow.file.save", input));
      mutateFileSession(streamId, (s) => markFileSaved(s, path, content));
      logUi("info", "saved file", { streamId, path });
    } catch (e) {
      mutateFileSession(streamId, (s) => setOpenFileLoading(s, path, false));
      logUi("error", "failed to save file", { streamId, path, error: String(e) });
      throw e;
    }
  }

  async function handleEditorSave() {
    if (!stream) return;
    const selectedPath = getFileSession(stream.id).selectedPath;
    if (!selectedPath) return;
    try {
      await saveFile(stream.id, selectedPath);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  function handleSelectOpenFile(path: string) {
    if (!stream) return;
    mutateFileSession(stream.id, (s) => selectOpenFile(s, path));
    setCenterActive(fileRef(path).id);
  }

  function handleCloseOpenFile(path: string) {
    if (!stream) return;
    // Guard against silently dropping unsaved edits when a user closes a
    // dirty tab via the × or Cmd+W. Phase-5 redesign: fire-and-undo
    // instead of a blocking confirm — close immediately, surface a
    // toast that offers Undo for ~7s. The toast captures the draft so
    // undo restores the unsaved buffer; if the user lets the toast
    // expire, the draft is gone (same end-state as the old "Discard").
    const currentFile = getFileSession(stream.id).files[path];
    const targetStream = stream;
    if (currentFile && currentFile.draftContent !== currentFile.savedContent) {
      const basename = path.split("/").pop() ?? path;
      const stashed = {
        savedContent: currentFile.savedContent,
        draftContent: currentFile.draftContent,
      };
      closeOpenFileNow(path);
      showToast({
        message: `Closed "${basename}" with unsaved changes.`,
        actionLabel: "Undo",
        onUndo: () => {
          mutateFileSession(targetStream.id, (session) => {
            const restored = setLoadedFileContent(session, path, stashed.savedContent);
            return updateFileDraft(restored, path, stashed.draftContent);
          });
          setCenterActive(fileRef(path).id);
        },
      });
      return;
    }
    closeOpenFileNow(path);
  }

  function closeOpenFileNow(path: string) {
    if (!stream) return;
    mutateFileSession(stream.id, (s) => closeOpenFile(s, path));
    setEditorNavigationTarget((current) => (current?.path === path ? null : current));
  }

  async function handleCreateFile(path: string) {
    if (!stream) return;
    const created = await createWorkspaceFile(stream.id, path, "");
    setError(null);
    await handleOpenFile(created.path);
  }

  async function handleCreateDirectory(path: string) {
    if (!stream) return;
    await createWorkspaceDirectory(stream.id, path);
    setError(null);
  }

  async function handleRenamePath(fromPath: string, toPath: string) {
    if (!stream) return;
    const renamed = await renameWorkspacePath(stream.id, fromPath, toPath);
    setError(null);
    mutateFileSession(stream.id, (s) =>
      renameOpenFilePaths(s, (path) => {
        if (path === renamed.fromPath) return renamed.toPath;
        if (path.startsWith(renamed.fromPath + "/")) {
          return `${renamed.toPath}${path.slice(renamed.fromPath.length)}`;
        }
        return path;
      }),
    );
    setEditorNavigationTarget((current) => {
      if (!current) return current;
      if (current.path === renamed.fromPath) {
        return { ...current, path: renamed.toPath };
      }
      if (current.path.startsWith(renamed.fromPath + "/")) {
        return { ...current, path: `${renamed.toPath}${current.path.slice(renamed.fromPath.length)}` };
      }
      return current;
    });
  }

  async function handleDeletePath(path: string) {
    if (!stream) return;
    await deleteWorkspacePath(stream.id, path);
    setError(null);
    mutateFileSession(stream.id, (current) => {
      const toRemove = current.openOrder.filter((candidate) => candidate === path || candidate.startsWith(path + "/"));
      return removeOpenFiles(current, toRemove);
    });
    setEditorNavigationTarget((current) => {
      if (!current) return current;
      return current.path === path || current.path.startsWith(path + "/") ? null : current;
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
      const thread = next.threads.find((candidate) => candidate.id === threadId);
      if (thread) {
        const work = await readWorkList(thread.id);
        setThreadWorkStates((prev) => ({ ...prev, [thread.id]: work }));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateThread(title: string, agent?: AgentKind, acpAgent?: string | null) {
    if (!stream) return;
    try {
      const next = await createThread(stream.id, title, agent, acpAgent);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      const thread = next.threads.find((candidate) => candidate.id === next.selectedThreadId);
      if (thread) {
        const work = await readWorkList(thread.id);
        setThreadWorkStates((prev) => ({ ...prev, [thread.id]: work }));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handlePromoteThread(threadId: string) {
    if (!stream) return;
    try {
      const next = await promoteThread(stream.id, threadId);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      setCenterActive(AGENT_TAB_ID);
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

  async function handleReorderThreads(orderedThreadIds: string[]) {
    if (!stream) return;
    try {
      await reorderThreads(stream.id, orderedThreadIds);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  // Work-item writes are work_item.* commands, by ref; the thread's and
  // the backlog's lists re-read from the models when they change
  // (useBackendSubscriptions), so a handler only runs the command.
  async function runTaskWrite(write: () => Promise<unknown>) {
    try {
      await write();
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleCreateTask(input: NewWorkItem) {
    if (!selectedThread) return;
    await runTaskWrite(() => createWorkItem(selectedThread.id, input));
  }

  async function handleUpdateTask(ref: string, change: ItemChange) {
    await runTaskWrite(() => applyItemChange(ref, change));
  }

  /** The person chose Delete (a right-click menu item, the item page's
   *  confirm): that is the confirmation the command asks for. */
  async function handleDeleteTask(ref: string) {
    await runTaskWrite(() => deleteWorkItem(ref, true));
    // If the deleted item is open in a tab, go back in its history rather
    // than leaving a stale page (or close it if there's nothing to go
    // back to).
    closeOrGoBackPageTab(workItemTabRef(ref).id);
  }

  /** A drag's new order on a list: the item it moved, placed by its
   *  neighbour (`oxplow.work_item.reorder`). */
  async function reorderList(list: WorkList | null | undefined, orderedRefs: string[]) {
    const before = (list?.all ?? []).map((i) => i.ref).filter((ref) => orderedRefs.includes(ref));
    await runTaskWrite(() => reorderWorkItems(before, orderedRefs));
  }

  async function handleReorderTasks(orderedRefs: string[]) {
    if (!selectedThread) return;
    await reorderList(threadWorkStates[selectedThread.id], orderedRefs);
  }

  async function handleMoveItemToBacklog(ref: string) {
    await runTaskWrite(() => moveWorkItem(ref, null));
  }

  async function handleReorderBacklog(orderedRefs: string[]) {
    await reorderList(backlogState, orderedRefs);
  }

  const currentSession = useMemo(
    () => (stream ? getFileSession(stream.id) : createEmptyFileSession()),
    [fileSessions, stream],
  );
  const selectedFilePath = currentSession.selectedPath;
  const currentFile = selectedFilePath ? currentSession.files[selectedFilePath] ?? null : null;
  const currentFileDirty = !!currentFile && currentFile.draftContent !== currentFile.savedContent;
  const currentThreadState = useMemo(
    () => (stream ? threadStates[stream.id] ?? { selectedThreadId: null, activeThreadId: null, threads: [] } : { selectedThreadId: null, activeThreadId: null, threads: [] }),
    [threadStates, stream],
  );
  const selectedThread = currentThreadState.threads.find((thread) => thread.id === currentThreadState.selectedThreadId) ?? null;
  // The selected thread's agent session: the first one open.
  const selectedThreadSessions = useThreadSessions(selectedThread?.id ?? null);
  const selectedSession = selectedThreadSessions?.[0] ?? null;
  const selectedThreadId = selectedThread?.id ?? null;
  // Derived from the per-thread map. When no thread is selected, fall back to
  // a sentinel that keeps existing UI selectors happy (they all default to
  // the agent tab eventually).
  const centerActive = selectedThreadId
    ? threadCenterActive[selectedThreadId] ?? readPersistedCenterActive() ?? AGENT_TAB_ID
    : AGENT_TAB_ID;
  const setCenterActive = useCallback(
    (next: string | ((prev: string) => string)) => {
      if (!selectedThreadId) return;
      setThreadCenterActive((prev) => {
        const current = prev[selectedThreadId] ?? readPersistedCenterActive() ?? AGENT_TAB_ID;
        const value = typeof next === "function" ? next(current) : next;
        if (value === current) return prev;
        return { ...prev, [selectedThreadId]: value };
      });
    },
    [selectedThreadId],
  );

  const selectedThreadWork = selectedThread ? threadWorkStates[selectedThread.id] ?? null : null;
  useEffect(() => {
    opErrorsStore.setActiveThread(selectedThreadId);
  }, [selectedThreadId, opErrorsStore]);
  useEffect(() => {
    opErrorsStore.setActiveStream(stream?.id ?? null);
  }, [stream?.id, opErrorsStore]);

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const threads = threadStates[s.id]?.threads ?? [];
      const anyWorking = threads.some((t) => agentStatuses[t.id] === "working");
      // Working (busy) outranks awaiting (needs you) outranks waiting so
      // a stream whose thread parked on your answer shows the blue dot
      // even from a collapsed rail.
      const anyAwaiting = threads.some((t) => agentStatuses[t.id] === "awaiting");
      out[s.id] = anyWorking ? "working" : anyAwaiting ? "awaiting" : "waiting";
    }
    return out;
  }, [streams, threadStates, agentStatuses]);
  const streamActiveThreadIds = useMemo<Record<string, string | null>>(() => {
    const out: Record<string, string | null> = {};
    for (const s of streams) out[s.id] = threadStates[s.id]?.activeThreadId ?? null;
    return out;
  }, [streams, threadStates]);

  const currentFileRef = useRef(currentFile);
  currentFileRef.current = currentFile;

  useEffect(() => {
    setExternalFilePrompt(null);
  }, [stream?.id, selectedFilePath]);

  // Persist the list of open file paths per stream on every session change.
  // We write the keys of openOrder only — dirty state, draft content, and
  // scroll position are intentionally dropped.
  useEffect(() => {
    writePersistedFileSessionPaths(fileSessions);
  }, [fileSessions]);

  // Persist the active center tab id so the user lands on the same tab next
  // restart. Diff tabs don't persist (their id includes ephemeral data), so
  // restoration validates the id against available tabs and falls back.
  useEffect(() => {
    writePersistedCenterActive(centerActive);
  }, [centerActive]);

  // After the first stream has had its file sessions rebuilt, verify the
  // initial (localStorage-seeded) centerActive is still resolvable. If it
  // points to a file that didn't come back, a diff tab (which never
  // persist), or a page tab that wasn't restored, snap back to the agent tab.
  // Runs once per mount — subsequent stream switches have their own
  // centerActive logic in handleSwitch. The page-tab case matters
  // because the user's first click after startup often opens a page tab
  // (tasks, plan-work, git-history, …); the previous fall-through
  // reset for unknown id shapes would clobber that click and snap focus
  // back to the agent. Now we trust `effectiveCenterActive`'s fallback
  // gate by checking membership in the same available set.
  useEffect(() => {
    if (centerActiveValidatedRef.current) return;
    if (!stream) return;
    if (!restoredStreamsRef.current.has(stream.id)) return;
    centerActiveValidatedRef.current = true;
    if (centerActive === AGENT_TAB_ID) return;
    const session = fileSessions[stream.id];
    const activeFile = diskFilePath(centerActive);
    if (activeFile !== null) {
      if (!session || !session.files[activeFile]) setCenterActive(AGENT_TAB_ID);
      return;
    }
    if (pageKindOf(centerActive) === "diff") {
      if (!diffTabs.some((tab) => tab.id === centerActive)) setCenterActive(AGENT_TAB_ID);
      return;
    }
    // Page tabs (tasks, plan-work, git-history, …) — validate
    // against the per-thread page-tab list. Reset to agent only when
    // the page wasn't restored. The previous unknown-id fall-through
    // unconditionally reset every page id, clobbering the user's
    // first click after startup.
    const pageTabs = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
    if (!pageTabs.some((ref) => ref.id === centerActive)) setCenterActive(AGENT_TAB_ID);
  }, [stream, fileSessions, centerActive, diffTabs, selectedThreadId, threadPageTabs]);

  // Restore previously-open file tabs the first time each stream becomes
  // active. We add the paths to the session in openOrder, mark each as
  // loading, then fetch content individually. Using the session helpers
  // directly (not handleOpenFile) avoids clobbering centerActive during
  // restore so the saved centerActive remains in effect.
  useEffect(() => {
    if (!stream) return;
    if (restoredStreamsRef.current.has(stream.id)) return;
    restoredStreamsRef.current.add(stream.id);
    const persisted = readPersistedFileSessionPaths();
    const paths = persisted[stream.id];
    if (!paths || paths.length === 0) return;
    const streamId = stream.id;
    // Seed the session with placeholder loading entries so the tabs render
    // immediately.
    mutateFileSession(streamId, (initial) => {
      let base = initial;
      for (const path of paths) {
        if (base.files[path]) continue;
        base = setOpenFileLoading(openFileInSession(base, path, "", true), path, true);
      }
      // Drop the selection that openFileInSession implicitly set — we want
      // the persisted centerActive, not the last restored file, to decide.
      base = { ...base, selectedPath: null };
      return enforceOpenFileLimit(base, MAX_OPEN_FILE_TABS);
    });
    // Fire content fetches in parallel.
    for (const path of paths) {
      void (async () => {
        try {
          const file = await readWorkspaceFile(streamId, path);
          mutateFileSession(streamId, (s) => setLoadedFileContent(s, file.path, file.content));
        } catch (err) {
          logUi("warn", "failed to restore open file tab", { streamId, path, error: String(err) });
          mutateFileSession(streamId, (s) => closeOpenFile(s, path));
        }
      })();
    }
    // Intentionally only depends on stream — we gate re-runs via the ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stream?.id]);

  useEffect(() => {
    if (!stream || !selectedThread || threadWorkStates[selectedThread.id]) return;
    void readWorkList(selectedThread.id)
      .then((next) => {
        setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      })
      .catch((e) => {
        setError(String(e));
      });
  }, [threadWorkStates, selectedThread, stream]);

  useEffect(() => {
    if (!stream) return;
    const missing = currentThreadState.threads.filter((thread) => !threadWorkStates[thread.id]);
    if (missing.length === 0) return;
    let cancelled = false;
    void Promise.all(
      missing.map(async (thread) => [thread.id, await readWorkList(thread.id)] as const),
    )
      .then((results) => {
        if (cancelled) return;
        setThreadWorkStates((prev) => {
          const next = { ...prev };
          for (const [threadId, work] of results) next[threadId] = work;
          return next;
        });
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [threadWorkStates, currentThreadState.threads, stream]);


  // Backend-event subscription wiring (workspace context, backlog, task
  // events, followup/thread/stream changes, config, agent status) lives
  // in this hook so App doesn't carry ~10 inline subscription effects.
  // Extensions' ref kinds (P8.D7): icons, labels, routes and wikilinks.
  useRefKindsLoader();
  useBackendSubscriptions({
    threadWorkStatesRef,
    threadStatesRef,
    setWorkspaceContext,
    setBacklogState,
    setThreadWorkStates,
    setThreadStates,
    setStreams,
    setStream,
    setAgentStatuses,
    setAgentQuestions,
    setGeneratedState,
    setEnabledAgents,
  });

  useEffect(() => {
    for (const [streamId, state] of Object.entries(threadStates)) {
      for (const thread of state.threads) {
        if (threadWorkStates[thread.id]) continue;
        void readWorkList(thread.id)
          .then((work) => setThreadWorkStates((prev) => (prev[thread.id] ? prev : { ...prev, [thread.id]: work })))
          .catch((error) => logUi("warn", "failed to preload thread work state", { streamId, threadId: thread.id, error: String(error) }));
      }
    }
  }, [threadStates]);

  const handleToggleGenerated = async (entry: string, mark: boolean) => {
    // The "mark as generated" toggle edits the `exclude` list (extra
    // ignores beyond .gitignore); `include` is left untouched.
    const exclude = mark
      ? Array.from(new Set([...generated.exclude, entry])).sort()
      : generated.exclude.filter((e) => e !== entry);
    try {
      const cfg = await setGenerated({ exclude, include: generated.include });
      setGeneratedState(generatedPaths(cfg));
    } catch (err) {
      setError(`Failed to update generated paths: ${String(err)}`);
    }
  };

  useEffect(() => {
    if (!stream || !selectedFilePath) return;
    let cancelled = false;
    let refreshTimer: number | null = null;
    let requestId = 0;

    const refreshSelectedFile = async () => {
      const currentRequestId = ++requestId;
      try {
        const file = await readWorkspaceFile(stream.id, selectedFilePath);
        if (cancelled || currentRequestId !== requestId || file.path !== selectedFilePath) return;
        const openFile = currentFileRef.current;
        switch (externalFileSyncAction(openFile, file.content)) {
          case "noop":
            return;
          case "update-saved":
            mutateFileSession(stream.id, (s) => setLoadedFileContent(s, file.path, file.content));
            return;
          case "replace-draft":
            mutateFileSession(stream.id, (s) => markFileSaved(s, file.path, file.content));
            setExternalFilePrompt((current) => (current?.path === file.path ? null : current));
            return;
          case "prompt":
            setExternalFilePrompt({ path: file.path, content: file.content });
            return;
        }
      } catch (e) {
        if (cancelled) return;
        setError(String(e));
        logUi("error", "failed to refresh file after filesystem change", {
          streamId: stream.id,
          path: selectedFilePath,
          error: String(e),
        });
      }
    };

    const unsubscribe = subscribeWorkspaceEvents(stream.id, (event) => {
      if (event.path !== selectedFilePath || event.kind === "deleted") return;
      if (refreshTimer) window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        void refreshSelectedFile();
      }, 75);
    });

    return () => {
      cancelled = true;
      unsubscribe();
      if (refreshTimer) window.clearTimeout(refreshTimer);
    };
  }, [selectedFilePath, stream]);
  // Imperative shortcut for opening the New-Task modal. When PlanPane is
  // mounted it registers its openCreateModal here; the menu handler can
  // call this ref directly instead of going through setState + useEffect.
  // Needed because menu clicks arrive as IPC messages (not "discrete user
  // input events"), so React doesn't auto-flush effects for them — the
  // useEffect chain can stall for 10+ seconds before committing. Direct
  // ref call inside flushSync sidesteps the scheduler entirely.
  const planOpenCreateRef = useRef<(() => void) | null>(null);
  // Forward ref so the forms and offers (declared above handleOpenPage) can
  // route through the same page-tab opener used by every other caller.
  // The ref is populated in a useEffect after handleOpenPage is defined.
  const handleOpenPageRef = useRef<((ref: TabRef) => void) | null>(null);
  const commandState = useMemo(
    () => ({
      hasStream: !!stream,
      hasSelectedFile: !!selectedFilePath,
      canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
      hasThread: !!selectedThread,
      canCommit: !!stream && !!workspaceContext.vcsEnabled,
    } as const),
    [currentFile, currentFileDirty, selectedThread, selectedFilePath, stream, workspaceContext.vcsEnabled],
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
      case "commit":
        if (!stream || !workspaceContext.vcsEnabled) return;
        handleOpenPageRef.current?.(indexRef("files"));
        setCommitFilesRequest((n) => n + 1);
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
    () => ({ streamId: stream?.id ?? null, threadId: selectedThreadId ?? null }),
    [stream?.id, selectedThreadId],
  );
  const offerDeps = useMemo<OfferDeps>(
    () => ({
      openPage: (tabId) => {
        const ref = refFromTabId(tabId);
        if (ref) handleOpenPageRef.current?.(ref);
      },
      openForm,
      // What the window has for one of its own to act on now.
      available: (spec) => {
        if (spec.ui?.form === "new-thread") return commandState.hasStream;
        if (spec.ui?.form === "commit") return !!commandState.canCommit;
        switch (spec.op ? `${spec.op.capability}/${spec.op.op}` : "") {
          case "editor.write/save":
            return commandState.canSave;
          case "window.show/find":
            return commandState.hasSelectedFile;
          case "window.show/quick_open":
            return commandState.hasStream;
          case "agent_input.write/draft":
            return commandState.hasThread;
          case "projects.write/create":
          case "projects.write/open":
            // The shell's: a browser window has none to ask.
            return shellAvailable();
          default:
            return true;
        }
      },
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
      }),
    [personSpecs, runGitMenuOp, openForm, commandState],
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
      if (!offer || offer.enabled === false) return;
      event.preventDefault();
      offer.run();
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
  const availableCenterIds = useMemo(() => {
    const ids = new Set([AGENT_TAB_ID]);
    for (const path of currentSession.openOrder) ids.add(fileRef(path).id);
    for (const tab of diffTabs) ids.add(tab.id);
    for (const ref of pageTabsForActiveThread) ids.add(ref.id);
    return ids;
  }, [currentSession.openOrder, diffTabs, pageTabsForActiveThread]);
  const effectiveCenterActive = availableCenterIds.has(centerActive) ? centerActive : AGENT_TAB_ID;

  // Feed the stall watchdog (logger.ts) the coarse "what's on screen"
  // context so a `main thread stalled` WARN names the active page + the
  // open file, not just a duration.
  useEffect(() => {
    setUiLogContext({
      streamId: stream?.id ?? null,
      centerActive: effectiveCenterActive,
      filePath: selectedFilePath ?? null,
    });
  }, [stream?.id, effectiveCenterActive, selectedFilePath]);

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
    openOrder: string[];
    streamId: string | null;
  }>({ pageTabs: [], openOrder: [], streamId: null });
  visitContextRef.current = {
    pageTabs: pageTabsForActiveThread,
    openOrder: currentSession.openOrder,
    streamId: stream?.id ?? null,
  };
  const hydratedHistoryRef = useRef(false);
  useEffect(() => {
    if (!hydratedHistoryRef.current) {
      hydratedHistoryRef.current = true;
      return;
    }
    if (!selectedThreadId) return;
    const ctx = visitContextRef.current;
    const ref = resolveActiveTabRef(effectiveCenterActive, ctx.pageTabs, ctx.openOrder);
    if (!ref || NON_TRACKED_KINDS.has(ref.kind)) return;
    // Defer briefly so the freshly-activated page's `usePageTitle`
    // effect fires and the centerTabs memo applies it before we
    // snapshot the label. tabLabelByIdRef is what the tab strip
    // currently displays — reading from it keeps the two surfaces
    // consistent; there is no separate fallback derivation.
    const timer = setTimeout(() => {
      const ctxAfter = visitContextRef.current;
      const label = tabLabelByIdRef.current[effectiveCenterActive] ?? ref.id;
      void recordPageVisit({
        refKind: ref.kind,
        refId: ref.id,
        payload: ref.payload,
        label,
        streamId: ctxAfter.streamId,
        threadId: selectedThreadId,
      });
    }, 250);
    return () => clearTimeout(timer);
  }, [effectiveCenterActive, selectedThreadId]);

  // Report the page the human has open (plus its published detail, e.g. a
  // lens's params) so an agent's `get_open_page` sees what they see.
  // Best-effort and debounced; see tabs/openPageDetail.ts.
  const [pageDetailTick, setPageDetailTick] = useState(0);
  useEffect(() => getPageDetailStore().subscribe(() => setPageDetailTick((t) => t + 1)), []);
  useEffect(() => {
    if (!selectedThreadId) return;
    const timer = setTimeout(() => {
      const ctx = visitContextRef.current;
      const ref = resolveActiveTabRef(effectiveCenterActive, ctx.pageTabs, ctx.openOrder);
      const pageId = ref?.id ?? effectiveCenterActive;
      const kind = ref?.kind ?? effectiveCenterActive;
      const detail = getPageDetailStore().get(pageId);
      void reportOpenPage(selectedThreadId, pageId, kind, detail ? JSON.stringify(detail) : null).catch(() => {});
    }, 300);
    return () => clearTimeout(timer);
  }, [effectiveCenterActive, selectedThreadId, pageDetailTick]);

  const handleOpenDiff = (request: DiffSpec) => {
    const id = computeDiffId(request);
    // Always (re)write the spec so per-click metadata that doesn't
    // affect the id (e.g. revealLine pointing at a specific function)
    // is honored on subsequent clicks of the same diff.
    setDiffTabs((prev) => {
      if (prev.some((tab) => tab.id === id)) {
        return prev.map((tab) => (tab.id === id ? { id, spec: request } : tab));
      }
      return [...prev, { id, spec: request }];
    });
    // Diff tabs live in threadPageTabs as the primary track now —
    // they participate in per-tab back/forward and share the same
    // chrome as every other page kind. `diffTabs` is only a spec
    // registry indexed by id.
    if (selectedThreadId) {
      const ref: TabRef = {
        id,
        kind: "diff",
        payload: {
          path: request.path,
          leftVersion: request.leftVersion,
          rightVersion: request.rightVersion,
          labelOverride: request.labelOverride ?? null,
        },
      };
      setThreadPageTabs((prev) => {
        const existing = prev[selectedThreadId] ?? [];
        if (existing.some((t) => t.id === id)) return prev;
        return { ...prev, [selectedThreadId]: [...existing, ref] };
      });
    }
    setCenterActive(id);
  };

  const handleCompareWithClipboard = async (selection: string, path: string) => {
    let clipboard = "";
    try {
      clipboard = await navigator.clipboard.readText();
    } catch (err) {
      setError(`Clipboard read failed: ${String(err)}`);
      return;
    }
    // Use a deterministic id with a timestamp so each compare-with-
    // clipboard session is its own tab; route through handleOpenDiff
    // so the spec lands in both the diffTabs registry and the
    // unified page-tab list.
    const ts = Date.now();
    const spec: DiffSpec = {
      path,
      // The compare-with-clipboard view never reads either side from
      // disk/git — the inline `leftContent` / `rightContent` literals
      // bypass the version dispatcher entirely. We still need to
      // satisfy the `Revision` shape; WORKING is a harmless sentinel
      // here.
      leftVersion: WORKING,
      rightVersion: WORKING,
      baseLabel: "clipboard",
      leftContent: selection,
      rightContent: clipboard,
      labelOverride: `selection vs clipboard (${ts})`,
    };
    handleOpenDiff(spec);
  };

  const handleRevealCommit = (sha: string) => {
    handleOpenPage(gitCommitRef(sha));
  };

  const closeDiffTab = (id: string) => {
    setDiffTabs((prev) => prev.filter((tab) => tab.id !== id));
    // Diffs live in threadPageTabs now — close from the unified list
    // too. closePageTab handles centerActive snap-back.
    closePageTab(id);
  };


  const handleOpenWiki = useCallback((slug: string) => {
    const tid = selectedThread?.id ?? null;
    if (!tid) return;
    const ref = wikiPageRef(slug);
    setThreadPageTabs((prev) => {
      const existing = prev[tid] ?? [];
      if (existing.some((t) => t.id === ref.id)) return prev;
      return { ...prev, [tid]: [...existing, ref] };
    });
    setCenterActive(ref.id);
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
  }, [stream?.id, selectedThread?.id]);

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

  /** Open the GitCommitPage for a wikilink-resolved commit SHA. */
  const handleOpenCommit = useCallback((sha: string) => {
    if (!sha) return;
    handleOpenPageRef.current?.(gitCommitRef(sha));
  }, []);

  /** Open the DirectoryPage for a wikilink-resolved workspace dir. */
  const handleOpenDirectory = useCallback((path: string) => {
    if (!path) return;
    handleOpenPageRef.current?.(directoryRef(path));
  }, []);

  const handleReorderCenterTabs = useCallback((orderedIds: string[]) => {
    if (!stream) return;
    const orderedFiles: string[] = [];
    const orderedDiffIds: string[] = [];
    for (const id of orderedIds) {
      const path = diskFilePath(id);
      if (path !== null) orderedFiles.push(path);
      else if (pageKindOf(id) === "diff") orderedDiffIds.push(id);
    }
    mutateFileSession(stream.id, (base) => reorderOpenFiles(base, orderedFiles));
    setDiffTabs((prev) => {
      if (orderedDiffIds.length !== prev.length) return prev;
      const byId = new Map(prev.map((d) => [d.id, d] as const));
      const next = orderedDiffIds.map((id) => byId.get(id)).filter((d): d is { id: string; spec: DiffSpec } => !!d);
      if (next.length !== prev.length) return prev;
      return next;
    });
    // `threadPageTabs` is the unified source of truth for the order of
    // EVERY non-agent tab (files, diffs, dashboards, snapshots, wiki,
    // tasks, …) — the strip renders `[agent, ...threadPageTabs]`. So
    // reorder it with the full ordered id list (minus the pinned agent),
    // not just the non-file/diff subset; splitting file/diff out here
    // would shove them to the end on every drag/promote. The
    // fileSessions/diffTabs reorders above are just bookkeeping for
    // their own registries.
    if (selectedThread?.id) {
      const threadId = selectedThread.id;
      const orderedNonAgent = orderedIds.filter((id) => id !== AGENT_TAB_ID);
      setThreadPageTabs((prev) => {
        const current = prev[threadId] ?? [];
        if (current.length === 0) return prev;
        const byId = new Map(current.map((ref) => [ref.id, ref] as const));
        const next: TabRef[] = [];
        for (const id of orderedNonAgent) {
          const ref = byId.get(id);
          if (ref) next.push(ref);
        }
        // Append any current refs missing from the ordered list so we
        // never drop a tab (defense in depth — the caller passes the
        // full id list, but hidden back-stack entries aren't in it).
        for (const ref of current) {
          if (!next.find((r) => r.id === ref.id)) next.push(ref);
        }
        if (next.every((r, idx) => r.id === current[idx]?.id)) return prev;
        return { ...prev, [threadId]: next };
      });
    }
  }, [stream, selectedThread?.id]);

  const agentThreadStatus: AgentStatus = selectedThread ? agentStatuses[selectedThread.id] ?? "waiting" : "waiting";

  const bookmarks = useBookmarks(selectedThreadId, stream?.id ?? null);

  const handleOpenPage = useCallback((ref: TabRef) => {
    // Page-visit recording lives in the central activation effect
    // below — it fires whenever `effectiveCenterActive` resolves to a
    // new TabRef, regardless of which handler caused the activation.
    switch (ref.kind) {
      case "agent":
        setCenterActive(AGENT_TAB_ID);
        return;
      case "symbol":
        void openSymbol((ref.payload as { ref: string }).ref);
        return;
      case "file": {
        const payload = ref.payload as {
          path?: string;
          version?: import("./revision.js").Revision;
          /** Open at this line (lens `file` links with `line:`). */
          line?: number;
        } | null;
        if (!payload?.path) return;
        const version = payload.version ?? WORKING;
        if (version === WORKING) {
          if (payload.line && payload.line > 0) {
            void handleNavigateToLocation({ path: payload.path, line: payload.line, column: 1 });
          } else {
            void handleOpenFile(payload.path);
          }
          return;
        }
        // Non-disk: register the ref directly without going through
        // the disk-only fileSessions cache. The render branch picks
        // FileViewerPage based on the payload version.
        if (selectedThreadId) {
          setThreadPageTabs((prev) => {
            const existing = prev[selectedThreadId] ?? [];
            if (existing.some((t) => t.id === ref.id)) return prev;
            return { ...prev, [selectedThreadId]: [...existing, ref] };
          });
          setCenterActive(ref.id);
        }
        return;
      }
      case "diff": {
        // A diff built from its spec (e.g. a lens `diff-at` link).
        const spec = ref.payload as DiffSpec | null;
        if (spec?.path && spec.leftVersion && spec.rightVersion) handleOpenDiff(spec);
        return;
      }
      default: {
        // Every other kind opens as a per-thread page tab.
        if (selectedThreadId) {
          setThreadPageTabs((prev) => {
            const existing = prev[selectedThreadId] ?? [];
            if (existing.some((t) => t.id === ref.id)) return prev;
            return { ...prev, [selectedThreadId]: [...existing, ref] };
          });
          setCenterActive(ref.id);
        }
        return;
      }
    }
    // handleOpenDiff is a plain function over state setters and
    // selectedThreadId, which is already a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handleOpenFile, handleOpenWiki, selectedThreadId, selectedThreadWork, setCenterActive, stream?.id]);

  /** Navigate to a unified-search hit. Shared by the search palette and
   *  the quick-open overlay's body-hit rows. */
  const openSearchHit = useCallback((hit: import("./api.js").SearchHit) => {
    const target = searchHitTarget(hit);
    if (!target) return;
    if ("file" in target) void handleOpenFile(target.file);
    else handleOpenPage(target.page);
  }, [handleOpenPage, handleOpenFile]);

  /**
   * Browser-style in-tab navigation. Replaces the page tab whose
   * current id is `currentTabId` with `ref`, and pushes the prior ref
   * onto that tab's back stack (carrying any existing back/forward
   * with it). The tab's id changes to `ref.id`; centerActive follows.
   * If `currentTabId` doesn't refer to a known page tab, falls back to
   * `handleOpenPage` (open in new tab).
   */
  const handleNavigateInTab = useCallback((
    currentTabId: string,
    ref: TabRef,
    siblings?: import("./tabs/PageNavigationContext.js").NavSiblings,
  ) => {
    if (!selectedThreadId) return;
    // A symbol isn't a page: it opens its file at its line.
    if (ref.kind === "symbol") {
      handleOpenPage(ref);
      return;
    }
    const existing = threadPageTabs[selectedThreadId] ?? [];
    const idx = existing.findIndex((t) => t.id === currentTabId);
    if (idx < 0) {
      handleOpenPage(ref);
      return;
    }
    if (existing[idx]!.id === ref.id) return;
    // agent still lives in its own slot — promote to a regular
    // open. Files and diffs DO support in-tab nav: the page tab
    // becomes a file viewer / diff viewer (back returns to the
    // prior page). Diffs require their spec to be pre-registered
    // in `diffTabs`; the helper that initiates the navigation
    // (handleOpenDiffInTab) is responsible for that.
    if (ref.kind === "agent") {
      handleOpenPage(ref);
      return;
    }
    if (ref.kind === "file") {
      const payload = ref.payload as {
        path?: string;
        version?: import("./revision.js").Revision;
        line?: number;
      } | null;
      // Only the disk version needs to populate the fileSessions
      // dirty-state cache + LSP wiring. Non-disk versions render
      // through FileViewerPage which loads content directly via
      // readAt(version) — calling handleOpenFile here would
      // pollute the cache with disk content the user didn't ask
      // for.
      const isDisk = !payload?.version || payload.version === WORKING;
      if (payload?.path && isDisk) void handleOpenFile(payload.path);
      // A line on the ref (lens `file` links with `line:`) reveals it.
      if (payload?.path && isDisk && payload.line && payload.line > 0) {
        setEditorNavigationTarget({ path: payload.path, line: payload.line, column: 1 });
      }
    }
    if (ref.kind === "diff") {
      // A diff ref that carries its whole spec (lens `diff-at` links)
      // registers it here, since no handleOpenDiffInTab ran first.
      const spec = ref.payload as DiffSpec | null;
      if (spec?.path && spec.leftVersion && spec.rightVersion) {
        setDiffTabs((prev) =>
          prev.some((t) => t.id === ref.id)
            ? prev.map((t) => (t.id === ref.id ? { id: ref.id, spec } : t))
            : [...prev, { id: ref.id, spec }],
        );
      }
    }
    const oldRef = existing[idx]!;
    setThreadPageTabs((prev) => {
      const list = prev[selectedThreadId] ?? [];
      const next = list.slice();
      // If `ref` already exists elsewhere as a page tab, drop the
      // duplicate to mirror browser dedup-on-navigate.
      const dupIdx = next.findIndex((t, i) => i !== idx && t.id === ref.id);
      if (dupIdx >= 0) next.splice(dupIdx, 1);
      const targetIdx = dupIdx >= 0 && dupIdx < idx ? idx - 1 : idx;
      next[targetIdx] = ref;
      return { ...prev, [selectedThreadId]: next };
    });
    setThreadPageHistory((prev) => {
      const perThread = prev[selectedThreadId] ?? {};
      const old = perThread[currentTabId] ?? { back: [], forward: [], siblings: null };
      const { [currentTabId]: _drop, ...rest } = perThread;
      // Snap the siblings index to whatever entry matches `ref.id` in
      // case the caller passed an out-of-date index.
      const resolvedSiblings = resolveSiblings(siblings, ref);
      return {
        ...prev,
        [selectedThreadId]: {
          ...rest,
          [ref.id]: {
            // Capture the prior page's ref AND its siblings so going
            // back later restores the originating list's prev/next
            // chain instead of dropping it.
            back: [...old.back, { ref: oldRef, siblings: old.siblings }],
            forward: [],
            siblings: resolvedSiblings,
          },
        },
      };
    });
    setCenterActive(ref.id);
  }, [handleOpenPage, selectedThreadId, setCenterActive, threadPageTabs]);

  /**
   * Open a diff *in-tab* — replaces the active page tab's ref with
   * the diff ref, with the diff's spec registered in `diffTabs` so
   * the page-tab renderer can find it. Pushes the prior ref onto the
   * back stack so Back returns to the originating page (e.g. the
   * change-analysis dashboard). Caller passes the active tab id so
   * we know which slot to mutate.
   */
  const handleOpenDiffInTab = useCallback((
    currentTabId: string,
    spec: DiffSpec,
    siblings?: import("./tabs/PageNavigationContext.js").NavSiblings,
  ) => {
    const id = computeDiffId(spec);
    // Always rewrite the spec so per-click metadata that doesn't
    // affect the id (e.g. revealLine pointing at a function's start
    // line) is honored on subsequent clicks of the same diff.
    //
    // When siblings carry diffSpecs, pre-register every sibling's
    // spec too — otherwise stepping the prev/next arrow lands on a
    // diff TabRef whose spec the renderer can't find and the tab
    // silently disappears.
    setDiffTabs((prev) => {
      const byId = new Map(prev.map((t) => [t.id, t.spec]));
      byId.set(id, spec);
      if (siblings) {
        for (const e of siblings.entries) {
          if (e.diffSpec) byId.set(computeDiffId(e.diffSpec), e.diffSpec);
        }
      }
      return [...byId.entries()].map(([tid, tspec]) => ({ id: tid, spec: tspec }));
    });
    const ref: TabRef = {
      id,
      kind: "diff",
      payload: {
        path: spec.path,
        leftVersion: spec.leftVersion,
        rightVersion: spec.rightVersion,
        labelOverride: spec.labelOverride ?? null,
      },
    };
    handleNavigateInTab(currentTabId, ref, siblings);
  }, [handleNavigateInTab]);

  /**
   * Step the active tab to a sibling at `targetIdx` without touching
   * back/forward. The tab id changes to the new ref's id and the
   * siblings record migrates with it.
   */
  const handleStepSibling = useCallback((currentTabId: string, targetIdx: number) => {
    if (!selectedThreadId) return;
    const perThread = threadPageHistory[selectedThreadId] ?? {};
    const entry = perThread[currentTabId];
    if (!entry || !entry.siblings) return;
    if (targetIdx < 0 || targetIdx >= entry.siblings.entries.length) return;
    const target = entry.siblings.entries[targetIdx]!.ref;
    const existing = threadPageTabs[selectedThreadId] ?? [];
    const idx = existing.findIndex((t) => t.id === currentTabId);
    if (idx < 0) return;
    if (target.id === currentTabId) return;
    if (target.kind === "file") {
      const payload = target.payload as { path?: string; version?: import("./revision.js").Revision } | null;
      const isDisk = !payload?.version || payload.version === WORKING;
      if (payload?.path && isDisk) void handleOpenFile(payload.path);
    }
    setThreadPageTabs((prev) => {
      const list = prev[selectedThreadId] ?? [];
      const next = list.slice();
      const dupIdx = next.findIndex((t, i) => i !== idx && t.id === target.id);
      if (dupIdx >= 0) next.splice(dupIdx, 1);
      const adjustedIdx = dupIdx >= 0 && dupIdx < idx ? idx - 1 : idx;
      next[adjustedIdx] = target;
      return { ...prev, [selectedThreadId]: next };
    });
    setThreadPageHistory((prev) => {
      const perThread = prev[selectedThreadId] ?? {};
      const old = perThread[currentTabId];
      if (!old) return prev;
      const { [currentTabId]: _drop, ...rest } = perThread;
      return {
        ...prev,
        [selectedThreadId]: {
          ...rest,
          [target.id]: {
            // Preserve back/forward — sibling navigation is orthogonal.
            back: old.back,
            forward: old.forward,
            siblings: old.siblings ? { entries: old.siblings.entries, index: targetIdx, title: old.siblings.title } : null,
          },
        },
      };
    });
    setCenterActive(target.id);
  }, [handleOpenFile, selectedThreadId, setCenterActive, threadPageHistory, threadPageTabs]);

  const handleGoBack = useCallback((currentTabId: string) => {
    if (!selectedThreadId) return;
    const perThread = threadPageHistory[selectedThreadId] ?? {};
    const entry = perThread[currentTabId];
    if (!entry || entry.back.length === 0) return;
    const targetFrame = entry.back[entry.back.length - 1]!;
    const target = targetFrame.ref;
    const existing = threadPageTabs[selectedThreadId] ?? [];
    const idx = existing.findIndex((t) => t.id === currentTabId);
    if (idx < 0) return;
    const oldRef = existing[idx]!;
    setThreadPageTabs((prev) => {
      const list = prev[selectedThreadId] ?? [];
      const next = list.slice();
      next[idx] = target;
      return { ...prev, [selectedThreadId]: next };
    });
    setThreadPageHistory((prev) => {
      const perThread = prev[selectedThreadId] ?? {};
      const { [currentTabId]: drop, ...rest } = perThread;
      return {
        ...prev,
        [selectedThreadId]: {
          ...rest,
          [target.id]: {
            back: entry.back.slice(0, -1),
            // Push the page we're leaving onto the forward stack
            // along with its siblings so a re-forward also restores.
            forward: [...entry.forward, { ref: oldRef, siblings: entry.siblings }],
            // Restore the back-target's original siblings — keep
            // up/down arrows alive on the page we're returning to.
            siblings: targetFrame.siblings,
          },
        },
      };
    });
    setCenterActive(target.id);
  }, [selectedThreadId, setCenterActive, threadPageHistory, threadPageTabs]);

  const handleGoForward = useCallback((currentTabId: string) => {
    if (!selectedThreadId) return;
    const perThread = threadPageHistory[selectedThreadId] ?? {};
    const entry = perThread[currentTabId];
    if (!entry || entry.forward.length === 0) return;
    const targetFrame = entry.forward[entry.forward.length - 1]!;
    const target = targetFrame.ref;
    const existing = threadPageTabs[selectedThreadId] ?? [];
    const idx = existing.findIndex((t) => t.id === currentTabId);
    if (idx < 0) return;
    const oldRef = existing[idx]!;
    setThreadPageTabs((prev) => {
      const list = prev[selectedThreadId] ?? [];
      const next = list.slice();
      next[idx] = target;
      return { ...prev, [selectedThreadId]: next };
    });
    setThreadPageHistory((prev) => {
      const perThread = prev[selectedThreadId] ?? {};
      const { [currentTabId]: drop, ...rest } = perThread;
      return {
        ...prev,
        [selectedThreadId]: {
          ...rest,
          [target.id]: {
            back: [...entry.back, { ref: oldRef, siblings: entry.siblings }],
            forward: entry.forward.slice(0, -1),
            siblings: targetFrame.siblings,
          },
        },
      };
    });
    setCenterActive(target.id);
  }, [selectedThreadId, setCenterActive, threadPageHistory, threadPageTabs]);

  const closePageTab = useCallback((id: string) => {
    if (!selectedThreadId) return;
    // File tabs live in fileSessions for content + dirty state; close
    // their content cache too when the tab closes from the unified
    // list so we don't leak buffers. Stream-scoped because the
    // session map is keyed by stream.
    const path = diskFilePath(id);
    if (path !== null && stream) {
      setFileSessions((prev) => {
        const session = prev[stream.id];
        if (!session || !session.files[path]) return prev;
        return { ...prev, [stream.id]: closeOpenFile(session, path) };
      });
    }
    setThreadPageTabs((prev) => {
      const existing = prev[selectedThreadId] ?? [];
      if (!existing.some((t) => t.id === id)) return prev;
      return { ...prev, [selectedThreadId]: existing.filter((t) => t.id !== id) };
    });
    setThreadPageHistory((prev) => {
      const perThread = prev[selectedThreadId] ?? {};
      if (!(id in perThread)) return prev;
      const { [id]: _drop, ...rest } = perThread;
      return { ...prev, [selectedThreadId]: rest };
    });
    setPageTitles((prev) => {
      if (!(id in prev)) return prev;
      const { [id]: _drop, ...rest } = prev;
      return rest;
    });
    setThreadPageMru((prev) => {
      const cur = prev[selectedThreadId] ?? [];
      const next = dropFromMru(cur, id);
      return next === cur ? prev : { ...prev, [selectedThreadId]: next };
    });
    setCenterActive((current) => (current === id ? AGENT_TAB_ID : current));
    // GC the per-page snapshot so closed tabs don't leak forever.
    if (selectedThreadId) {
      const pageKey = `${selectedThreadId}::${id}`;
      clearPageSnapshot(pageKey);
    }
  }, [selectedThreadId, setCenterActive, setThreadPageMru, stream]);

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
      const id = (input as { ref?: unknown } | null)?.ref;
      if (typeof id !== "string") throw new Error("`ref` is a page's ref (`file:src/a.rs`)");
      const ref = refFromTabId(id);
      if (!ref) throw new Error(`\`${id}\` isn't a page's ref`);
      return ref;
    };
    const open = async (input: unknown, ctx: ClientCallContext, focus: boolean) => {
      const thread = threadOf(ctx);
      const ref = refOf(input);
      if (thread === selectedThreadId && focus) {
        handleOpenPageRef.current?.(ref);
        return { ref: ref.id, focused: true };
      }
      // A working-tree file's content lives in its stream's session: read
      // it first (a path that isn't a file opens nothing), keeping what
      // the session shows.
      const path = diskFilePath(ref.id);
      const streamId = streamOfThread(threadStates, thread);
      if (path !== null && streamId && !getFileSession(streamId).files[path]) {
        const file = await readWorkspaceFile(streamId, path);
        mutateFileSession(streamId, (base) => {
          const opened = openFileInSession(base, path, "", false);
          return enforceOpenFileLimit({ ...opened, selectedPath: base.selectedPath }, MAX_OPEN_FILE_TABS);
        });
        mutateFileSession(streamId, (s) => setLoadedFileContent(s, path, file.content));
      }
      setThreadPageTabs((prev) => withTab(prev, thread, ref));
      if (focus) setThreadCenterActive((prev) => ({ ...prev, [thread]: ref.id }));
      return { ref: ref.id, focused: focus };
    };
    return {
      "editor.write": {
        // The file `ref` names, else the one the thread shows.
        save: async (input, ctx) => {
          const thread = threadOf(ctx);
          const id = (input as { ref?: unknown } | null)?.ref;
          const shown = thread === selectedThreadId ? centerActive : threadCenterActive[thread];
          const tab = typeof id === "string" ? id : shown;
          const path = tab ? diskFilePath(tab) : null;
          if (path === null) throw new Error(typeof id === "string" ? `\`${id}\` isn't a file in the working tree` : "no file is shown");
          const streamId = streamOfThread(threadStates, thread);
          if (!streamId) throw new Error(`no stream has thread \`${thread}\``);
          await saveFile(streamId, path, ctx.call);
          return { saved: path };
        },
      },
      "window.show": {
        find: () => {
          if (!selectedFilePath) throw new Error("no file is shown");
          setCenterActive(fileRef(selectedFilePath).id);
          setEditorFindRequest((current) => current + 1);
          return null;
        },
        quick_open: () => {
          if (!stream) throw new Error("no stream is shown");
          setQuickOpenVisible(true);
          return null;
        },
      },
      // A person's only (the command's invokers): oxplow never types for
      // the agent. It fills the shown thread's agent input, unsent.
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
            setThreadPageTabs((prev) => withoutTab(prev, thread, ref.id));
            setThreadCenterActive((prev) =>
              prev[thread] === ref.id ? { ...prev, [thread]: AGENT_TAB_ID } : prev,
            );
          }
          return { ref: ref.id, closed: true };
        },
      },
    };
    // getFileSession / mutateFileSession are plain functions over the
    // session state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedThreadId, threadStates, closePageTab, setThreadPageTabs, setThreadCenterActive, centerActive, threadCenterActive, selectedFilePath, stream, setCenterActive]);
  const windowHandlersRef = useRef(windowHandlers);
  windowHandlersRef.current = windowHandlers;
  useEffect(() => startClientHost(() => windowHandlersRef.current), []);

  // Used when the record a tab shows is *deleted* (wiki page / task).
  // Rather than closing the tab outright, navigate it back one entry in
  // its own history; only close when there's nothing to go back to.
  const closeOrGoBackPageTab = useCallback((id: string) => {
    if (!selectedThreadId) { closePageTab(id); return; }
    const perThread = threadPageHistory[selectedThreadId] ?? {};
    const existing = threadPageTabs[selectedThreadId] ?? [];
    const idx = existing.findIndex((t) => t.id === id);
    const plan = planCloseOrGoBack(perThread[id], idx >= 0);
    if (plan.action === "close") {
      closePageTab(id);
      return;
    }
    const target = plan.target;
    setThreadPageTabs((prev) => {
      const list = prev[selectedThreadId] ?? [];
      const i = list.findIndex((t) => t.id === id);
      if (i < 0) return prev;
      const next = list.slice();
      next[i] = target;
      return { ...prev, [selectedThreadId]: next };
    });
    setThreadPageHistory((prev) => {
      const pt = prev[selectedThreadId] ?? {};
      const { [id]: _drop, ...rest } = pt;
      return { ...prev, [selectedThreadId]: { ...rest, [target.id]: plan.nextEntry } };
    });
    setPageTitles((prev) => {
      if (!(id in prev)) return prev;
      const { [id]: _drop, ...rest } = prev;
      return rest;
    });
    setCenterActive((current) => (current === id ? target.id : current));
    const pageKey = `${selectedThreadId}::${id}`;
    clearPageSnapshot(pageKey);
  }, [selectedThreadId, threadPageHistory, threadPageTabs, closePageTab, setCenterActive]);

  // Track tab recency: whatever tab becomes active moves to the front of
  // its thread's MRU list. Driven off `centerActive` (not setCenterActive)
  // so it captures every activation path — explicit clicks, opens, history
  // navigation. (the agent tab lands here too but is never a page tab, so it's
  // harmless filler the eviction pass ignores.)
  useEffect(() => {
    if (!selectedThreadId) return;
    setThreadPageMru((prev) => {
      const cur = prev[selectedThreadId] ?? [];
      const next = touchMru(cur, centerActive);
      return next === cur ? prev : { ...prev, [selectedThreadId]: next };
    });
  }, [centerActive, selectedThreadId, setThreadPageMru]);

  // Enforce the page-tab cap on the active thread: when its open tabs
  // exceed MAX_PAGE_TABS, evict the least-recently-used ones. The active
  // tab and any dirty file tabs are protected (a soft cap never discards
  // unsaved work). Eviction routes through closePageTab — the same cleanup
  // a manual close does (drops file session / history / title / snapshot).
  // Closing shrinks threadPageTabs, re-running this effect until it
  // settles at or below the cap.
  useEffect(() => {
    if (!selectedThreadId) return;
    const tabsList = threadPageTabs[selectedThreadId] ?? [];
    if (tabsList.length <= MAX_PAGE_TABS) return;
    const protect = new Set<string>();
    if (centerActive) protect.add(centerActive);
    const session = stream ? fileSessions[stream.id] : undefined;
    if (session) {
      for (const t of tabsList) {
        const path = diskFilePath(t.id);
        if (path === null) continue;
        const file = session.files[path];
        if (file && file.draftContent !== file.savedContent) protect.add(t.id);
      }
    }
    const mru = threadPageMru[selectedThreadId] ?? [];
    const victims = selectLruEvictions(
      tabsList.map((t) => t.id),
      mru,
      { max: MAX_PAGE_TABS, protect },
    );
    for (const id of victims) closePageTab(id);
  }, [threadPageTabs, selectedThreadId, centerActive, threadPageMru, fileSessions, stream, closePageTab]);

  // Keep the forward ref in sync with the latest handleOpenPage. Used by
  // the forms and offers (declared above handleOpenPage) so menu/keyboard
  // dispatches route through the same page-tab opener.
  useEffect(() => {
    handleOpenPageRef.current = handleOpenPage;
  }, [handleOpenPage]);

  const centerTabs: CenterTab[] = useMemo(() => {
    const tabs: CenterTab[] = [
      {
        id: AGENT_TAB_ID,
        label: selectedSession ? sessionLabel(selectedSession) : "Agent",
        closable: false,
        agentStatus: agentThreadStatus,
        render: () => (
          <AgentPage
            thread={selectedThread}
            session={selectedSession}
            stream={stream}
            visible={effectiveCenterActive === AGENT_TAB_ID}
            onOpenFile={(absPath, line, column) => {
              if (!stream) return;
              // The terminal link provider hands us absolute paths
              // (resolved against stream.worktree_path). The rest of
              // the app's open-file path takes a workspace-relative
              // path, so trim the worktree prefix when present.
              const wt = stream.worktree_path.endsWith("/")
                ? stream.worktree_path.slice(0, -1)
                : stream.worktree_path;
              const rel = absPath.startsWith(wt + "/")
                ? absPath.slice(wt.length + 1)
                : absPath;
              if (typeof line === "number" && line > 0) {
                void handleNavigateToLocation({ path: rel, line, column: column ?? 1 });
              } else {
                void handleOpenFile(rel);
              }
            }}
            onOpenDiff={handleOpenDiff}
            onOpenSettings={() => handleOpenPage(indexRef("settings"))}
            onOpenPage={handleOpenPage}
          />
        ),
      },
    ];
    // File tabs live in `threadPageTabs` like every other page kind;
    // the page-tab loop's `ref.kind === "file"` branch handles the
    // render. fileSessions owns the file content + dirty state, but
    // tab membership and order are driven by the unified list.
    // The unified-chrome wrap loop below applies to every tab pushed
    // after this index — every per-thread page tab (notes, files,
    // diffs, tasks, etc.). Only the agent at index 0 is excluded.
    // Diffs live in `threadPageTabs` like every other page kind; the
    // standalone diffTabs array is just the spec registry indexed by
    // id, looked up by the diff render branch below.
    const pageTabStartIdx = tabs.length;
    const pageTabsForThread = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
    // Pre-build the per-slot stack ([...back, current, ...forward]) so
    // back/forward stack entries get their own tab body that stays
    // mounted alongside the current page. This preserves state
    // (scroll, expanded trees, draft text) across in-tab navigation.
    // After the loop we tag non-current tabs as hidden so they don't
    // appear in the strip.
    const perThreadHistoryForBuilder = selectedThreadId ? threadPageHistory[selectedThreadId] ?? {} : {};
    const stripVisibleIds = new Set<string>([AGENT_TAB_ID]);
    for (const slotRef of pageTabsForThread) stripVisibleIds.add(slotRef.id);
    // Tab ids must be unique across the whole rendered list — they
    // back React keys, the active-tab lookup, and the close-button
    // handler. Duplicates make all three break: React warns about
    // duplicate keys, only one of the rendered tabs becomes
    // selectable, and the close button fires against an arbitrary
    // matching record. Track ids globally so a back-stack entry
    // that shares an id with another slot (or with its own slot's
    // current ref, which can happen when the same ref appears in
    // back+current after a malformed history transition) only
    // contributes one tab to the render.
    const renderedTabIds = new Set<string>([AGENT_TAB_ID]);
    // One renderer per page kind. `Record<PageKind, …>` makes the table
    // exhaustive: a kind without a renderer (or a renderer for a kind that
    // no longer exists) is a compile error, not a blank tab.
    type SlotNav = {
      /** The slot's current ref id — in-tab navigation mutates the slot. */
      slotId: string;
      navOpen: (newRef: TabRef, opts?: { newTab?: boolean; siblings?: import("./tabs/PageNavigationContext.js").NavSiblings }) => void;
      navOpenFile: (path: string, opts?: { newTab?: boolean }) => void;
      navOpenDiff: (spec: DiffSpec, siblings?: import("./tabs/PageNavigationContext.js").NavSiblings) => void;
      navRevealCommit: (sha: string) => void;
    };
    const activeRef = refFromTabId(centerActive);
    const activeWikiSlug = activeRef?.kind === "wiki" ? (activeRef.payload as { slug: string }).slug : null;
    const workPage = (ref: TabRef, nav: SlotNav): CenterTab | null => {
      const sharedProps = {
        thread: selectedThread,
        activeThreadId: currentThreadState.activeThreadId,
        threadWork: selectedThreadWork,
        agentStatus: agentThreadStatus,
        backlog: backlogState,
        profile: workListProfile,
        fields: workListProfile.fields,
        onUpdateTask: handleUpdateTask,
        onDeleteTask: handleDeleteTask,
        onReorderTasks: handleReorderTasks,
        onReorderBacklog: handleReorderBacklog,
        onMoveItemToBacklog: handleMoveItemToBacklog,
        registerOpenCreate: (fn: () => void) => { planOpenCreateRef.current = fn; },
        onOpenNewTaskPage: (payload: { parentRef?: string | null }) =>
          nav.navOpen(newTaskRef(payload)),
        onOpenTaskPage: (ref: string) => nav.navOpen(workItemTabRef(ref)),
      };
      const labelByKind: Record<string, string> = {
        "tasks": "Tasks",
        "done-work": "Done Work",
        "backlog": "Backlog",
      };
      return {
        id: ref.id,
        label: labelByKind[ref.kind] ?? ref.kind,
        closable: true,
        render: () => {
          switch (ref.kind) {
            case "tasks":
              return <TasksPage {...sharedProps} streams={streams} currentStreamId={stream?.id ?? null} onOpenPage={nav.navOpen} />;
            case "done-work":
              return <DoneWorkPage {...sharedProps} />;
            case "backlog":
              return <BacklogPage {...sharedProps} />;
            default:
              return null;
          }
        },
      };
    };
    // A snapshot, an effort, an agent turn (entity pages, P2.11) and an
    // ad-hoc endpoints pair (`diff-view` route) all render as a diff.
    const diffViewTab = (ref: TabRef, nav: SlotNav): CenterTab | null => {
      const payload = ref.payload as DiffViewPayload | null;
      if (!payload) return null;
      const label =
        payload.mode === "snapshot"
          ? `Snapshot ${payload.snapshotId}`
          : payload.mode === "effort"
            ? `Effort ${payload.effortId}`
            : payload.mode === "turn"
              ? `Turn ${payload.turnId}`
              : "Diff";
      return {
        id: ref.id,
        label,
        closable: true,
        render: () => (
          <DiffViewPage
            stream={stream}
            spec={payload}
            onOpenDiff={nav.navOpenDiff}
            onOpenDiffInTab={nav.navOpenDiff}
            onOpenPage={nav.navOpen}
            onOpenFile={nav.navOpenFile}
          />
        ),
      };
    };
    const pageRenderers: Record<PageKind, (ref: TabRef, nav: SlotNav) => CenterTab | null> = {
      // The agent tab is slot 0 above; it is never a page tab.
      agent: () => null,
      diff: (ref, nav) => {
        // Diff that arrived via in-tab navigation. Look up the
        // registered spec; skip if missing (the registration path is
        // handleOpenDiffInTab — a stale ref without a spec would be
        // a bug).
        const spec = diffTabs.find((t) => t.id === ref.id)?.spec;
        if (!spec) return null;
        const label = spec.path.split("/").pop() ?? spec.path;
        const suffix = spec.labelOverride ?? "diff";
        return {
          id: ref.id,
          label: `${label} (${suffix})`,
          closable: true,
          render: () => stream ? (
            <DiffPage
              stream={stream}
              spec={spec}
              visible={effectiveCenterActive === ref.id}
              onJumpToSource={(p) => {
                // In-tab navigation: replace the slot's diff with
                // the file. Back returns to the diff. Browser-tab
                // semantics; do NOT close the diff manually here —
                // handleNavigateInTab takes care of swapping the
                // slot's ref while keeping the diff in the back stack.
                nav.navOpenFile(p);
              }}
            />
          ) : null,
        };
      },
      "duplicate-block": (ref, nav) => {
        const payload = ref.payload as import("./tabs/pageRefs.js").DuplicateBlockPayload | null;
        if (!payload) return null;
        const leftBase = payload.leftPath.split("/").pop() ?? payload.leftPath;
        const rightBase = payload.rightPath.split("/").pop() ?? payload.rightPath;
        return {
          id: ref.id,
          label: `${leftBase} ↔ ${rightBase}`,
          closable: true,
          render: () => stream ? (
            <DuplicateBlockPage
              stream={stream}
              payload={payload}
              visible={effectiveCenterActive === ref.id}
              onJumpToSource={(p, v) => {
                handleNavigateInTab(nav.slotId, fileRef(p, v));
              }}
            />
          ) : null,
        };
      },
      file: (ref, nav) => {
        const payload = ref.payload as { path?: string; version?: import("./revision.js").Revision } | null;
        const path = payload?.path;
        if (!path) return null;
        const version = payload?.version ?? WORKING;
        const basename = path.split("/").pop() ?? path;
        // Non-disk versions render through FileViewerPage — read-only,
        // no dirty state, no save plumbing. The EditorPane / save
        // pipeline stays disk-only on purpose so the dirty cache,
        // LSP, find-in-file, etc. don't have to grow "is this read
        // only?" branches.
        if (version !== WORKING) {
          const versionLabel = shortRevisionLabel(version);
          return {
            id: ref.id,
            label: `${basename} (${versionLabel})`,
            closable: true,
            render: () => stream ? (
              <FileViewerPage
                stream={stream}
                path={path}
                version={version}
                visible={effectiveCenterActive === ref.id}
              />
            ) : null,
          };
          return null;
        }
        const file = currentSession.files[path];
        const dirty = !!file && file.draftContent !== file.savedContent;
        return {
          id: ref.id,
          label: `${dirty ? "● " : ""}${basename}`,
          closable: true,
          render: () => stream ? (
            <FilePage
              dirty={dirty}
              stream={stream}
              filePath={path}
              value={file?.draftContent ?? ""}
              isDirty={dirty}
              onChange={handleEditorChange}
              onSave={() => { void handleEditorSave(); }}
              findRequest={editorFindRequest}
              navigationTarget={editorNavigationTarget?.path === path ? editorNavigationTarget : null}
              onNavigateToLocation={handleNavigateToLocation}
              openFileOrder={currentSession.openOrder}
              openFiles={currentSession.files}
              onRevealCommit={handleRevealCommit}
              onCompareWithClipboard={handleCompareWithClipboard}
            />
          ) : null,
        };
      },
      settings: (ref, nav) => {
        return {
          id: ref.id,
          label: "Settings",
          closable: true,
          render: () => <SettingsPage onClose={() => closePageTab(ref.id)} />,
        };
      },
      "local-history": (ref, nav) => {
        return {
          id: ref.id,
          label: "Local History",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={nav.navOpen}
            />
          ),
        };
      },
      "local-history-full": (ref, nav) => {
        return {
          id: ref.id,
          label: "All snapshots",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={nav.navOpen}
              mode="full-list"
            />
          ),
        };
      },
      "local-history-by-commit-full": (ref, nav) => {
        return {
          id: ref.id,
          label: "All commits",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={nav.navOpen}
              mode="full-by-commit"
            />
          ),
        };
      },
      "diff-view": (ref, nav) => diffViewTab(ref, nav),
      snapshot: (ref, nav) => diffViewTab(ref, nav),
      effort: (ref, nav) => diffViewTab(ref, nav),
      turn: (ref, nav) => diffViewTab(ref, nav),
      "git-history": (ref, nav) => {
        return {
          id: ref.id,
          label: "Git History",
          closable: true,
          render: () => (
            <GitHistoryPage stream={stream} onOpenPage={nav.navOpen} />
          ),
        };
      },
      "git-dashboard": (ref, nav) => {
        return {
          id: ref.id,
          label: "Git Dashboard",
          closable: true,
          render: () => (
            <GitDashboardPage
              stream={stream}
              onOpenPage={nav.navOpen}
              onRevealCommit={nav.navRevealCommit}
            />
          ),
        };
      },
      "uncommitted-changes": (ref, nav) => {
        return {
          id: ref.id,
          label: "Uncommitted",
          closable: true,
          render: () => (
            <UncommittedChangesPage
              stream={stream}
              onOpenPage={nav.navOpen}
              onOpenFile={nav.navOpenFile}
              onOpenDiff={nav.navOpenDiff}
              onOpenDiffInTab={nav.navOpenDiff}
            />
          ),
        };
      },
      commit: (ref, nav) => {
        const sha = (ref.payload as { sha?: string } | null)?.sha ?? "";
        return {
          id: ref.id,
          label: sha ? sha.slice(0, 7) : "commit",
          closable: true,
          render: () => (
            <GitCommitPage
              stream={stream}
              sha={sha}
              threadWork={selectedThreadWork}
              onOpenDiff={nav.navOpenDiff}
              onOpenDiffInTab={nav.navOpenDiff}
              onOpenPage={nav.navOpen}
              onOpenFile={nav.navOpenFile}
            />
          ),
        };
      },
      "hook-events": (ref, nav) => {
        return {
          id: ref.id,
          label: "Hook Events",
          closable: true,
          render: () => <HookEventsPage streamId={stream?.id ?? null} />,
        };
      },
      terminal: (ref, nav) => {
        return {
          id: ref.id,
          label: "Terminal",
          closable: true,
          render: () => (
            <TerminalPage
              stream={stream}
              visible={effectiveCenterActive === ref.id}
              onOpenFile={(absPath, line, column) => {
                if (!stream) return;
                const wt = stream.worktree_path.endsWith("/")
                  ? stream.worktree_path.slice(0, -1)
                  : stream.worktree_path;
                const rel = absPath.startsWith(wt + "/")
                  ? absPath.slice(wt.length + 1)
                  : absPath;
                if (typeof line === "number" && line > 0) {
                  void handleNavigateToLocation({ path: rel, line, column: column ?? 1 });
                } else {
                  void handleOpenFile(rel);
                }
              }}
            />
          ),
        };
      },
      alerts: (ref, nav) => {
        return {
          id: ref.id,
          label: "Alerts",
          closable: true,
          render: () => <AlertsPage onOpenPage={nav.navOpen} />,
        };
      },
      files: (ref, nav) => {
        return {
          id: ref.id,
          label: "Files",
          closable: true,
          render: () => (
            <FilesPage
              stream={stream}
              vcsEnabled={workspaceContext.vcsEnabled}
              selectedFilePath={selectedFilePath}
              generated={generated.exclude}
              onOpenFile={nav.navOpenFile}
              onOpenDiff={nav.navOpenDiff}
              onCreateFile={handleCreateFile}
              onCreateDirectory={handleCreateDirectory}
              onRenamePath={handleRenamePath}
              onDeletePath={handleDeletePath}
              onToggleGenerated={handleToggleGenerated}
              commitRequest={commitFilesRequest}
            />
          ),
        };
      },
      "wiki-index": (ref, nav) => {
        return {
          id: ref.id,
          label: "Wiki",
          closable: true,
          render: () => (
            <WikiIndexPage
              stream={stream}
              selectedSlug={activeWikiSlug}
              onOpenWikiPage={handleOpenWiki}
            />
          ),
        };
      },
      comments: (ref, nav) => {
        return {
          id: ref.id,
          label: "Comments Dashboard",
          closable: true,
          render: () => <CommentsInboxPage stream={stream} onOpenPage={nav.navOpen} />,
        };
      },
      "metrics-recorded": (ref, nav) => {
        return {
          id: ref.id,
          label: "Metrics",
          closable: true,
          render: () => <MetricsPage onOpenPage={nav.navOpen} />,
        };
      },
      metric: (ref, nav) => {
        const p = (ref.payload ?? null) as { metricKey?: string } | null;
        return {
          id: ref.id,
          label: "Metric",
          closable: true,
          render: () => (
            <MetricDetailPage metricKey={p?.metricKey} onOpenPage={nav.navOpen} />
          ),
        };
      },
      "custom-dashboard": (ref, nav) => {
        // `customDashboardRef(id)` — one user-created dashboard (grid of tiles).
        const p = (ref.payload ?? null) as { id?: string } | null;
        return {
          id: ref.id,
          label: "Dashboard",
          closable: true,
          render: () => <CustomDashboardPage dashboardId={p?.id} onOpenPage={nav.navOpen} />,
        };
      },
      dashboards: (ref, nav) => {
        return {
          id: ref.id,
          label: "Dashboards",
          closable: true,
          render: () => <DashboardsIndexPage onOpenPage={nav.navOpen} />,
        };
      },
      problems: (ref, nav) => ({
        id: ref.id,
        label: "Problems",
        closable: true,
        render: () => (stream ? <ProblemsPage streamId={stream.id} onOpenPage={nav.navOpen} /> : null),
      }),
      symbols: (ref, nav) => {
        const path = (ref.payload as { path?: string | null } | null)?.path ?? null;
        return {
          id: ref.id,
          label: path ? `Symbols — ${path.split("/").pop()}` : "Symbols",
          closable: true,
          render: () => (stream ? <SymbolsPage streamId={stream.id} path={path} onOpenPage={nav.navOpen} /> : null),
        };
      },
      // A symbol opens its file at its line (openSymbol); it's never a tab.
      symbol: () => null,
      board: (ref, nav) => ({
        id: ref.id,
        label: "Board",
        closable: true,
        render: () => (
          <BoardPage threadId={selectedThreadId ?? null} streamId={stream?.id ?? null} onOpenPage={nav.navOpen} />
        ),
      }),
      catalog: (ref, nav) => ({
        id: ref.id,
        label: "Catalog",
        closable: true,
        render: () => <CatalogPage onOpenPage={nav.navOpen} />,
      }),
      "explore-data": (ref, nav) => {
        return {
          id: ref.id,
          label: "Explore Data",
          closable: true,
          render: () => <ExploreDataPage stream={stream} onOpenPage={nav.navOpen} />,
        };
      },
      "ext-page": (ref, nav) => {
        const { extension, page, params } = ref.payload as {
          extension: string;
          page: string;
          params?: Record<string, string>;
        };
        return {
          id: ref.id,
          label: page,
          closable: true,
          render: () => (
            <ExtensionPageView
              extension={extension}
              page={page}
              params={params}
              stream={stream}
              onOpenPage={nav.navOpen}
            />
          ),
        };
      },
      lens: (ref, nav) => {
        const payload = ref.payload as { lensId?: string; params?: Record<string, SqlCell> } | null;
        const lensId = payload?.lensId ?? "";
        return {
          id: ref.id,
          label: lensId,
          closable: true,
          render: () => (
            <LensPage lensId={lensId} initialParams={payload?.params} stream={stream} onOpenPage={nav.navOpen} />
          ),
        };
      },
      "tasks": workPage,
      "done-work": workPage,
      "backlog": workPage,
      "closed-threads": (ref, nav) => {
        return {
          id: ref.id,
          label: "Closed Threads",
          closable: true,
          render: () => <ClosedThreadsPage stream={stream} />,
        };
      },
      "external-url": (ref, nav) => {
        const externalUrl = (ref.payload as { url?: string } | null)?.url ?? "";
        let label = externalUrl;
        try {
          const u = new URL(externalUrl);
          label = u.host + (u.pathname && u.pathname !== "/" ? u.pathname : "");
        } catch { /* keep raw */ }
        return {
          id: ref.id,
          label: label.length > 40 ? label.slice(0, 40) + "…" : label,
          closable: true,
          contextMenu: [
            {
              id: "external-url.open-in-browser",
              label: "Open in Browser",
              enabled: true,
              run: () => { void openExternalUrl(externalUrl); },
            },
            {
              id: "external-url.copy",
              label: "Copy URL",
              enabled: true,
              run: () => { void navigator.clipboard.writeText(externalUrl).catch(() => {}); },
            },
          ],
          render: () => (
            <ExternalUrlPage
              url={externalUrl}
              onOpenInBrowser={(u) => { void openExternalUrl(u); }}
            />
          ),
        };
      },
      wiki: (ref, nav) => {
        const slug = (ref.payload as { slug?: string } | null)?.slug ?? "";
        const wikiNavOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        return {
          id: ref.id,
          label: slug,
          closable: true,
          render: () => stream ? (
            <WikiPage
              stream={stream}
              slug={slug}
              threadWork={selectedThreadWork}
              onClosed={() => closeOrGoBackPageTab(ref.id)}
              onOpenWikiPage={handleOpenWiki}
              onOpenFile={nav.navOpenFile}
              onOpenDirectory={handleOpenDirectory}
              onOpenPage={wikiNavOpen}
              onOpenCommit={handleOpenCommit}
              onOpenExternalUrl={handleOpenExternalUrl}
            />
          ) : null,
        };
      },
      "wiki-freshness": (ref, nav) => {
        const slug = (ref.payload as { slug?: string } | null)?.slug ?? "";
        const freshnessNavOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        return {
          id: ref.id,
          label: `Freshness — ${slug}`,
          closable: true,
          render: () => <WikiFreshnessPage slug={slug} onOpenPage={freshnessNavOpen} />,
        };
      },
      dir: (ref, nav) => {
        const dirPath = (ref.payload as { path?: string } | null)?.path ?? "";
        const dirNavOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        return {
          id: ref.id,
          label: dirPath || "/",
          closable: true,
          render: () => (
            <DirectoryPage
              stream={stream}
              path={dirPath}
              onOpenPage={dirNavOpen}
            />
          ),
        };
      },
      work_item: (ref, nav) => {
        // Every work item, whichever list it's on, has one page.
        const itemRef = (ref.payload as { ref?: string } | null)?.ref ?? ref.id;
        const loaded = selectedThreadWork?.all.find((i) => i.ref === itemRef);
        return {
          id: ref.id,
          label: loaded ? loaded.title : workItemLabel(itemRef),
          closable: true,
          render: () => (
            <WorkItemPage
              workItemRef={itemRef}
              stream={stream}
              thread={selectedThread}
              onOpenPage={nav.navOpen}
              onOpenFile={(p) => nav.navOpenFile(p)}
              onShowEffortDiff={(effortId) => nav.navOpen(effortDiffRef(effortId))}
              onOpenDiff={nav.navOpenDiff}
              onDeleted={(deleted) => closeOrGoBackPageTab(workItemTabRef(deleted).id)}
            />
          ),
        };
      },
      "stream-settings": (ref, nav) => {
        const targetStreamId = (ref.payload as { streamId?: string } | null)?.streamId ?? "";
        const targetStream = streams.find((s) => s.id === targetStreamId) ?? null;
        return {
          id: ref.id,
          label: targetStream ? `Settings · ${targetStream.title}` : "Stream Settings",
          closable: true,
          render: () => (
            <StreamSettingsPage
              stream={targetStream}
              onClose={() => closePageTab(ref.id)}
              onSaved={(next) => setStreams(next)}
            />
          ),
        };
      },
      "thread-settings": (ref, nav) => {
        const targetThreadId = (ref.payload as { threadId?: string } | null)?.threadId ?? "";
        const targetThread = currentThreadState.threads.find((t) => t.id === targetThreadId) ?? null;
        return {
          id: ref.id,
          label: targetThread ? `Settings · ${targetThread.title}` : "Thread Settings",
          closable: true,
          render: () => (
            <ThreadSettingsPage
              streamId={stream?.id ?? ""}
              thread={targetThread}
              onClose={() => closePageTab(ref.id)}
              onSaved={(nextThreads) => {
                if (!stream) return;
                setThreadStates((prev) => ({
                  ...prev,
                  [stream.id]: {
                    ...(prev[stream.id] ?? { selectedThreadId: null, activeThreadId: null, threads: [] }),
                    threads: nextThreads,
                  },
                }));
              }}
            />
          ),
        };
      },
      "new-stream": (ref, nav) => {
        return {
          id: ref.id,
          label: "New Stream",
          closable: true,
          render: () => (
            <NewStreamPage
              vcsEnabled={workspaceContext.vcsEnabled}
              defaultTitle={`Stream ${streams.length + 1}`}
              onClose={() => closePageTab(ref.id)}
              onCreated={(created) => {
                handleStreamCreated(created);
                closePageTab(ref.id);
              }}
            />
          ),
        };
      },
      "new-task": (ref) => {
        const payload = (ref.payload as { parentRef?: string | null } | null) ?? {};
        return {
          id: ref.id,
          label: "New item",
          closable: true,
          render: () => (
            <NewTaskPage
              defaults={{ parentRef: payload.parentRef ?? null }}
              epics={selectedThreadWork?.epics ?? []}
              onClose={() => closePageTab(ref.id)}
              onSubmit={(input) => handleCreateTask({ ...input, state: input.state ?? "todo" })}
            />
          ),
        };
      },
      dashboard: (ref, nav) => {
        return {
          id: ref.id,
          label: "Go To",
          closable: true,
          render: () => (
            <DashboardPage stream={stream} threadId={selectedThreadId} onOpenPage={nav.navOpen} />
          ),
        };
      },
    };
    for (const slotRef of pageTabsForThread) {
      const histEntry = perThreadHistoryForBuilder[slotRef.id] ?? { back: [], forward: [], siblings: null };
      // back/forward are HistoryFrame[] (ref + siblings); we only
      // need the refs for the render pass — siblings are restored
      // by handleGoBack/Forward when the user actually navigates.
      const rawSlotStack = [
        ...histEntry.back.map((f) => f.ref),
        slotRef,
        ...histEntry.forward.map((f) => f.ref),
      ];
      // Per-slot dedup: if back/forward contains an entry with the
      // same id as the slot's current ref (shouldn't happen in
      // normal flow, but corrupted history can produce it), keep
      // only the slot's current ref so we don't render two copies.
      const seenInSlot = new Set<string>();
      const slotStack: TabRef[] = [];
      for (const r of rawSlotStack) {
        if (seenInSlot.has(r.id)) continue;
        seenInSlot.add(r.id);
        slotStack.push(r);
      }
      // Closures bind navigation to the SLOT's current ref id so that
      // when a back-stack page (still mounted, hidden) navigates, it
      // mutates the slot — same behavior as the visible page.
      const navOpen = (
        newRef: TabRef,
        opts?: {
          newTab?: boolean;
          siblings?: import("./tabs/PageNavigationContext.js").NavSiblings;
        },
      ) => {
        if (opts?.newTab) handleOpenPage(newRef);
        else handleNavigateInTab(slotRef.id, newRef, opts?.siblings);
      };
      const navOpenFile = (path: string, opts?: { newTab?: boolean }) => {
        if (opts?.newTab) handleOpenPage(fileRef(path));
        else handleNavigateInTab(slotRef.id, fileRef(path));
      };
      // Open a diff *in this slot* — slot navigates to the diff,
      // back returns to the originating page. Used by pages that
      // surface a diff (tasks, wiki, local history, etc.).
      const navOpenDiff = (
        spec: DiffSpec,
        siblings?: import("./tabs/PageNavigationContext.js").NavSiblings,
      ) => {
        handleOpenDiffInTab(slotRef.id, spec, siblings);
      };
      const navRevealCommit = (sha: string) => {
        navOpen(gitCommitRef(sha));
      };
      const slotNav: SlotNav = { slotId: slotRef.id, navOpen, navOpenFile, navOpenDiff, navRevealCommit };
      for (const ref of slotStack) {
      // Global dedup across all slots — never push two tabs with
      // the same id, even if multiple slots' stacks happen to
      // contain it.
      if (renderedTabIds.has(ref.id)) continue;
      renderedTabIds.add(ref.id);
      const tab = pageRenderers[ref.kind](ref, slotNav);
      if (tab) tabs.push(tab);
      } // end inner stack loop
    }
    // Tag back/forward stack entries as hidden so they don't appear in
    // the tab strip but their bodies stay mounted (preserving state).
    for (let i = pageTabStartIdx; i < tabs.length; i++) {
      if (!stripVisibleIds.has(tabs[i]!.id)) {
        tabs[i] = { ...tabs[i]!, hidden: true };
      }
    }
    // Wrap each page-tab render with PageNavigationContext so descendants
    // (BacklinksList, RouteLink, in-page cross-references) can navigate
    // in-tab and the Page chrome auto-mounts a back/forward nav bar.
    const perThreadHistory = selectedThreadId ? threadPageHistory[selectedThreadId] ?? {} : {};
    const pageRefsForThread = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
    for (let i = pageTabStartIdx; i < tabs.length; i++) {
      const tab = tabs[i]!;
      const tabId = tab.id;
      const entry = perThreadHistory[tabId] ?? { back: [], forward: [], siblings: null };
      const ref = pageRefsForThread.find((r) => r.id === tabId);
      const innerRender = tab.render;
      const bookmarkScope = ref ? bookmarks.find((b) => b.ref.id === ref.id)?.scope ?? null : null;
      const registeredTitle = pageTitles[tabId];
      const navValue = {
        navigate: (newRef: TabRef, opts?: { newTab?: boolean; siblings?: import("./tabs/PageNavigationContext.js").NavSiblings }) => {
          if (opts?.newTab) handleOpenPage(newRef);
          else handleNavigateInTab(tabId, newRef, opts?.siblings);
        },
        goBack: () => handleGoBack(tabId),
        goForward: () => handleGoForward(tabId),
        canGoBack: entry.back.length > 0,
        canGoForward: entry.forward.length > 0,
        siblings: entry.siblings,
        goPrevSibling: entry.siblings && entry.siblings.index > 0
          ? () => handleStepSibling(tabId, entry.siblings!.index - 1)
          : undefined,
        goNextSibling: entry.siblings && entry.siblings.index < entry.siblings.entries.length - 1
          ? () => handleStepSibling(tabId, entry.siblings!.index + 1)
          : undefined,
        goSibling: entry.siblings
          ? (i: number) => handleStepSibling(tabId, i)
          : undefined,
        setTitle: (t: string) => setPageTitle(tabId, t),
        title: registeredTitle,
        pageKey: selectedThreadId ? `${selectedThreadId}::${tabId}` : undefined,
        ask: ref && parseRef(ref.id) ? { ref: ref.id } : undefined,
        bookmark: ref ? {
          scope: bookmarkScope,
          toggle: (scope: BookmarkScope) => {
            const viewer = { threadId: selectedThreadId, streamId: stream?.id ?? null };
            const write = bookmarkScope === scope
              ? removeBookmark(viewer, ref.id)
              : setBookmark(viewer, ref, registeredTitle ?? tab.label, scope);
            write.catch((e: unknown) =>
              recordOpError({ label: "Bookmark", message: e instanceof Error ? e.message : String(e) }));
          },
        } : undefined,
      };
      if (registeredTitle && registeredTitle !== tab.label) {
        tab.label = registeredTitle;
      }
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
    effectiveCenterActive,
    stream,
    currentSession.openOrder,
    currentSession.files,
    editorFindRequest,
    editorNavigationTarget,
    diffTabs,
    handleOpenWiki,
    handleOpenCommit,
    handleOpenExternalUrl,
    selectedThreadId,
    threadPageTabs,
    threadPageHistory,
    handleOpenPage,
    handleNavigateInTab,
    handleGoBack,
    handleGoForward,
    handleStepSibling,
    closePageTab,
    pageTitles,
    setPageTitle,
    bookmarks,
    workspaceContext.vcsEnabled,
    selectedFilePath,
    generated,
    commitFilesRequest,
    centerActive,
    currentThreadState.activeThreadId,
    selectedThreadWork,
    backlogState,
  ]);

  return (
    // One owner of every extension panel's runs, read by the rail, the
    // status bar's bell and the Alerts page.
    <PanelRunsProvider streamId={stream?.id ?? null} threadId={selectedThreadId}>
    <AlertToasts onReview={() => handleOpenPage(alertsRef())} />
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
          enabledAgents={enabledAgents}
          onSwitchStream={handleSwitch}
          onSelectThread={handleSelectThread}
          onCreateThread={async (streamId, title, agent, acpAgent) => {
            if (streamId !== stream?.id) await handleSwitch(streamId);
            await handleCreateThread(title, agent, acpAgent);
          }}
          onOpenNewStreamPage={() => handleOpenPage(newStreamRef())}
          onRenameStream={handleRenameStreamById}
          onRenameThread={handleRenameThreadById}
          onPromoteThread={handlePromoteThread}
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
              onActivate={(id) => {
                const path = diskFilePath(id);
                if (path !== null) handleSelectOpenFile(path);
                else setCenterActive(id);
              }}
              onClose={(id) => {
                const path = diskFilePath(id);
                if (path !== null) {
                  handleCloseOpenFile(path);
                  // A file can also live in threadPageTabs when it was
                  // reached via in-tab navigation from a page (Files
                  // index, git history, etc.). Removing only the
                  // session entry left the page-tab clinging with a
                  // blank EditorPane, uncloseable on a second click.
                  closePageTab(id);
                }
                else if (id.startsWith("diff:")) closeDiffTab(id);
                else closePageTab(id);
              }}
              onReorder={handleReorderCenterTabs}
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
              threadId={currentThreadState.activeThreadId ?? null}
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
        stream={stream}
        threadId={selectedThreadId}
        selectedFilePath={selectedFilePath}
        pages={computePagesDirectory({
          backlogReadyCount: backlogState?.items.length ?? 0,
        })}
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
      {stream && externalFilePrompt ? (
        <ExternalFileChangedDialog
          path={externalFilePrompt.path}
          onReload={() => {
            mutateFileSession(stream.id, (s) =>
              markFileSaved(s, externalFilePrompt.path, externalFilePrompt.content),
            );
            setExternalFilePrompt(null);
          }}
          onKeepMine={() => {
            mutateFileSession(stream.id, (s) =>
              setLoadedFileContent(s, externalFilePrompt.path, externalFilePrompt.content),
            );
            setExternalFilePrompt(null);
          }}
        />
      ) : null}
      {daemonUnavailable ? <DaemonDownDialog /> : null}
      <UndoToastStack />
      <PersonCommandConfirm />
      <RemoteConnectionBanner />
    </div>
    </PanelRunsProvider>
  );
}

/** A toast as each new thing needs the person; Review opens Alerts. */
function AlertToasts({ onReview }: { onReview(): void }) {
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

function ExternalFileChangedDialog({
  path,
  onReload,
  onKeepMine,
}: {
  path: string;
  onReload(): void;
  onKeepMine(): void;
}) {
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
          display: "flex",
          flexDirection: "column",
          gap: 16,
        }}
      >
        <div>
          <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>File changed on disk</div>
          <div style={{ color: "var(--muted)", lineHeight: 1.5 }}>
            <code>{path}</code> changed on disk while you had unsaved edits. Reload the file from disk or keep your
            draft and treat the new disk content as the latest saved version.
          </div>
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button"
            onClick={onKeepMine}
            style={{
              background: "transparent",
              color: "var(--fg)",
              border: "1px solid var(--border)",
              padding: "8px 14px",
              borderRadius: 4,
              cursor: "pointer",
              fontFamily: "inherit",
            }}
          >
            Keep my changes
          </button>
          <button type="button"
            onClick={onReload}
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
            Reload from disk
          </button>
        </div>
      </div>
    </div>
  );
}
