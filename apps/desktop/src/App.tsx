import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { flushSync } from "react-dom";
import {
  createTask,
  closeThread,
  createThread,
  deleteTask,
  getThreadWorkState,
  getThreadState,
  createWorkspaceDirectory,
  reorderTasks,
  moveTaskToThread,
  updateBacklogItem,
  deleteBacklogItem,
  reorderBacklog,
  moveTaskToBacklog,
  type AgentStatus,
  createWorkspaceFile,
  deleteWorkspacePath,
  getCurrentStream,
  getWorkspaceContext,
  gitPull,
  gitPush,
  type GitOpKickoff,
  type GitOpResult,
  desktopBridge,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameWorkspacePath,
  renameThread,
  renameStream,
  subscribeWikiPageEvents,
  subscribeTaskEvents,
  subscribeWorkspaceEvents,
  listRecentlyFinished,
  clearRecentlyFinished,
  openExternalUrl,
  type FinishedEntry,
  getBranchChanges,
  getRepoConflictState,
  subscribeGitRefsEvents,
  setGenerated,
  selectThread,
  promoteThread,
  recordUsage,
  reorderThreads,
  reorderStreams,
  switchStream,
  updateTask,
  writeWorkspaceFile,
  type BacklogState,
  type ThreadWorkState,
  type ThreadState,
  type Stream,
  type WorkspaceContext,
} from "./api.js";
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
import { buildMenuGroupSnapshots, buildMenuGroups, buildNativeMenuSnapshots, OPEN_RECENT_PREFIX } from "./commands.js";
import { externalFileSyncAction } from "./external-file-sync.js";
import type { EditorNavigationTarget } from "./lsp.js";
import { Navigator } from "./components/Navigator.js";
import { StatusBar } from "./components/StatusBar.js";
import { showToast } from "./components/toastStore.js";
import { UndoToastStack } from "./components/UndoToast.js";
import { subscribeUiError } from "./ui-error.js";
import { useBackendSubscriptions } from "./useBackendSubscriptions.js";
import { useFileSessions } from "./useFileSessions.js";
import { Menubar } from "./components/Menubar.js";
import { CenterTabs, type CenterTab } from "./components/CenterTabs/CenterTabs.js";
import type { DiffSpec } from "./components/Diff/DiffPane.js";
import { computeDiffId } from "./diff-id.js";
import { DiffPage } from "./pages/DiffPage.js";
import { DuplicateBlockPage } from "./pages/DuplicateBlockPage.js";
import { FileViewerPage } from "./pages/FileViewerPage.js";
import { RailHud } from "./components/RailHud/RailHud.js";
import type { TabRef } from "./tabs/tabState.js";
import { PageNavigationContext } from "./tabs/PageNavigationContext.js";
import { clearPageSnapshot } from "./tabs/usePageSnapshot.js";
import { useBookmarksStore } from "./tabs/useBookmarks.js";
import type { BookmarkScope } from "./tabs/bookmarks.js";
import { SettingsPage } from "./pages/SettingsPage.js";
import { CodeQualityPage } from "./pages/CodeQualityPage.js";
import { LocalHistoryDashboardPage } from "./pages/LocalHistoryDashboardPage.js";
import { SnapshotDetailPage } from "./pages/SnapshotDetailPage.js";
import { GitHistoryPage } from "./pages/GitHistoryPage.js";
import { GitDashboardPage } from "./pages/GitDashboardPage.js";
import { UncommittedChangesPage } from "./pages/UncommittedChangesPage.js";
import { AgentPage } from "./pages/AgentPage.js";
import { TerminalPage } from "./pages/TerminalPage.js";
import { HookEventsPage } from "./pages/HookEventsPage.js";
import { FilesPage } from "./pages/FilesPage.js";
import { DirectoryPage } from "./pages/DirectoryPage.js";
import { WikiIndexPage } from "./pages/WikiIndexPage.js";
import { TasksPage } from "./pages/TasksPage.js";
import { DoneWorkPage } from "./pages/DoneWorkPage.js";
import { BacklogPage } from "./pages/BacklogPage.js";
import { CommentsInboxPage } from "./pages/CommentsInboxPage.js";
import { ArchivedPage } from "./pages/ArchivedPage.js";
import { ClosedThreadsPage } from "./pages/ClosedThreadsPage.js";
import { ExternalUrlPage } from "./pages/ExternalUrlPage.js";
import { SubsystemDocsPage } from "./pages/SubsystemDocsPage.js";
import { TaskPage } from "./pages/TaskPage.js";
import { FindingPage } from "./pages/FindingPage.js";
import { WikiPage } from "./pages/WikiPage.js";
import { WikiFreshnessPage } from "./pages/WikiFreshnessPage.js";
import { DashboardPage } from "./pages/DashboardPage.js";
import { StreamSettingsPage } from "./pages/StreamSettingsPage.js";
import { ThreadSettingsPage } from "./pages/ThreadSettingsPage.js";
import { NewStreamPage } from "./pages/NewStreamPage.js";
import { NewTaskPage } from "./pages/NewTaskPage.js";
import { GitCommitPage } from "./pages/GitCommitPage.js";
import { OpErrorPage } from "./pages/OpErrorPage.js";
import { DomCommentLayer } from "./components/Comments/DomCommentLayer.js";
import { closedThreadsRef, commentsRef, directoryRef, externalUrlRef, fileRef, gitCommitRef, gitDashboardRef, indexRef, newStreamRef, newTaskRef, opErrorRef, snapshotRef, uncommittedChangesRef, wikiPageRef, streamSettingsRef, threadSettingsRef, taskRef } from "./tabs/pageRefs.js";
import { getOpErrorsStore, recordOpError } from "./components/opErrorsStore.js";
import { classifyExternalUrl } from "./external-url-allowlist.js";
import { installContextMenuSuppressor } from "./context-menu.js";
import { TerminalPane } from "./components/TerminalPane.js";
import { FilePage } from "./pages/FilePage.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { computePagesDirectory } from "./components/RailHud/sections.js";
import { NON_TRACKED_KINDS } from "./components/RailHud/history.js";
import { resolveActiveTabRef } from "./tabs/resolveActiveTabRef.js";
import { forgetPage, recordPageVisit, recordUserInterrupt } from "./api.js";
import { openProjectGuarded, listRecentProjects } from "./api.js";
import type { RecentProjectView } from "./tauri-bridge/generated/bindings.js";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { DISK } from "./file-version.js";
import { CommandPalette } from "./components/CommandPalette/CommandPalette.js";
import { SearchPalette } from "./components/SearchPalette.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { getCommandIdForShortcut } from "./keybindings.js";
import { logUi, setUiLogContext } from "./logger.js";

// Cap on concurrent file tabs in the center. Intellij uses ~10 by default;
// when this is exceeded, the oldest-touched tab without unsaved changes is
// closed automatically via enforceOpenFileLimit. Dirty tabs stay pinned.
const MAX_OPEN_FILE_TABS = 10;

// Persists which file tabs are open (per stream) across app restarts. Only the
// paths are saved — dirty state and scroll position are intentionally dropped.
const FILE_SESSIONS_STORAGE_KEY = "oxplow.layout.v1.fileSessions";
// Persists which center pane was last active ("agent", "file:<path>", or a
// diff tab id). Restored after file sessions are rebuilt; falls back to
// "agent" if the saved id is no longer resolvable (diff tabs never persist,
// and a file tab may have failed to reopen).
const CENTER_ACTIVE_STORAGE_KEY = "oxplow.layout.v1.centerActive";
// Persists the per-thread tab list (TabRef[]) and per-tab history
// across restarts. Pages mount fresh — no per-page snapshot yet. The
// snapshot layer is a follow-up that will rehydrate scroll positions,
// expanded trees, view toggles, etc.
const THREAD_TABS_STORAGE_KEY = "oxplow.layout.v1.threadPageTabs";
const THREAD_HISTORY_STORAGE_KEY = "oxplow.layout.v1.threadPageHistory";
const DIFF_SPECS_STORAGE_KEY = "oxplow.layout.v1.diffSpecs";

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

/** Read persisted per-thread tab lists. Returns an empty record on
 *  parse failure or absence — the user lands with no page tabs. */
function readPersistedThreadPageTabs(): Record<string, TabRef[]> {
  try {
    const raw = window.localStorage.getItem(THREAD_TABS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, TabRef[]> = {};
    for (const [threadId, refs] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof threadId !== "string" || !Array.isArray(refs)) continue;
      const clean = refs.filter((r): r is TabRef =>
        !!r && typeof r === "object" && typeof (r as TabRef).id === "string" && typeof (r as TabRef).kind === "string",
      );
      // Dedupe by id at read time so any pre-existing corrupted
      // state (the duplicate-git-dashboard bug) self-heals on
      // next launch. First occurrence wins so order is preserved.
      const seen = new Set<string>();
      const deduped: TabRef[] = [];
      for (const r of clean) {
        if (seen.has(r.id)) continue;
        seen.add(r.id);
        deduped.push(r);
      }
      if (deduped.length > 0) out[threadId] = deduped;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted threadPageTabs", { error: String(err) });
    return {};
  }
}

function writePersistedThreadPageTabs(tabs: Record<string, TabRef[]>): void {
  try {
    // Drop empty thread entries to keep the blob small.
    const out: Record<string, TabRef[]> = {};
    for (const [threadId, refs] of Object.entries(tabs)) {
      if (refs.length > 0) out[threadId] = refs;
    }
    window.localStorage.setItem(THREAD_TABS_STORAGE_KEY, JSON.stringify(out));
  } catch (err) {
    logUi("warn", "failed to write persisted threadPageTabs", { error: String(err) });
  }
}

/** A single back/forward stack frame. Stores both the ref and the
 *  siblings record from when that page was active, so going back
 *  restores the originating list's prev/next chain instead of
 *  dropping it. */
type HistoryFrame = { ref: TabRef; siblings: import("./tabs/PageNavigationContext.js").NavSiblings | null };
type ThreadHistory = Record<string, Record<string, { back: HistoryFrame[]; forward: HistoryFrame[]; siblings: import("./tabs/PageNavigationContext.js").NavSiblings | null }>>;

function readPersistedThreadPageHistory(): ThreadHistory {
  try {
    const raw = window.localStorage.getItem(THREAD_HISTORY_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    // Persisted blob may be the older shape (TabRef[] for back/
    // forward) — coerce on the fly so old data still restores.
    const out: ThreadHistory = {};
    for (const [threadId, perThread] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof threadId !== "string" || !perThread || typeof perThread !== "object") continue;
      const inner: ThreadHistory[string] = {};
      for (const [tabId, raw] of Object.entries(perThread as Record<string, unknown>)) {
        if (!raw || typeof raw !== "object") continue;
        const entry = raw as { back?: unknown; forward?: unknown; siblings?: unknown };
        const coerce = (arr: unknown): HistoryFrame[] => {
          if (!Array.isArray(arr)) return [];
          return arr.map((item) => {
            if (item && typeof item === "object" && "ref" in (item as object)) {
              return item as HistoryFrame;
            }
            return { ref: item as TabRef, siblings: null };
          });
        };
        inner[tabId] = {
          back: coerce(entry.back),
          forward: coerce(entry.forward),
          siblings: (entry.siblings ?? null) as ThreadHistory[string][string]["siblings"],
        };
      }
      out[threadId] = inner;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted threadPageHistory", { error: String(err) });
    return {};
  }
}

function writePersistedThreadPageHistory(history: ThreadHistory): void {
  try {
    window.localStorage.setItem(THREAD_HISTORY_STORAGE_KEY, JSON.stringify(history));
  } catch (err) {
    logUi("warn", "failed to write persisted threadPageHistory", { error: String(err) });
  }
}

function readPersistedDiffSpecs(): Array<{ id: string; spec: DiffSpec }> {
  try {
    const raw = window.localStorage.getItem(DIFF_SPECS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: Array<{ id: string; spec: DiffSpec }> = [];
    for (const entry of parsed) {
      if (!entry || typeof entry !== "object") continue;
      const id = (entry as { id?: unknown }).id;
      const rawSpec = (entry as { spec?: unknown }).spec;
      if (typeof id !== "string" || !rawSpec || typeof rawSpec !== "object") continue;
      const s = rawSpec as Record<string, unknown>;
      // Coerce pre-versioning persisted specs (leftRef + rightKind)
      // into the new (leftVersion, rightVersion) shape. Existing
      // tabs survive a restart without losing their target.
      let leftVersion = (s.leftVersion ?? null) as DiffSpec["leftVersion"] | null;
      let rightVersion = (s.rightVersion ?? null) as DiffSpec["rightVersion"] | null;
      if (!leftVersion) {
        const lr = typeof s.leftRef === "string" ? s.leftRef : null;
        leftVersion = lr ? { kind: "ref", ref: lr } : { kind: "disk" };
      }
      if (!rightVersion) {
        const rk = s.rightKind;
        if (rk === "working") rightVersion = { kind: "disk" };
        else if (rk && typeof rk === "object" && typeof (rk as { ref?: unknown }).ref === "string") {
          rightVersion = { kind: "ref", ref: (rk as { ref: string }).ref };
        } else {
          rightVersion = { kind: "disk" };
        }
      }
      out.push({
        id,
        spec: {
          path: typeof s.path === "string" ? s.path : "",
          leftVersion,
          rightVersion,
          baseLabel: typeof s.baseLabel === "string" ? s.baseLabel : "",
          labelOverride: typeof s.labelOverride === "string" ? s.labelOverride : undefined,
          revealLine: typeof s.revealLine === "number" ? s.revealLine : undefined,
        },
      });
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted diff specs", { error: String(err) });
    return [];
  }
}

function writePersistedDiffSpecs(specs: Array<{ id: string; spec: DiffSpec }>): void {
  try {
    // Drop clipboard / synthetic specs that carry inline content too
    // large to persist comfortably; their `leftContent` / `rightContent`
    // are runtime-only. Keep ref-based diffs (fromRef/toRef paths) which
    // can be re-resolved on boot by reading the git refs.
    const persistable = specs.filter((s) => !s.spec.leftContent && !s.spec.rightContent);
    window.localStorage.setItem(DIFF_SPECS_STORAGE_KEY, JSON.stringify(persistable));
  } catch (err) {
    logUi("warn", "failed to write persisted diff specs", { error: String(err) });
  }
}

function readPersistedFileSessionPaths(): Record<string, string[]> {
  try {
    const raw = window.localStorage.getItem(FILE_SESSIONS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, string[]> = {};
    for (const [streamId, paths] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof streamId !== "string") continue;
      if (!Array.isArray(paths)) continue;
      const clean = paths.filter((p): p is string => typeof p === "string");
      if (clean.length > 0) out[streamId] = clean;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted file sessions", { error: String(err) });
    return {};
  }
}

function writePersistedFileSessionPaths(sessions: Record<string, FileSessionState>): void {
  try {
    const out: Record<string, string[]> = {};
    for (const [streamId, session] of Object.entries(sessions)) {
      if (session.openOrder.length > 0) out[streamId] = session.openOrder;
    }
    window.localStorage.setItem(FILE_SESSIONS_STORAGE_KEY, JSON.stringify(out));
  } catch {}
}

function readPersistedCenterActive(): string | null {
  try {
    const raw = window.localStorage.getItem(CENTER_ACTIVE_STORAGE_KEY);
    return typeof raw === "string" && raw.length > 0 ? raw : null;
  } catch {
    return null;
  }
}

function writePersistedCenterActive(value: string): void {
  try {
    window.localStorage.setItem(CENTER_ACTIVE_STORAGE_KEY, value);
  } catch {}
}

/// Prompt for a folder and open it as a project. `newWindow=false`
/// replaces the current window (this process exits once the new one
/// spawns); `true` opens an additional independent window. Shared by
/// the File ▸ Open Project commands and Cmd+K palette.
async function pickAndOpenProject(newWindow: boolean) {
  const selected = await openFolderDialog({
    directory: true,
    multiple: false,
    title: newWindow ? "Open Project in New Window" : "Open Project",
  });
  if (typeof selected !== "string") return;
  try {
    await openProjectGuarded(selected, newWindow);
  } catch (e) {
    recordOpError({
      label: "Open project",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [threadStates, setThreadStates] = useState<Record<string, ThreadState>>({});
  // Mirror of threadStates for subscription callbacks that need the
  // latest map without re-subscribing when it changes (see
  // useBackendSubscriptions). Kept current on every render.
  const threadStatesRef = useRef(threadStates);
  threadStatesRef.current = threadStates;
  const [threadWorkStates, setThreadWorkStates] = useState<Record<string, ThreadWorkState>>({});
  const [backlogState, setBacklogState] = useState<BacklogState | null>(null);
  const [agentStatuses, setAgentStatuses] = useState<Record<string, AgentStatus>>({});
  const [stream, setStream] = useState<Stream | null>(null);
  // Per-thread active center tab. The map is the source of truth; `centerActive`
  // and `setCenterActive` below are derived helpers so existing handler code
  // keeps working unchanged. Each thread remembers its last active tab so
  // switching threads restores it. The initial seed comes from the legacy
  // global localStorage key (the "default" thread inherits whatever was last
  // active before the per-thread refactor).
  const [threadCenterActive, setThreadCenterActive] = useState<Record<string, string>>({});
  // Per-thread open "page" tabs that aren't files/notes/diffs (Start, future
  // index/dashboard pages). Stored as TabRef so the rendering side can
  // dispatch by kind. Independent of the legacy noteTabs/diffTabs lists,
  // which still drive the tabs they own.
  const [threadPageTabs, setThreadPageTabs] = useState<Record<string, TabRef[]>>(() => readPersistedThreadPageTabs());
  // Per-tab page titles, keyed by tab id. Pages register their title via
  // PageNavigationContext.setTitle (the usePageTitle helper). Drives both
  // the tab strip label and the shared chrome header so the title lives in
  // exactly one place.
  const [pageTitles, setPageTitles] = useState<Record<string, string>>({});
  const setPageTitle = useCallback((tabId: string, title: string) => {
    setPageTitles((prev) => (prev[tabId] === title ? prev : { ...prev, [tabId]: title }));
  }, []);
  // Per-thread browser-style back/forward history for page tabs. Keyed by
  // the tab's *current* ref id; when an in-tab navigation replaces a tab's
  // ref, the entry is migrated to the new id along with the swap. Files,
  // notes, diffs, and the agent tab don't participate.
  const [threadPageHistory, setThreadPageHistory] = useState<ThreadHistory>(() => readPersistedThreadPageHistory());
  const [diffTabs, setDiffTabs] = useState<Array<{ id: string; spec: DiffSpec }>>(() => readPersistedDiffSpecs());
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const { fileSessions, setFileSessions, getFileSession, mutateFileSession } = useFileSessions();
  const restoredStreamsRef = useRef<Set<string>>(new Set());
  const centerActiveValidatedRef = useRef(false);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const [streamCreateRequest, setStreamCreateRequest] = useState(0);
  const [threadCreateRequest, setThreadCreateRequest] = useState(0);
  const [commitFilesRequest, setCommitFilesRequest] = useState(0);
  const [generated, setGeneratedState] = useState<string[]>([]);
  const opErrorsStore = getOpErrorsStore();
  const opErrorsAll = useSyncExternalStore(opErrorsStore.subscribe, opErrorsStore.getSnapshot);
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

  useEffect(() => {
    Promise.all([listStreams(), getCurrentStream(), getWorkspaceContext()])
      .then(async ([allStreams, current, context]) => {
        const initialThreadState = await getThreadState(current.id);
        const initialThread = initialThreadState.threads.find((thread) => thread.id === initialThreadState.selectedThreadId);
        if (initialThread) {
          const initialWork = await getThreadWorkState(current.id, initialThread.id);
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
          gitEnabled: context.gitEnabled,
        });
      })
      .catch((e) => {
        setError(String(e));
        setDaemonUnavailable(true);
        logUi("error", "failed to load initial app state", { error: String(e) });
      });
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function check() {
      const alive = await probeDaemon();
      if (cancelled) return;
      const decision = advanceDaemonProbeState(daemonProbeState.current, alive);
      daemonProbeState.current = decision.next;
      if (decision.refresh) {
        logUi("info", "daemon recovered, refreshing ui");
        window.location.reload();
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
        const nextWork = await getThreadWorkState(next.id, nextThread.id);
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
        const seeded = nextSession.selectedPath ? `file:${nextSession.selectedPath}` : "agent";
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
        const seeded = "agent";
        setThreadCenterActive((prev) => (
          prev[thread.id] !== undefined ? prev : { ...prev, [thread.id]: seeded }
        ));
        void getThreadWorkState(next.id, thread.id).then((work) => {
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
    const currentSession = getFileSession(stream.id);
    const existing = currentSession.files[path];
    mutateFileSession(stream.id, (base) => {
      const opened = existing
        ? selectOpenFile(base, path)
        : setOpenFileLoading(openFileInSession(base, path, "", true), path, true);
      return enforceOpenFileLimit(opened, MAX_OPEN_FILE_TABS);
    });
    // File tabs participate in the unified per-thread page tab list
    // so they share the same chrome and back/forward substrate as
    // every other page kind. fileSessions still owns the file content
    // + dirty state; threadPageTabs owns the tab membership.
    if (selectedThreadId) {
      const ref = fileRef(path);
      setThreadPageTabs((prev) => {
        const existing = prev[selectedThreadId] ?? [];
        if (existing.some((t) => t.id === ref.id)) return prev;
        return { ...prev, [selectedThreadId]: [...existing, ref] };
      });
    }
    setCenterActive(`file:${path}`);
    setError(null);
    void recordUsage({
      kind: "editor-file",
      key: path,
      event: "open",
      streamId: stream.id,
      threadId: selectedThread?.id ?? null,
    }).catch(() => {});
    if (existing && !existing.isLoading) return;
    try {
      logUi("debug", "open file: readWorkspaceFile start", { streamId: stream.id, path });
      const file = await readWorkspaceFile(stream.id, path);
      logUi("debug", "open file: readWorkspaceFile end", {
        streamId: stream.id,
        path,
        size: file.content.length,
        lineCount: file.content.split("\n").length,
      });
      mutateFileSession(stream.id, (s) => setLoadedFileContent(s, file.path, file.content));
      logUi("info", "opened file", { streamId: stream.id, path: file.path });
    } catch (e) {
      const msg = String(e);
      // A simply-missing file (e.g. a stale terminal link) is benign — show a
      // friendly message and log at warn, not a scary error with a raw OS code.
      const notFound = /no such file|not found|os error 2/i.test(msg);
      if (notFound) {
        setError(`File not found: ${path}`);
        logUi("warn", "open file: target does not exist", { streamId: stream.id, path });
      } else {
        setError(msg);
        logUi("error", "failed to open file", { streamId: stream.id, path, error: msg });
      }
      mutateFileSession(stream.id, (s) => closeOpenFile(s, path));
    }
  }

  async function handleNavigateToLocation(target: EditorNavigationTarget) {
    await handleOpenFile(target.path);
    setEditorNavigationTarget(target);
    setCenterActive(`file:${target.path}`);
  }

  function handleEditorChange(value: string) {
    if (!stream) return;
    const session = getFileSession(stream.id);
    if (!session.selectedPath) return;
    mutateFileSession(stream.id, (s) => updateFileDraft(s, session.selectedPath!, value));
  }

  async function handleEditorSave() {
    if (!stream) return;
    const session = getFileSession(stream.id);
    const selectedPath = session.selectedPath;
    if (!selectedPath) return;
    const current = session.files[selectedPath];
    if (!current || current.isLoading) return;
    mutateFileSession(stream.id, (s) => setOpenFileLoading(s, selectedPath, true));
    try {
      const saved = await writeWorkspaceFile(stream.id, selectedPath, current.draftContent);
      mutateFileSession(stream.id, (s) => markFileSaved(s, saved.path, saved.content));
      setError(null);
      logUi("info", "saved file", { streamId: stream.id, path: saved.path });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to save file", { streamId: stream.id, path: selectedPath, error: String(e) });
      mutateFileSession(stream.id, (s) => setOpenFileLoading(s, selectedPath, false));
    }
  }

  function handleSelectOpenFile(path: string) {
    if (!stream) return;
    mutateFileSession(stream.id, (s) => selectOpenFile(s, path));
    setCenterActive(`file:${path}`);
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
          setCenterActive(`file:${path}`);
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
        const work = await getThreadWorkState(streamId, thread.id);
        setThreadWorkStates((prev) => ({ ...prev, [thread.id]: work }));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateThread(title: string) {
    if (!stream) return;
    try {
      const next = await createThread(stream.id, title);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      const thread = next.threads.find((candidate) => candidate.id === next.selectedThreadId);
      if (thread) {
        const work = await getThreadWorkState(stream.id, thread.id);
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
      setCenterActive("agent");
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

  async function handleReorderStreams(orderedStreamIds: string[]) {
    try {
      await reorderStreams(orderedStreamIds);
      setStreams((prev) => {
        const byId = new Map(prev.map((s) => [s.id, s]));
        return orderedStreamIds.map((id) => byId.get(id)).filter((s): s is Stream => s !== undefined);
      });
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateTask(input: {
    title: string;
    description?: string;
    parentId?: number | null;
    status?: "ready" | "in_progress" | "blocked" | "done" | "canceled" | "archived";
    priority?: "low" | "medium" | "high" | "urgent";
  }) {
    if (!stream || !selectedThread) return;
    try {
      const next = await createTask(stream.id, selectedThread.id, input);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleUpdateTask(
    itemId: number,
    changes: {
      title?: string;
      description?: string;
      parentId?: number | null;
      status?: "ready" | "in_progress" | "blocked" | "done" | "canceled" | "archived";
      priority?: "low" | "medium" | "high" | "urgent";
    },
  ) {
    if (!stream || !selectedThread) return;
    try {
      const next = await updateTask(stream.id, selectedThread.id, itemId, changes);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleDeleteTask(itemId: number) {
    if (!stream || !selectedThread) return;
    try {
      const next = await deleteTask(stream.id, selectedThread.id, itemId);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderTasks(orderedItemIds: number[]) {
    if (!stream || !selectedThread) return;
    try {
      const next = await reorderTasks(stream.id, selectedThread.id, orderedItemIds);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveTaskToThread(itemId: number, fromThreadId: string, toThreadId: string) {
    if (!stream || fromThreadId === toThreadId) return;
    try {
      const { from, to } = await moveTaskToThread(stream.id, fromThreadId, itemId, toThreadId);
      setThreadWorkStates((prev) => ({ ...prev, [fromThreadId]: from, [toThreadId]: to }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveItemToBacklog(itemId: number, fromThreadId: string) {
    if (!stream) return;
    try {
      const { from, backlog } = await moveTaskToBacklog(stream.id, fromThreadId, itemId);
      setThreadWorkStates((prev) => ({ ...prev, [fromThreadId]: from }));
      setBacklogState(backlog);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleUpdateBacklogItem(itemId: number, changes: Parameters<typeof updateBacklogItem>[1]) {
    try {
      const next = await updateBacklogItem(itemId, changes);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleDeleteBacklogItem(itemId: number) {
    try {
      const next = await deleteBacklogItem(itemId);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderBacklog(orderedItemIds: number[]) {
    try {
      const next = await reorderBacklog(orderedItemIds);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
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
  const selectedThreadId = selectedThread?.id ?? null;
  // Derived from the per-thread map. When no thread is selected, fall back to
  // a sentinel that keeps existing UI selectors happy (they all default to
  // "agent" eventually).
  const centerActive = selectedThreadId
    ? threadCenterActive[selectedThreadId] ?? readPersistedCenterActive() ?? "agent"
    : "agent";
  const setCenterActive = useCallback(
    (next: string | ((prev: string) => string)) => {
      if (!selectedThreadId) return;
      setThreadCenterActive((prev) => {
        const current = prev[selectedThreadId] ?? readPersistedCenterActive() ?? "agent";
        const value = typeof next === "function" ? next(current) : next;
        if (value === current) return prev;
        return { ...prev, [selectedThreadId]: value };
      });
    },
    [selectedThreadId],
  );
  // Reset terminal transport to direct whenever the active pane target
  // changes — matches the old TerminalPane's internal useEffect.
  useEffect(() => { setAgentTransportMode("direct"); }, [selectedThread?.pane_target]);

  const selectedThreadWork = selectedThread ? threadWorkStates[selectedThread.id] ?? null : null;
  const opErrors = useMemo(
    () => opErrorsAll.filter((e) => e.threadId === null || e.threadId === selectedThreadId),
    [opErrorsAll, selectedThreadId],
  );
  useEffect(() => {
    opErrorsStore.setActiveThread(selectedThreadId);
  }, [selectedThreadId, opErrorsStore]);

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const threads = threadStates[s.id]?.threads ?? [];
      const anyWorking = threads.some((t) => agentStatuses[t.id] === "working");
      out[s.id] = anyWorking ? "working" : "waiting";
    }
    return out;
  }, [streams, threadStates, agentStatuses]);
  const streamActiveThreadIds = useMemo<Record<string, string | null>>(() => {
    const out: Record<string, string | null> = {};
    for (const s of streams) out[s.id] = threadStates[s.id]?.activeThreadId ?? null;
    return out;
  }, [streams, threadStates]);

  async function handleDropTaskOnStream(targetStreamId: string, itemId: number, fromThreadId: string | null) {
    if (!stream || !fromThreadId) return;
    const toThreadId = streamActiveThreadIds[targetStreamId];
    if (!toThreadId || toThreadId === fromThreadId) return;
    try {
      const { from, to } = await moveTaskToThread(stream.id, fromThreadId, itemId, toThreadId, targetStreamId);
      setThreadWorkStates((prev) => ({ ...prev, [fromThreadId]: from, [toThreadId]: to }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }
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

  // Persist the per-thread tab list + per-tab history. Pages mount
  // fresh on the next boot — the snapshot layer is a follow-up that
  // will rehydrate scroll positions, expanded trees, etc.
  useEffect(() => {
    writePersistedThreadPageTabs(threadPageTabs);
  }, [threadPageTabs]);
  useEffect(() => {
    writePersistedThreadPageHistory(threadPageHistory);
  }, [threadPageHistory]);
  useEffect(() => {
    writePersistedDiffSpecs(diffTabs);
  }, [diffTabs]);

  // After the first stream has had its file sessions rebuilt, verify the
  // initial (localStorage-seeded) centerActive is still resolvable. If it
  // points to a file that didn't come back, a diff tab (which never
  // persist), or a page tab that wasn't restored, snap back to "agent".
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
    if (centerActive === "agent") return;
    const session = fileSessions[stream.id];
    if (centerActive.startsWith("file:")) {
      const path = centerActive.slice("file:".length);
      if (!session || !session.files[path]) setCenterActive("agent");
      return;
    }
    if (centerActive.startsWith("diff:")) {
      if (!diffTabs.some((tab) => tab.id === centerActive)) setCenterActive("agent");
      return;
    }
    // Page tabs (tasks, plan-work, git-history, …) — validate
    // against the per-thread page-tab list. Reset to agent only when
    // the page wasn't restored. The previous unknown-id fall-through
    // unconditionally reset every page id, clobbering the user's
    // first click after startup.
    const pageTabs = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
    if (!pageTabs.some((ref) => ref.id === centerActive)) setCenterActive("agent");
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
    void getThreadWorkState(stream.id, selectedThread.id)
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
      missing.map(async (thread) => [thread.id, await getThreadWorkState(stream.id, thread.id)] as const),
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
  useBackendSubscriptions({
    threadStatesRef,
    setWorkspaceContext,
    setBacklogState,
    setThreadWorkStates,
    setThreadStates,
    setStreams,
    setStream,
    setAgentStatuses,
    setGeneratedState,
  });

  useEffect(() => {
    for (const [streamId, state] of Object.entries(threadStates)) {
      for (const thread of state.threads) {
        if (threadWorkStates[thread.id]) continue;
        void getThreadWorkState(streamId, thread.id)
          .then((work) => setThreadWorkStates((prev) => (prev[thread.id] ? prev : { ...prev, [thread.id]: work })))
          .catch((error) => logUi("warn", "failed to preload thread work state", { streamId, threadId: thread.id, error: String(error) }));
      }
    }
  }, [threadStates]);

  const handleToggleGenerated = async (entry: string, mark: boolean) => {
    const next = mark
      ? Array.from(new Set([...generated, entry])).sort()
      : generated.filter((e) => e !== entry);
    try {
      const cfg = await setGenerated(next);
      setGeneratedState(cfg.generated);
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
  // Agent-terminal transport — lifted from TerminalPane so the Agent
  // tab's right-click menu can toggle between direct stdin and tmux.
  // Reset to direct when the active thread changes (the old TerminalPane
  // had this behavior via a useEffect on paneTarget).
  const [agentTransportMode, setAgentTransportMode] = useState<"direct" | "tmux">("direct");
  const [planEditRequest, setPlanEditRequest] = useState<{ itemId: number; token: number } | null>(null);
  // Imperative shortcut for opening the New-Task modal. When PlanPane is
  // mounted it registers its openCreateModal here; the menu handler can
  // call this ref directly instead of going through setState + useEffect.
  // Needed because menu clicks arrive as IPC messages (not "discrete user
  // input events"), so React doesn't auto-flush effects for them — the
  // useEffect chain can stall for 10+ seconds before committing. Direct
  // ref call inside flushSync sidesteps the scheduler entirely.
  const planOpenCreateRef = useRef<(() => void) | null>(null);
  // Forward ref so commandHandlers (declared above handleOpenPage) can
  // route through the same page-tab opener used by every other caller.
  // The ref is populated in a useEffect after handleOpenPage is defined.
  const handleOpenPageRef = useRef<((ref: TabRef) => void) | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const commandState = useMemo(
    () => ({
      hasStream: !!stream,
      hasSelectedFile: !!selectedFilePath,
      canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
      hasThread: !!selectedThread,
      canCommit: !!stream && !!workspaceContext.gitEnabled,
    } as const),
    [currentFile, currentFileDirty, selectedThread, selectedFilePath, stream, workspaceContext.gitEnabled],
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
      const { awaitDone } = await action();
      const task = await awaitDone;
      const result = task?.result as GitOpResult | undefined;
      if (result && result.success) return;
      const errorId = recordOpError({
        label,
        command,
        stderr: result?.stderr ?? task?.error ?? "",
        stdout: result?.stdout ?? "",
        exitCode: result?.status ?? null,
        blankFailure: !result || (!result.stderr && !result.stdout && result.status == null),
      });
      showToast({
        message: `${label} failed`,
        actionLabel: "Show details",
        onUndo: () => handleOpenPageRef.current?.(opErrorRef(errorId)),
      });
    },
    [],
  );
  const commandHandlers = useMemo(() => ({
    save() {
      void handleEditorSave();
    },
    quickOpen() {
      if (!stream) return;
      setQuickOpenVisible(true);
    },
    find() {
      if (!selectedFilePath) return;
      setCenterActive(`file:${selectedFilePath}`);
      setEditorFindRequest((current) => current + 1);
    },
    showFiles() {
      handleOpenPageRef.current?.(indexRef("files"));
    },
    showUncommitted() {
      handleOpenPageRef.current?.(uncommittedChangesRef());
    },
    showComments() {
      handleOpenPageRef.current?.(commentsRef());
    },
    showGit() {
      handleOpenPageRef.current?.(gitDashboardRef());
    },
    showTasks() {
      handleOpenPageRef.current?.(indexRef("tasks"));
    },
    showWiki() {
      handleOpenPageRef.current?.(indexRef("wiki-index"));
    },
    newTask() {
      // handleOpenPage is declared further down; forward through the ref
      // so the menu/keyboard handler routes to a NewTaskPage tab
      // (replaces the legacy openCreateModal-via-PlanPane path).
      handleOpenPageRef.current?.(newTaskRef());
    },
    newStream() {
      handleOpenPageRef.current?.(newStreamRef());
    },
    newThread() {
      if (!stream) return;
      setThreadCreateRequest((n) => n + 1);
    },
    openHistory() {
      handleOpenPageRef.current?.(indexRef("git-history"));
    },
    commitFiles() {
      if (!stream || !workspaceContext.gitEnabled) return;
      handleOpenPageRef.current?.(indexRef("files"));
      setCommitFilesRequest((n) => n + 1);
    },
    pullChanges() {
      if (!stream || !workspaceContext.gitEnabled) return;
      void runGitMenuOp("Pull", "git pull", () => gitPull(stream.id));
    },
    pushChanges() {
      if (!stream || !workspaceContext.gitEnabled) return;
      void runGitMenuOp("Push", "git push", () => gitPush(stream.id));
    },
    openProject() {
      void pickAndOpenProject(false);
    },
    openProjectNewWindow() {
      void pickAndOpenProject(true);
    },
  }), [stream, selectedFilePath, workspaceContext.gitEnabled, runGitMenuOp]);
  const [recentProjects, setRecentProjects] = useState<RecentProjectView[]>([]);
  useEffect(() => {
    listRecentProjects()
      .then(setRecentProjects)
      .catch((e) => logUi("warn", "failed to load recent projects for menu", { error: String(e) }));
  }, []);
  // Cancel the native WKWebView context menu everywhere except inputs,
  // contenteditable, Monaco, and the terminal (see context-menu.ts).
  useEffect(() => installContextMenuSuppressor(), []);
  const menuGroupSnapshots = useMemo(() => buildMenuGroupSnapshots(commandState), [commandState]);
  const nativeMenuSnapshots = useMemo(
    () => buildNativeMenuSnapshots(commandState, recentProjects),
    [commandState, recentProjects],
  );
  const menuGroups = useMemo(
    () => buildMenuGroups(commandState, commandHandlers),
    [commandState, commandHandlers],
  );
  const commandMap = useMemo(
    () => new Map(menuGroups.flatMap((group) => group.items.map((item) => [item.id, item] as const))),
    [menuGroups],
  );

  useEffect(() => {
    // Palette shortcut lives OUTSIDE the menu system (no associated
    // CommandId) so it works in both Electron and browser modes identically.
    // Native-menu accelerators can't intercept Cmd+K because there's no menu
    // item for it — keeping the shortcut here means Electron and browser
    // users get the same behaviour without a round-trip through main.ts.
    function handlePaletteShortcut(event: KeyboardEvent) {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      if (event.key.toLowerCase() !== "k") return;
      event.preventDefault();
      // stopImmediatePropagation so Monaco's own keydown listener doesn't also
      // see Cmd+K (it otherwise runs its default "trigger editor command"
      // keybinding flow and eats the event before the bubble-phase handler).
      event.stopImmediatePropagation();
      setPaletteOpen((prev) => !prev);
    }
    // capture:true so the shortcut fires during the capture phase, BEFORE any
    // focused descendant (Monaco, a textarea, a <select>) can call
    // stopPropagation or call preventDefault on its own Cmd+K handling.
    window.addEventListener("keydown", handlePaletteShortcut, { capture: true });
    return () => window.removeEventListener("keydown", handlePaletteShortcut, { capture: true } as EventListenerOptions);
  }, []);

  useEffect(() => {
    // Site-wide search overlay: Cmd/Ctrl+Shift+F. Capture-phase +
    // stopImmediatePropagation for the same reason as the Cmd+K palette —
    // so Monaco's own find-in-files binding doesn't eat the event first.
    function handleSearchShortcut(event: KeyboardEvent) {
      if (!(event.metaKey || event.ctrlKey) || !event.shiftKey || event.altKey) return;
      if (event.key.toLowerCase() !== "f") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      setSearchOpen((prev) => !prev);
    }
    window.addEventListener("keydown", handleSearchShortcut, { capture: true });
    return () =>
      window.removeEventListener("keydown", handleSearchShortcut, {
        capture: true,
      } as EventListenerOptions);
  }, []);

  useEffect(() => {
    // Runs in both Electron and browser modes. In Electron the native
    // menu's accelerator should also fire for the same command, but the
    // handler is idempotent (commandMap.run() → modal setters are no-ops
    // when the modal is already open) so a double-dispatch is harmless
    // — and not relying on the native menu means Cmd+Shift+N works even
    // when the menu snapshot is momentarily stale at startup.
    function handleKeyDown(event: KeyboardEvent) {
      const commandId = getCommandIdForShortcut(event);
      if (!commandId) return;
      // Only "plan.newTask" suppresses itself inside a text input — the
      // rest (save, find, quick-open) are explicitly useful while editing.
      // Rationale: a user in the middle of typing a description shouldn't
      // lose focus to a New-Task modal and drop their half-typed text.
      if (commandId === "plan.newTask" && isEditableTarget(event.target)) return;
      const command = commandMap.get(commandId);
      if (!command || !command.enabled || !command.run) return;
      event.preventDefault();
      command.run();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [commandMap]);

  useEffect(() => {
    void desktopBridge().setNativeMenu(nativeMenuSnapshots).catch((error) => {
      logUi("error", "failed to update native menu", { error: String(error) });
    });
  }, [nativeMenuSnapshots]);

  useEffect(() => {
    return desktopBridge().onMenuCommand((commandId: string) => {
      // Dynamic "Open Recent ▸ <project>" entries aren't in commandMap;
      // open the trailing path in a new window.
      if (commandId.startsWith(OPEN_RECENT_PREFIX)) {
        const path = commandId.slice(OPEN_RECENT_PREFIX.length);
        void openProjectGuarded(path, true);
        return;
      }
      const command = commandMap.get(commandId as never);
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
      flushSync(() => { run(); });
    });
  }, [commandMap]);

  const pageTabsForActiveThread = selectedThreadId ? threadPageTabs[selectedThreadId] ?? [] : [];
  const availableCenterIds = useMemo(() => {
    const ids = new Set(["agent"]);
    for (const path of currentSession.openOrder) ids.add(`file:${path}`);
    for (const tab of diffTabs) ids.add(tab.id);
    for (const ref of pageTabsForActiveThread) ids.add(ref.id);
    return ids;
  }, [currentSession.openOrder, diffTabs, pageTabsForActiveThread]);
  const effectiveCenterActive = availableCenterIds.has(centerActive) ? centerActive : "agent";

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
    // chrome as every other page kind. The legacy `diffTabs` array
    // is just a spec registry indexed by id.
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
      // satisfy the `FileVersion` shape; DISK is a harmless sentinel
      // here.
      leftVersion: DISK,
      rightVersion: DISK,
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

  const handleRequestEditTask = (itemId: number) => {
    const token = Date.now();
    handleOpenPage(indexRef("tasks"));
    setPlanEditRequest({ itemId, token });
    void recordUsage({
      kind: "task",
      key: itemId,
      event: "open",
      streamId: stream?.id ?? null,
      threadId: selectedThread?.id ?? null,
    }).catch(() => {});
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
      if (id.startsWith("file:")) orderedFiles.push(id.slice("file:".length));
      else if (id.startsWith("diff:")) orderedDiffIds.push(id);
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
      const orderedNonAgent = orderedIds.filter((id) => id !== "agent");
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

  const bookmarksStore = useBookmarksStore();

  const recentFileEntries = useMemo(() => {
    const order = currentSession.openOrder;
    return order.map((path, idx) => ({ path, touchedAt: order.length - idx }));
  }, [currentSession.openOrder]);

  // Recently-finished work merged across closed tasks efforts
  // (per-thread) and updated wiki notes (global). Refetched on
  // tasks or wiki-page changes; sub-100ms IPC, so coarse
  // invalidation is fine.
  const [uncommittedSummary, setUncommittedSummary] = useState<{
    added: number; modified: number; deleted: number; additions: number; deletions: number;
    conflictedCount: number; gitOperation: "merge" | "rebase" | "cherry-pick" | "revert" | null;
  } | null>(null);
  useEffect(() => {
    const sid = stream?.id;
    if (!sid) { setUncommittedSummary(null); return; }
    let cancelled = false;
    const refresh = () => {
      void Promise.all([getBranchChanges(sid, "HEAD"), getRepoConflictState(sid)])
        .then(([res, conflict]) => {
          if (cancelled) return;
          let added = 0, modified = 0, deleted = 0, additions = 0, deletions = 0;
          for (const f of res.files) {
            if (f.status === "added" || f.status === "untracked") added++;
            else if (f.status === "modified" || f.status === "renamed") modified++;
            else if (f.status === "deleted") deleted++;
            additions += f.additions ?? 0;
            deletions += f.deletions ?? 0;
          }
          setUncommittedSummary({
            added, modified, deleted, additions, deletions,
            conflictedCount: conflict.conflictedCount,
            gitOperation: conflict.operation,
          });
        })
        .catch(() => { if (!cancelled) setUncommittedSummary(null); });
    };
    refresh();
    const offGit = subscribeGitRefsEvents(sid, () => refresh());
    const offWs = subscribeWorkspaceEvents(sid, () => refresh());
    return () => { cancelled = true; offGit(); offWs(); };
  }, [stream?.id]);

  const [recentlyFinished, setRecentlyFinished] = useState<FinishedEntry[]>([]);
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listRecentlyFinished(selectedThreadId, 5)
        .then((entries) => { if (!cancelled) setRecentlyFinished(entries); })
        .catch(() => { /* ignore — empty list keeps the section hidden */ });
    };
    refresh();
    const offWork = subscribeTaskEvents("all", () => refresh());
    const offNotes = subscribeWikiPageEvents(() => refresh());
    return () => {
      cancelled = true;
      offWork();
      offNotes();
    };
  }, [selectedThreadId]);

  const handleOpenPage = useCallback((ref: TabRef) => {
    // Page-visit recording lives in the central activation effect
    // below — it fires whenever `effectiveCenterActive` resolves to a
    // new TabRef, regardless of which handler caused the activation.
    switch (ref.kind) {
      case "agent":
        setCenterActive("agent");
        return;
      case "file": {
        const payload = ref.payload as { path?: string; version?: import("./file-version.js").FileVersion } | null;
        if (!payload?.path) return;
        const version = payload.version ?? DISK;
        if (version.kind === "disk") {
          void handleOpenFile(payload.path);
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
      case "wiki":
      case "wiki-freshness":
      case "directory":
      case "task":
      case "finding":
      case "dashboard":
      case "settings":
      case "code-quality":
      case "local-history":
      case "git-history":
      case "git-dashboard":
      case "git-commit":
      case "snapshot":
      case "uncommitted-changes":
      case "hook-events":
      case "terminal":
      case "files":
      case "wiki-index":
      case "comments":
      case "tasks":
      case "done-work":
      case "backlog":
      case "archived":
      case "subsystem-docs":
      case "stream-settings":
      case "thread-settings":
      case "new-stream":
      case "new-task":
      case "closed-threads":
      case "external-url":
      case "op-error": {
        // Open as a per-thread page tab.
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
      default:
        return;
    }
  }, [handleOpenFile, handleOpenWiki, selectedThreadId, selectedThreadWork, setCenterActive, stream?.id]);

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
      const payload = ref.payload as { path?: string; version?: import("./file-version.js").FileVersion } | null;
      // Only the disk version needs to populate the fileSessions
      // dirty-state cache + LSP wiring. Non-disk versions render
      // through FileViewerPage which loads content directly via
      // readFile(version) — calling handleOpenFile here would
      // pollute the cache with disk content the user didn't ask
      // for.
      const isDisk = !payload?.version || payload.version.kind === "disk";
      if (payload?.path && isDisk) void handleOpenFile(payload.path);
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
      const payload = target.payload as { path?: string; version?: import("./file-version.js").FileVersion } | null;
      const isDisk = !payload?.version || payload.version.kind === "disk";
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
    if (id.startsWith("file:") && stream) {
      const path = id.slice("file:".length);
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
    setCenterActive((current) => (current === id ? "agent" : current));
    // GC the per-page snapshot so closed tabs don't leak forever.
    if (selectedThreadId) {
      clearPageSnapshot(`${selectedThreadId}::${id}`);
    }
  }, [selectedThreadId, setCenterActive, stream]);

  // Keep the forward ref in sync with the latest handleOpenPage. Used by
  // commandHandlers (declared above handleOpenPage) so menu/keyboard
  // dispatches route through the same page-tab opener.
  useEffect(() => {
    handleOpenPageRef.current = handleOpenPage;
  }, [handleOpenPage]);

  const centerTabs: CenterTab[] = useMemo(() => {
    const tabs: CenterTab[] = [
      {
        id: "agent",
        label: "Agent",
        closable: false,
        agentStatus: agentThreadStatus,
        contextMenu: selectedThread ? [
          {
            id: "agent.transport.toggle",
            label: agentTransportMode === "direct" ? "Open in tmux" : "Use direct mode",
            enabled: true,
            run: () => setAgentTransportMode((prev) => prev === "direct" ? "tmux" : "direct"),
          },
        ] : undefined,
        render: () => (
          <AgentPage
            thread={selectedThread}
            stream={stream}
            visible={effectiveCenterActive === "agent"}
            transportMode={agentTransportMode}
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
    const stripVisibleIds = new Set<string>(["agent"]);
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
    const renderedTabIds = new Set<string>(["agent"]);
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
      for (const ref of slotStack) {
      // Global dedup across all slots — never push two tabs with
      // the same id, even if multiple slots' stacks happen to
      // contain it.
      if (renderedTabIds.has(ref.id)) continue;
      renderedTabIds.add(ref.id);
      if (ref.kind === "diff") {
        // Diff that arrived via in-tab navigation. Look up the
        // registered spec; skip if missing (the registration path is
        // handleOpenDiffInTab — a stale ref without a spec would be
        // a bug).
        const spec = diffTabs.find((t) => t.id === ref.id)?.spec;
        if (!spec) continue;
        const label = spec.path.split("/").pop() ?? spec.path;
        const suffix = spec.labelOverride ?? "diff";
        tabs.push({
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
                navOpenFile(p);
              }}
            />
          ) : null,
        });
        continue;
      }
      if (ref.kind === "duplicate-block") {
        const payload = ref.payload as import("./tabs/pageRefs.js").DuplicateBlockPayload | null;
        if (!payload) continue;
        const leftBase = payload.leftPath.split("/").pop() ?? payload.leftPath;
        const rightBase = payload.rightPath.split("/").pop() ?? payload.rightPath;
        tabs.push({
          id: ref.id,
          label: `${leftBase} ↔ ${rightBase}`,
          closable: true,
          render: () => stream ? (
            <DuplicateBlockPage
              stream={stream}
              payload={payload}
              visible={effectiveCenterActive === ref.id}
              onJumpToSource={(p, v) => {
                handleNavigateInTab(slotRef.id, fileRef(p, v));
              }}
            />
          ) : null,
        });
        continue;
      }
      if (ref.kind === "file") {
        const payload = ref.payload as { path?: string; version?: import("./file-version.js").FileVersion } | null;
        const path = payload?.path;
        if (!path) continue;
        const version = payload?.version ?? DISK;
        const basename = path.split("/").pop() ?? path;
        // Non-disk versions render through FileViewerPage — read-only,
        // no dirty state, no save plumbing. The EditorPane / save
        // pipeline stays disk-only on purpose so the dirty cache,
        // LSP, find-in-file, etc. don't have to grow "is this read
        // only?" branches.
        if (version.kind !== "disk") {
          const versionLabel =
            version.kind === "ref"
              ? version.ref.length > 12
                ? version.ref.slice(0, 7)
                : version.ref
              : `snap:${version.id.slice(0, 7)}`;
          tabs.push({
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
          });
          continue;
        }
        const file = currentSession.files[path];
        const dirty = !!file && file.draftContent !== file.savedContent;
        tabs.push({
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
              onRevealTask={handleRequestEditTask}
              onCompareWithClipboard={handleCompareWithClipboard}
            />
          ) : null,
        });
      } else if (ref.kind === "settings") {
        tabs.push({
          id: ref.id,
          label: "Settings",
          closable: true,
          render: () => <SettingsPage onClose={() => closePageTab(ref.id)} />,
        });
      } else if (ref.kind === "code-quality") {
        tabs.push({
          id: ref.id,
          label: "Code Quality",
          closable: true,
          render: () => <CodeQualityPage stream={stream} onOpenFile={navOpenFile} />,
        });
      } else if (ref.kind === "local-history") {
        tabs.push({
          id: ref.id,
          label: "Local History",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={navOpen}
            />
          ),
        });
      } else if (ref.kind === "local-history-full") {
        tabs.push({
          id: ref.id,
          label: "All snapshots",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={navOpen}
              mode="full-list"
            />
          ),
        });
      } else if (ref.kind === "local-history-by-commit-full") {
        tabs.push({
          id: ref.id,
          label: "All commits",
          closable: true,
          render: () => (
            <LocalHistoryDashboardPage
              stream={stream}
              onOpenPage={navOpen}
              mode="full-by-commit"
            />
          ),
        });
      } else if (ref.kind === "snapshot") {
        const payload = (ref.payload as { snapshotId?: number } | null) ?? null;
        const snapshotId = payload?.snapshotId ?? 0;
        tabs.push({
          id: ref.id,
          label: `Snapshot ${snapshotId}`,
          closable: true,
          render: () => (
            <SnapshotDetailPage
              stream={stream}
              snapshotId={snapshotId}
              onOpenDiff={navOpenDiff}
              onOpenDiffInTab={navOpenDiff}
              onOpenPage={navOpen}
              onOpenFile={navOpenFile}
            />
          ),
        });
      } else if (ref.kind === "git-history") {
        tabs.push({
          id: ref.id,
          label: "Git History",
          closable: true,
          render: () => (
            <GitHistoryPage stream={stream} onOpenPage={navOpen} />
          ),
        });
      } else if (ref.kind === "git-dashboard") {
        tabs.push({
          id: ref.id,
          label: "Git Dashboard",
          closable: true,
          render: () => (
            <GitDashboardPage
              stream={stream}
              onOpenPage={navOpen}
              onRevealCommit={navRevealCommit}
            />
          ),
        });
      } else if (ref.kind === "uncommitted-changes") {
        const payload = (ref.payload as {
          scope?: { kind: "ext" | "dir" | "status"; value: string };
        } | null) ?? null;
        const scope = payload?.scope ?? undefined;
        tabs.push({
          id: ref.id,
          label: scope ? `Uncommitted — ${scope.value}` : "Uncommitted",
          closable: true,
          render: () => (
            <UncommittedChangesPage
              stream={stream}
              scope={scope}
              onOpenPage={navOpen}
              onOpenFile={navOpenFile}
              onOpenDiff={navOpenDiff}
              onOpenDiffInTab={navOpenDiff}
            />
          ),
        });
      } else if (ref.kind === "git-commit") {
        const payload = (ref.payload as {
          sha?: string;
          scope?: { kind: "ext" | "dir" | "status"; value: string };
        } | null) ?? null;
        const sha = payload?.sha ?? "";
        const scope = payload?.scope ?? undefined;
        const shaLabel = sha ? sha.slice(0, 7) : "commit";
        tabs.push({
          id: ref.id,
          label: scope ? `${shaLabel} — ${scope.value}` : shaLabel,
          closable: true,
          render: () => (
            <GitCommitPage
              stream={stream}
              sha={sha}
              scope={scope}
              threadWork={selectedThreadWork}
              onOpenDiff={navOpenDiff}
              onOpenDiffInTab={navOpenDiff}
              onOpenPage={navOpen}
              onOpenFile={navOpenFile}
            />
          ),
        });
      } else if (ref.kind === "hook-events") {
        tabs.push({
          id: ref.id,
          label: "Hook Events",
          closable: true,
          render: () => <HookEventsPage streamId={stream?.id ?? null} />,
        });
      } else if (ref.kind === "terminal") {
        tabs.push({
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
        });
      } else if (ref.kind === "op-error") {
        const errorId = (ref.payload as { errorId?: string } | null)?.errorId ?? "";
        tabs.push({
          id: ref.id,
          label: "Op Error",
          closable: true,
          render: () => <OpErrorPage errorId={errorId} />,
        });
      } else if (ref.kind === "files") {
        tabs.push({
          id: ref.id,
          label: "Files",
          closable: true,
          render: () => (
            <FilesPage
              stream={stream}
              gitEnabled={workspaceContext.gitEnabled}
              selectedFilePath={selectedFilePath}
              generated={generated}
              onOpenFile={navOpenFile}
              onOpenDiff={navOpenDiff}
              onCreateFile={handleCreateFile}
              onCreateDirectory={handleCreateDirectory}
              onRenamePath={handleRenamePath}
              onDeletePath={handleDeletePath}
              onToggleGenerated={handleToggleGenerated}
              commitRequest={commitFilesRequest}
            />
          ),
        });
      } else if (ref.kind === "wiki-index") {
        tabs.push({
          id: ref.id,
          label: "Wiki",
          closable: true,
          render: () => (
            <WikiIndexPage
              stream={stream}
              selectedSlug={centerActive.startsWith("wiki:") ? centerActive.slice("wiki:".length) : null}
              onOpenWikiPage={handleOpenWiki}
            />
          ),
        });
      } else if (ref.kind === "comments") {
        tabs.push({
          id: ref.id,
          label: "Comments Dashboard",
          closable: true,
          render: () => <CommentsInboxPage stream={stream} onOpenPage={navOpen} />,
        });
      } else if (
        ref.kind === "tasks"
        || ref.kind === "done-work"
        || ref.kind === "backlog"
        || ref.kind === "archived"
      ) {
        const sharedProps = {
          thread: selectedThread,
          activeThreadId: currentThreadState.activeThreadId,
          threadWork: selectedThreadWork,
          agentStatus: agentThreadStatus,
          backlog: backlogState,
          onUpdateTask: handleUpdateTask,
          onDeleteTask: handleDeleteTask,
          onReorderTasks: handleReorderTasks,
          onUpdateBacklogItem: handleUpdateBacklogItem,
          onDeleteBacklogItem: handleDeleteBacklogItem,
          onReorderBacklog: handleReorderBacklog,
          onMoveItemToBacklog: handleMoveItemToBacklog,
          editRequest: planEditRequest,
          registerOpenCreate: (fn: () => void) => { planOpenCreateRef.current = fn; },
          onOpenNewTaskPage: (payload: { parentId?: number | null }) =>
            navOpen(newTaskRef(payload)),
          onOpenTaskPage: (itemId: number) => navOpen(taskRef(itemId)),
        };
        const labelByKind: Record<string, string> = {
          "tasks": "Tasks",
          "done-work": "Done Work",
          "backlog": "Backlog",
          "archived": "Archived",
        };
        tabs.push({
          id: ref.id,
          label: labelByKind[ref.kind] ?? ref.kind,
          closable: true,
          render: () => {
            switch (ref.kind) {
              case "tasks":
                return <TasksPage {...sharedProps} streams={streams} currentStreamId={stream?.id ?? null} onOpenPage={navOpen} />;
              case "done-work":
                return <DoneWorkPage {...sharedProps} onOpenPage={navOpen} />;
              case "backlog":
                return <BacklogPage {...sharedProps} />;
              case "archived":
                return <ArchivedPage {...sharedProps} />;
              default:
                return null;
            }
          },
        });
      } else if (ref.kind === "subsystem-docs") {
        tabs.push({
          id: ref.id,
          label: "Subsystem Docs",
          closable: true,
          render: () => <SubsystemDocsPage stream={stream} onOpenPage={navOpen} />,
        });
      } else if (ref.kind === "closed-threads") {
        tabs.push({
          id: ref.id,
          label: "Closed Threads",
          closable: true,
          render: () => <ClosedThreadsPage stream={stream} />,
        });
      } else if (ref.kind === "external-url") {
        const externalUrl = (ref.payload as { url?: string } | null)?.url ?? "";
        let label = externalUrl;
        try {
          const u = new URL(externalUrl);
          label = u.host + (u.pathname && u.pathname !== "/" ? u.pathname : "");
        } catch { /* keep raw */ }
        tabs.push({
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
        });
      } else if (ref.kind === "wiki") {
        const slug = (ref.payload as { slug?: string } | null)?.slug ?? "";
        const wikiNavOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        tabs.push({
          id: ref.id,
          label: slug,
          closable: true,
          render: () => stream ? (
            <WikiPage
              stream={stream}
              slug={slug}
              threadWork={selectedThreadWork}
              onClosed={() => closePageTab(ref.id)}
              onOpenWikiPage={handleOpenWiki}
              onOpenFile={navOpenFile}
              onOpenDirectory={handleOpenDirectory}
              onOpenPage={wikiNavOpen}
              onOpenCommit={handleOpenCommit}
              onOpenExternalUrl={handleOpenExternalUrl}
            />
          ) : null,
        });
      } else if (ref.kind === "wiki-freshness") {
        const slug = (ref.payload as { slug?: string } | null)?.slug ?? "";
        const navOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        tabs.push({
          id: ref.id,
          label: `Freshness — ${slug}`,
          closable: true,
          render: () => <WikiFreshnessPage slug={slug} onOpenPage={navOpen} />,
        });
      } else if (ref.kind === "directory") {
        const dirPath = (ref.payload as { path?: string } | null)?.path ?? "";
        const dirNavOpen = (newRef: TabRef) => handleNavigateInTab(ref.id, newRef);
        tabs.push({
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
        });
      } else if (ref.kind === "task") {
        const itemId = (ref.payload as { itemId?: number } | null)?.itemId ?? 0;
        // ThreadWorkState splits items by status (Ready→items, InProgress→inProgress,
        // Done/Canceled/Archived→done, Blocked→waiting, Epics→epics). Merge them all
        // for the lookup so TaskPage can resolve any item on this thread, not
        // just Ready ones — otherwise clicking a done/in-progress item renders the
        // misleading "not loaded in the current thread" fallback.
        const items = selectedThreadWork
          ? [
              ...selectedThreadWork.inProgress,
              ...selectedThreadWork.items,
              ...selectedThreadWork.waiting,
              ...selectedThreadWork.done,
              ...selectedThreadWork.epics,
            ]
          : [];
        const matching = items.find((i) => i.id === itemId);
        tabs.push({
          id: ref.id,
          label: matching ? matching.title : `task:${itemId}`,
          closable: true,
          render: () => (
            <TaskPage
              stream={stream}
              thread={selectedThread}
              itemId={itemId}
              items={items}
              threadWork={selectedThreadWork}
              onOpenPage={navOpen}
              onOpenFile={(p) => navOpenFile(p)}
              onShowInHistory={(snapshotId) => {
                const id = Number(snapshotId);
                if (Number.isFinite(id)) navOpen(snapshotRef(id));
              }}
            />
          ),
        });
      } else if (ref.kind === "finding") {
        const findingId = (ref.payload as { findingId?: string } | null)?.findingId ?? "";
        tabs.push({
          id: ref.id,
          label: `Finding ${findingId}`,
          closable: true,
          render: () => (
            <FindingPage
              stream={stream}
              findingId={findingId}
              threadWork={selectedThreadWork}
              onOpenPage={navOpen}
              onOpenFileAtLine={(p) => { navOpenFile(p); }}
            />
          ),
        });
      } else if (ref.kind === "stream-settings") {
        const targetStreamId = (ref.payload as { streamId?: string } | null)?.streamId ?? "";
        const targetStream = streams.find((s) => s.id === targetStreamId) ?? null;
        tabs.push({
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
        });
      } else if (ref.kind === "thread-settings") {
        const targetThreadId = (ref.payload as { threadId?: string } | null)?.threadId ?? "";
        const targetThread = currentThreadState.threads.find((t) => t.id === targetThreadId) ?? null;
        tabs.push({
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
        });
      } else if (ref.kind === "new-stream") {
        tabs.push({
          id: ref.id,
          label: "New Stream",
          closable: true,
          render: () => (
            <NewStreamPage
              gitEnabled={workspaceContext.gitEnabled}
              defaultTitle={`Stream ${streams.length + 1}`}
              onClose={() => closePageTab(ref.id)}
              onCreated={(created) => {
                handleStreamCreated(created);
                closePageTab(ref.id);
              }}
            />
          ),
        });
      } else if (ref.kind === "new-task") {
        const payload = (ref.payload as {
          parentId?: number | null;
          initialCategory?: string | null;
          initialPriority?: string | null;
        } | null) ?? {};
        tabs.push({
          id: ref.id,
          label: "New task",
          closable: true,
          render: () => (
            <NewTaskPage
              defaults={{
                parentId: payload.parentId ?? null,
                initialCategory: payload.initialCategory ?? null,
                initialPriority: payload.initialPriority ?? null,
              }}
              epics={selectedThreadWork?.epics ?? []}
              onClose={() => closePageTab(ref.id)}
              onSubmit={async (input) => {
                await handleCreateTask({
                  title: input.title,
                  description: input.description,
                  parentId: input.parentId ?? null,
                  status: input.status ?? "ready",
                  priority: input.priority ?? "medium",
                });
              }}
            />
          ),
        });
      } else if (ref.kind === "dashboard") {
        const variant = (ref.payload as { variant?: "planning" | "review" | "quality" | "visits" } | null)?.variant ?? "planning";
        tabs.push({
          id: ref.id,
          label: `${variant.charAt(0).toUpperCase()}${variant.slice(1)}`,
          closable: true,
          render: () => (
            <DashboardPage
              variant={variant}
              stream={stream}
              threadWork={selectedThreadWork}
              backlog={backlogState}
              onOpenPage={navOpen}
            />
          ),
        });
      }
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
      const scopes = ref ? bookmarksStore.scopesFor(selectedThreadId, stream?.id ?? null, ref.id) : [];
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
        bookmark: ref ? {
          scopes,
          toggle: (scope: BookmarkScope) => {
            const currentScopes = bookmarksStore.scopesFor(selectedThreadId, stream?.id ?? null, ref.id);
            if (currentScopes.includes(scope)) {
              bookmarksStore.remove(scope, selectedThreadId, stream?.id ?? null, ref.id);
            } else {
              bookmarksStore.add(scope, selectedThreadId, stream?.id ?? null, ref, registeredTitle ?? tab.label);
            }
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
    agentTransportMode,
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
    bookmarksStore,
    workspaceContext.gitEnabled,
    selectedFilePath,
    generated,
    commitFilesRequest,
    centerActive,
    currentThreadState.activeThreadId,
    selectedThreadWork,
    backlogState,
    planEditRequest,
  ]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden" }}>
      <div style={{ borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        {!isMac ? <Menubar groups={menuGroups} /> : null}
        {error ? (
          <div style={{ padding: "2px 12px", background: "var(--bg-2)", color: "#ff6b6b", fontSize: 11, minHeight: 22, borderBottom: "1px solid var(--border)" }}>{error}</div>
        ) : null}
      </div>
      <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <Navigator
          streams={streams}
          currentStreamId={stream?.id ?? null}
          threadStates={threadStates}
          streamStatuses={streamStatuses}
          agentStatuses={agentStatuses}
          onSwitchStream={handleSwitch}
          onSelectThread={handleSelectThread}
          onCreateThread={async (streamId, title) => {
            if (streamId !== stream?.id) await handleSwitch(streamId);
            await handleCreateThread(title);
          }}
          onOpenNewStreamPage={() => handleOpenPage(newStreamRef())}
          onRenameStream={handleRenameStreamById}
          onRenameThread={handleRenameThreadById}
          onPromoteThread={handlePromoteThread}
          onCloseThread={handleCloseThread}
          onOpenStreamSettings={(streamId) => handleOpenPage(streamSettingsRef(streamId))}
          onOpenThreadSettings={(threadId) => handleOpenPage(threadSettingsRef(threadId))}
          gitEnabled={workspaceContext.gitEnabled}
        />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0 }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <RailHud
          threadId={selectedThread?.id ?? null}
          streamId={stream?.id ?? null}
          threadWork={selectedThreadWork}
          backlog={backlogState}
          recentFiles={recentFileEntries}
          recentlyFinished={recentlyFinished}
          uncommitted={uncommittedSummary}
          opErrors={opErrors}
          onDismissOpError={(id) => {
            opErrorsStore.dismiss(id);
            void forgetPage("op-error", `op-error:${id}`);
          }}
          onClearOpErrors={() => {
            const ids = opErrorsAll.map((e) => e.id);
            opErrorsStore.clear();
            for (const id of ids) void forgetPage("op-error", `op-error:${id}`);
          }}
          onClearFinished={() => {
            void clearRecentlyFinished(selectedThreadId)
              .then(() => listRecentlyFinished(selectedThreadId, 5))
              .then((entries) => { setRecentlyFinished(entries); })
              .catch(() => {});
          }}
          bookmarks={bookmarksStore.bookmarks(selectedThreadId, stream?.id ?? null).map((b) => {
            const scopeBadge = b.scope === "thread" ? "T" : b.scope === "stream" ? "S" : "G";
            return {
              ref: b.ref,
              label: b.label ?? b.ref.id,
              scopeBadge,
              onRemove: () => bookmarksStore.remove(b.scope, selectedThreadId, stream?.id ?? null, b.ref.id),
            };
          })}
          onOpenPage={handleOpenPage}
          onOpenSearch={() => setQuickOpenVisible(true)}
        />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0, overflow: "hidden" }}>
          {stream ? (
            <CenterTabs
              tabs={centerTabs}
              activeId={effectiveCenterActive}
              onActivate={(id) => {
                if (id.startsWith("file:")) handleSelectOpenFile(id.slice("file:".length));
                else setCenterActive(id);
              }}
              onClose={(id) => {
                if (id.startsWith("file:")) {
                  handleCloseOpenFile(id.slice("file:".length));
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
          justifyContent: "space-between",
          gap: 8,
          padding: "4px 10px",
          borderTop: "1px solid var(--border)",
          background: "var(--bg-2)",
          flexShrink: 0,
          minHeight: 26,
        }}
      >
        {(() => {
          const parts = [stream?.title, selectedThread?.title].filter(Boolean) as string[];
          const text = parts.join(" : ");
          return (
            <span
              data-testid="status-bar-context"
              title={text}
              style={{
                fontSize: "var(--text-xs)",
                color: "var(--text-secondary)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                minWidth: 0,
              }}
            >
              {text}
            </span>
          );
        })()}
        <StatusBar stream={stream} gitEnabled={workspaceContext.gitEnabled} />
      </div>
        </div>
      </div>
      <QuickOpenOverlay
        open={quickOpenVisible}
        stream={stream}
        selectedFilePath={selectedFilePath}
        pages={computePagesDirectory({
          backlogReadyCount: backlogState?.items.filter((i) => i.status === "ready").length ?? 0,
        })}
        onClose={() => setQuickOpenVisible(false)}
        onOpenFile={(path) => {
          void handleOpenFile(path);
        }}
        onOpenPage={(ref) => {
          handleOpenPage(ref);
        }}
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
      {paletteOpen ? (
        <CommandPalette menuGroups={menuGroups} onClose={() => setPaletteOpen(false)} />
      ) : null}
      {searchOpen ? (
        <SearchPalette
          streamId={stream?.id ?? null}
          onClose={() => setSearchOpen(false)}
          onOpen={(hit) => {
            switch (hit.kind) {
              case "task": {
                const id = Number(hit.ref_id);
                if (Number.isFinite(id)) handleOpenPage(taskRef(id));
                break;
              }
              case "wiki":
                handleOpenPage(wikiPageRef(hit.ref_id));
                break;
              case "file":
                void handleOpenFile(hit.ref_id);
                break;
              case "comment":
                handleOpenPage(commentsRef());
                break;
              // notes have no standalone page — surfaced in results only.
            }
          }}
        />
      ) : null}
      <UndoToastStack />
    </div>
  );
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
