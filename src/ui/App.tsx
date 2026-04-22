import { useEffect, useMemo, useRef, useState } from "react";
import {
  createWorkItem,
  completeThread,
  createThread,
  deleteWorkItem,
  getThreadWorkState,
  getThreadState,
  createWorkspaceDirectory,
  listAgentStatuses,
  listAgentTurns,
  listWorkItemEfforts,
  getSnapshotPairDiff,
  reorderWorkItems,
  moveWorkItemToThread,
  getBacklogState,
  createBacklogItem,
  updateBacklogItem,
  deleteBacklogItem,
  reorderBacklog,
  moveWorkItemToBacklog,
  moveBacklogItemToThread,
  subscribeBacklogEvents,
  subscribeAgentStatus,
  subscribeTurnEvents,
  type AgentStatus,
  type AgentTurn,
  createWorkspaceFile,
  deleteWorkspacePath,
  getCurrentStream,
  getWorkspaceContext,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameWorkspacePath,
  renameThread,
  renameStream,
  subscribeNewdeEvents,
  subscribeWorkItemEvents,
  subscribeWorkspaceContext,
  subscribeWorkspaceEvents,
  getConfig,
  setGeneratedDirs,
  selectThread,
  promoteThread,
  reorderThreads,
  reorderStreams,
  switchStream,
  updateWorkItem,
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
  selectOpenFile,
  setLoadedFileContent,
  setOpenFileLoading,
  updateFileDraft,
  type FileSessionState,
} from "../session/file-session.js";
import { buildMenuGroupSnapshots, buildMenuGroups } from "./commands.js";
import { externalFileSyncAction } from "./external-file-sync.js";
import type { EditorNavigationTarget } from "./lsp.js";
import { StreamRail } from "./components/StreamRail.js";
import { SettingsModal } from "./components/SettingsModal.js";
import { ConfirmDialog } from "./components/ConfirmDialog.js";
import { subscribeUiError } from "./ui-error.js";
import { Menubar } from "./components/Menubar.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { DockShell } from "./components/Dock/DockShell.js";
import type { ToolWindow } from "./components/Dock/ToolWindow.js";
import { CenterTabs, type CenterTab } from "./components/CenterTabs/CenterTabs.js";
import { ThreadRail } from "./components/ThreadRail.js";
import { ProjectPanel } from "./components/Panels/ProjectPanel.js";
import { DiffPane, type DiffSpec } from "./components/Diff/DiffPane.js";
import { Activity } from "./components/Activity/Activity.js";
import { PlanPane } from "./components/Plan/PlanPane.js";
import { HistoryPanel } from "./components/History/HistoryPanel.js";
import { SnapshotsPanel } from "./components/Snapshots/SnapshotsPanel.js";
import { TerminalPane } from "./components/TerminalPane.js";
import { EditorPane } from "./components/EditorPane.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { CommandPalette } from "./components/CommandPalette/CommandPalette.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { getCommandIdForShortcut } from "./keybindings.js";
import { logUi } from "./logger.js";

// Cap on concurrent file tabs in the center. Intellij uses ~10 by default;
// when this is exceeded, the oldest-touched tab without unsaved changes is
// closed automatically via enforceOpenFileLimit. Dirty tabs stay pinned.
const MAX_OPEN_FILE_TABS = 10;

// Persists which file tabs are open (per stream) across app restarts. Only the
// paths are saved — dirty state and scroll position are intentionally dropped.
const FILE_SESSIONS_STORAGE_KEY = "newde.layout.v1.fileSessions";
// Persists which center pane was last active ("agent", "file:<path>", or a
// diff tab id). Restored after file sessions are rebuilt; falls back to
// "agent" if the saved id is no longer resolvable (diff tabs never persist,
// and a file tab may have failed to reopen).
const CENTER_ACTIVE_STORAGE_KEY = "newde.layout.v1.centerActive";

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

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [threadStates, setThreadStates] = useState<Record<string, ThreadState>>({});
  const [threadWorkStates, setThreadWorkStates] = useState<Record<string, ThreadWorkState>>({});
  const [backlogState, setBacklogState] = useState<BacklogState | null>(null);
  const [agentTurns, setAgentTurns] = useState<Record<string, AgentTurn[]>>({});
  const [agentStatuses, setAgentStatuses] = useState<Record<string, AgentStatus>>({});
  const [stream, setStream] = useState<Stream | null>(null);
  const [centerActive, setCenterActive] = useState<string>(() => readPersistedCenterActive() ?? "agent");
  const [diffTabs, setDiffTabs] = useState<Array<{ id: string; spec: DiffSpec }>>([]);
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});
  const restoredStreamsRef = useRef<Set<string>>(new Set());
  const centerActiveValidatedRef = useRef(false);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const [pendingDirtyClose, setPendingDirtyClose] = useState<{ path: string; basename: string } | null>(null);
  const [historyReveal, setHistoryReveal] = useState<{ sha: string; token: number } | null>(null);
  const [snapshotsReveal, setSnapshotsReveal] = useState<{ snapshotId: string; token: number } | null>(null);
  const [bottomActivate, setBottomActivate] = useState<{ id: string; token: number } | undefined>(undefined);
  const [streamCreateRequest, setStreamCreateRequest] = useState(0);
  const [threadCreateRequest, setThreadCreateRequest] = useState(0);
  const [commitFilesRequest, setCommitFilesRequest] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [generatedDirs, setGeneratedDirsState] = useState<string[]>([]);
  const daemonDownLogged = useRef(false);
  const daemonProbeState = useRef(INITIAL_DAEMON_PROBE_STATE);
  const isElectron = !!window.newdeDesktop?.isElectron;

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
      const nextThreadState = threadStates[next.id] ?? await getThreadState(next.id);
      const nextThread = nextThreadState.threads.find((thread) => thread.id === nextThreadState.selectedThreadId);
      if (nextThread && !threadWorkStates[nextThread.id]) {
        const nextWork = await getThreadWorkState(next.id, nextThread.id);
        setThreadWorkStates((prev) => ({ ...prev, [nextThread.id]: nextWork }));
      }
      setThreadStates((prev) => ({ ...prev, [next.id]: nextThreadState }));
      setStream(next);
      const nextSession = fileSessions[next.id] ?? createEmptyFileSession();
      setCenterActive(nextSession.selectedPath ? `file:${nextSession.selectedPath}` : "agent");
      setError(null);
      setDaemonUnavailable(false);
      logUi("info", "switched stream", { streamId: next.id, title: next.title });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to switch stream", { streamId: id, error: String(e) });
    }
  }

  async function handleRenameStreamById(streamId: string, currentTitle: string) {
    const nextTitle = window.prompt("Rename stream", currentTitle)?.trim();
    if (!nextTitle || nextTitle === currentTitle) return;
    try {
      const updated = await renameStream(streamId, nextTitle);
      if (stream?.id === updated.id) setStream(updated);
      setStreams((prev) =>
        prev
          .map((candidate) => (candidate.id === updated.id ? updated : candidate))
          .sort((a, b) => a.created_at.localeCompare(b.created_at)),
      );
      setError(null);
    } catch (e) {
      setError(String(e));
    }
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

  function handleStreamCreated(next: Stream) {
    void getThreadState(next.id).then((state) => {
      setThreadStates((prev) => ({ ...prev, [next.id]: state }));
      const thread = state.threads.find((candidate) => candidate.id === state.selectedThreadId);
      if (thread) {
        void getThreadWorkState(next.id, thread.id).then((work) => {
          setThreadWorkStates((prev) => ({ ...prev, [thread.id]: work }));
        });
      }
    }).catch((e) => {
      setError(String(e));
    });
    setStreams((prev) => {
      const others = prev.filter((stream) => stream.id !== next.id);
      return [...others, next].sort((a, b) => a.created_at.localeCompare(b.created_at));
    });
    setStream(next);
    const nextSession = fileSessions[next.id] ?? createEmptyFileSession();
    setCenterActive(nextSession.selectedPath ? `file:${nextSession.selectedPath}` : "agent");
    setError(null);
    setDaemonUnavailable(false);
    logUi("info", "stream created in ui", { streamId: next.id, title: next.title, branch: next.branch });
  }

  async function handleOpenFile(path: string) {
    if (!stream) return;
    const currentSession = fileSessions[stream.id] ?? createEmptyFileSession();
    const existing = currentSession.files[path];
    setFileSessions((prev) => {
      const base = prev[stream.id] ?? createEmptyFileSession();
      const opened = existing
        ? selectOpenFile(base, path)
        : setOpenFileLoading(openFileInSession(base, path, "", true), path, true);
      return { ...prev, [stream.id]: enforceOpenFileLimit(opened, MAX_OPEN_FILE_TABS) };
    });
    setCenterActive(`file:${path}`);
    setError(null);
    if (existing && !existing.isLoading) return;
    try {
      const file = await readWorkspaceFile(stream.id, path);
      setFileSessions((prev) => ({
        ...prev,
        [stream.id]: setLoadedFileContent(prev[stream.id] ?? createEmptyFileSession(), file.path, file.content),
      }));
      logUi("info", "opened file", { streamId: stream.id, path: file.path });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to open file", { streamId: stream.id, path, error: String(e) });
      setFileSessions((prev) => ({
        ...prev,
        [stream.id]: closeOpenFile(prev[stream.id] ?? createEmptyFileSession(), path),
      }));
    }
  }

  async function handleNavigateToLocation(target: EditorNavigationTarget) {
    await handleOpenFile(target.path);
    setEditorNavigationTarget(target);
    setCenterActive(`file:${target.path}`);
  }

  function handleEditorChange(value: string) {
    if (!stream) return;
    const session = fileSessions[stream.id] ?? createEmptyFileSession();
    if (!session.selectedPath) return;
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: updateFileDraft(prev[stream.id] ?? createEmptyFileSession(), session.selectedPath!, value),
    }));
  }

  async function handleEditorSave() {
    if (!stream) return;
    const session = fileSessions[stream.id] ?? createEmptyFileSession();
    const selectedPath = session.selectedPath;
    if (!selectedPath) return;
    const current = session.files[selectedPath];
    if (!current || current.isLoading) return;
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: setOpenFileLoading(prev[stream.id] ?? createEmptyFileSession(), selectedPath, true),
    }));
    try {
      const saved = await writeWorkspaceFile(stream.id, selectedPath, current.draftContent);
      setFileSessions((prev) => ({
        ...prev,
        [stream.id]: markFileSaved(prev[stream.id] ?? createEmptyFileSession(), saved.path, saved.content),
      }));
      setError(null);
      logUi("info", "saved file", { streamId: stream.id, path: saved.path });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to save file", { streamId: stream.id, path: selectedPath, error: String(e) });
      setFileSessions((prev) => ({
        ...prev,
        [stream.id]: setOpenFileLoading(prev[stream.id] ?? createEmptyFileSession(), selectedPath, false),
      }));
    }
  }

  function handleSelectOpenFile(path: string) {
    if (!stream) return;
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: selectOpenFile(prev[stream.id] ?? createEmptyFileSession(), path),
    }));
    setCenterActive(`file:${path}`);
  }

  function handleCloseOpenFile(path: string) {
    if (!stream) return;
    // Guard against silently dropping unsaved edits when a user closes a
    // dirty tab via the × or Cmd+W. Mirrors IntelliJ / VS Code behavior.
    const currentFile = fileSessions[stream.id]?.files[path];
    if (currentFile && currentFile.draftContent !== currentFile.savedContent) {
      const basename = path.split("/").pop() ?? path;
      setPendingDirtyClose({ path, basename });
      return;
    }
    closeOpenFileNow(path);
  }

  function closeOpenFileNow(path: string) {
    if (!stream) return;
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: closeOpenFile(prev[stream.id] ?? createEmptyFileSession(), path),
    }));
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
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: renameOpenFilePaths(prev[stream.id] ?? createEmptyFileSession(), (path) => {
        if (path === renamed.fromPath) return renamed.toPath;
        if (path.startsWith(renamed.fromPath + "/")) {
          return `${renamed.toPath}${path.slice(renamed.fromPath.length)}`;
        }
        return path;
      }),
    }));
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
    setFileSessions((prev) => {
      const current = prev[stream.id] ?? createEmptyFileSession();
      const toRemove = current.openOrder.filter((candidate) => candidate === path || candidate.startsWith(path + "/"));
      return {
        ...prev,
        [stream.id]: removeOpenFiles(current, toRemove),
      };
    });
    setEditorNavigationTarget((current) => {
      if (!current) return current;
      return current.path === path || current.path.startsWith(path + "/") ? null : current;
    });
  }

  async function handleSelectThread(threadId: string) {
    if (!stream) return;
    try {
      const next = await selectThread(stream.id, threadId);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      const thread = next.threads.find((candidate) => candidate.id === threadId);
      if (thread) {
        const work = await getThreadWorkState(stream.id, thread.id);
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

  async function handleCompleteThread(threadId: string) {
    if (!stream) return;
    try {
      const next = await completeThread(stream.id, threadId);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
      setCenterActive("agent");
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

  async function handleCreateWorkItem(input: {
    kind: "epic" | "task" | "subtask" | "bug" | "note";
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    parentId?: string | null;
    status?: "ready" | "in_progress" | "human_check" | "blocked" | "done" | "canceled" | "archived";
    priority?: "low" | "medium" | "high" | "urgent";
  }) {
    if (!stream || !selectedThread) return;
    try {
      const next = await createWorkItem(stream.id, selectedThread.id, input);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleUpdateWorkItem(
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: "ready" | "in_progress" | "human_check" | "blocked" | "done" | "canceled" | "archived";
      priority?: "low" | "medium" | "high" | "urgent";
    },
  ) {
    if (!stream || !selectedThread) return;
    try {
      const next = await updateWorkItem(stream.id, selectedThread.id, itemId, changes);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleDeleteWorkItem(itemId: string) {
    if (!stream || !selectedThread) return;
    try {
      const next = await deleteWorkItem(stream.id, selectedThread.id, itemId);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderWorkItems(orderedItemIds: string[]) {
    if (!stream || !selectedThread) return;
    try {
      const next = await reorderWorkItems(stream.id, selectedThread.id, orderedItemIds);
      setThreadWorkStates((prev) => ({ ...prev, [selectedThread.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveWorkItemToThread(itemId: string, fromThreadId: string, toThreadId: string) {
    if (!stream || fromThreadId === toThreadId) return;
    try {
      const { from, to } = await moveWorkItemToThread(stream.id, fromThreadId, itemId, toThreadId);
      setThreadWorkStates((prev) => ({ ...prev, [fromThreadId]: from, [toThreadId]: to }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveItemToBacklog(itemId: string, fromThreadId: string) {
    if (!stream) return;
    try {
      const { from, backlog } = await moveWorkItemToBacklog(stream.id, fromThreadId, itemId);
      setThreadWorkStates((prev) => ({ ...prev, [fromThreadId]: from }));
      setBacklogState(backlog);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveBacklogItemToThread(itemId: string, toThreadId: string) {
    if (!stream) return;
    try {
      const { backlog, to } = await moveBacklogItemToThread(stream.id, itemId, toThreadId);
      setBacklogState(backlog);
      setThreadWorkStates((prev) => ({ ...prev, [toThreadId]: to }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleCreateBacklogItem(input: Parameters<typeof createBacklogItem>[0]) {
    try {
      const next = await createBacklogItem(input);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleUpdateBacklogItem(itemId: string, changes: Parameters<typeof updateBacklogItem>[1]) {
    try {
      const next = await updateBacklogItem(itemId, changes);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleDeleteBacklogItem(itemId: string) {
    try {
      const next = await deleteBacklogItem(itemId);
      setBacklogState(next);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderBacklog(orderedItemIds: string[]) {
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
    () => (stream ? fileSessions[stream.id] ?? createEmptyFileSession() : createEmptyFileSession()),
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
  const selectedThreadWork = selectedThread ? threadWorkStates[selectedThread.id] ?? null : null;
  const selectedThreadTurns = selectedThread ? agentTurns[selectedThread.id] ?? null : null;

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const activeThreadId = threadStates[s.id]?.activeThreadId;
      if (activeThreadId) out[s.id] = agentStatuses[activeThreadId] ?? "idle";
      else out[s.id] = "idle";
    }
    return out;
  }, [streams, threadStates, agentStatuses]);
  const streamActiveThreadIds = useMemo<Record<string, string | null>>(() => {
    const out: Record<string, string | null> = {};
    for (const s of streams) out[s.id] = threadStates[s.id]?.activeThreadId ?? null;
    return out;
  }, [streams, threadStates]);

  async function handleDropWorkItemOnStream(targetStreamId: string, itemId: string, fromThreadId: string | null) {
    if (!stream || !fromThreadId) return;
    const toThreadId = streamActiveThreadIds[targetStreamId];
    if (!toThreadId || toThreadId === fromThreadId) return;
    try {
      const { from, to } = await moveWorkItemToThread(stream.id, fromThreadId, itemId, toThreadId, targetStreamId);
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

  // After the first stream has had its file sessions rebuilt, verify the
  // initial (localStorage-seeded) centerActive is still resolvable. If it
  // points to a file that didn't come back or a diff tab (which never
  // persist), snap back to "agent". This runs once per mount — subsequent
  // stream switches have their own centerActive logic in handleSwitch.
  useEffect(() => {
    if (centerActiveValidatedRef.current) return;
    if (!stream) return;
    if (!restoredStreamsRef.current.has(stream.id)) return;
    centerActiveValidatedRef.current = true;
    const session = fileSessions[stream.id];
    if (centerActive === "agent") return;
    if (centerActive.startsWith("file:")) {
      const path = centerActive.slice("file:".length);
      if (!session || !session.files[path]) {
        setCenterActive("agent");
      }
      return;
    }
    if (centerActive.startsWith("diff:")) {
      if (!diffTabs.some((tab) => tab.id === centerActive)) {
        setCenterActive("agent");
      }
      return;
    }
    // Unknown id shape — fall back defensively.
    setCenterActive("agent");
  }, [stream, fileSessions, centerActive, diffTabs]);

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
    setFileSessions((prev) => {
      let base = prev[streamId] ?? createEmptyFileSession();
      for (const path of paths) {
        if (base.files[path]) continue;
        base = setOpenFileLoading(openFileInSession(base, path, "", true), path, true);
      }
      // Drop the selection that openFileInSession implicitly set — we want
      // the persisted centerActive, not the last restored file, to decide.
      base = { ...base, selectedPath: null };
      return { ...prev, [streamId]: enforceOpenFileLimit(base, MAX_OPEN_FILE_TABS) };
    });
    // Fire content fetches in parallel.
    for (const path of paths) {
      void (async () => {
        try {
          const file = await readWorkspaceFile(streamId, path);
          setFileSessions((prev) => ({
            ...prev,
            [streamId]: setLoadedFileContent(prev[streamId] ?? createEmptyFileSession(), file.path, file.content),
          }));
        } catch (err) {
          logUi("warn", "failed to restore open file tab", { streamId, path, error: String(err) });
          setFileSessions((prev) => ({
            ...prev,
            [streamId]: closeOpenFile(prev[streamId] ?? createEmptyFileSession(), path),
          }));
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

  useEffect(() => {
    if (!stream || !selectedThread || agentTurns[selectedThread.id]) return;
    void listAgentTurns(stream.id, selectedThread.id)
      .then((turns) => {
        setAgentTurns((prev) => ({ ...prev, [selectedThread.id]: turns }));
      })
      .catch((e) => {
        logUi("warn", "failed to load agent turns", {
          streamId: stream.id,
          threadId: selectedThread.id,
          error: String(e),
        });
      });
  }, [agentTurns, selectedThread, stream]);

  useEffect(() => {
    const unsubscribe = subscribeTurnEvents("all", (event) => {
      void listAgentTurns(event.streamId, event.threadId)
        .then((turns) => {
          setAgentTurns((prev) => ({ ...prev, [event.threadId]: turns }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh agent turns after change event", {
            streamId: event.streamId,
            threadId: event.threadId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    return subscribeWorkspaceContext((next) => setWorkspaceContext(next));
  }, []);

  useEffect(() => {
    let cancelled = false;
    void getBacklogState()
      .then((state) => { if (!cancelled) setBacklogState(state); })
      .catch((error) => logUi("warn", "failed to load backlog state", { error: String(error) }));
    const unsubscribe = subscribeBacklogEvents(() => {
      void getBacklogState()
        .then((state) => setBacklogState(state))
        .catch((error) => logUi("warn", "failed to refresh backlog state", { error: String(error) }));
    });
    return () => { cancelled = true; unsubscribe(); };
  }, []);

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

  useEffect(() => {
    const unsubscribe = subscribeWorkItemEvents("all", (event) => {
      void getThreadWorkState(event.streamId, event.threadId)
        .then((workState) => {
          setThreadWorkStates((prev) => ({ ...prev, [event.threadId]: workState }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh thread work state after change event", {
            streamId: event.streamId,
            threadId: event.threadId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    const unsubscribe = subscribeNewdeEvents((event) => {
      if (event.type !== "thread.changed") return;
      void getThreadState(event.streamId)
        .then((state) => {
          setThreadStates((prev) => ({ ...prev, [event.streamId]: state }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh thread state after change event", {
            streamId: event.streamId,
            threadId: event.threadId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    const unsubscribe = subscribeNewdeEvents((event) => {
      if (event.type !== "stream.changed" || event.kind !== "prompt-changed" || !event.streamId) return;
      void listStreams()
        .then((updated) => {
          setStreams(updated);
          const updatedStream = updated.find((s) => s.id === event.streamId);
          if (updatedStream) setStream((prev) => (prev?.id === updatedStream.id ? updatedStream : prev));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh streams after prompt change", { error: String(error) });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    let cancelled = false;
    const reload = () => {
      void getConfig()
        .then((cfg) => {
          if (cancelled) return;
          setGeneratedDirsState(cfg.generatedDirs);
        })
        .catch((error) => {
          logUi("warn", "failed to load config", { error: String(error) });
        });
    };
    reload();
    const unsub = subscribeNewdeEvents((event) => {
      if (event.type === "config.changed") reload();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, []);

  const handleToggleGeneratedDir = async (name: string, mark: boolean) => {
    const next = mark
      ? Array.from(new Set([...generatedDirs, name])).sort()
      : generatedDirs.filter((entry) => entry !== name);
    try {
      const cfg = await setGeneratedDirs(next);
      setGeneratedDirsState(cfg.generatedDirs);
    } catch (err) {
      setError(`Failed to update generated dirs: ${String(err)}`);
    }
  };

  useEffect(() => {
    let cancelled = false;
    listAgentStatuses()
      .then((entries) => {
        if (cancelled) return;
        const next: Record<string, AgentStatus> = {};
        for (const entry of entries) next[entry.threadId] = entry.status;
        setAgentStatuses(next);
      })
      .catch((error) => {
        logUi("warn", "failed to seed agent statuses", { error: String(error) });
      });
    const unsubscribe = subscribeAgentStatus("all", (entry) => {
      setAgentStatuses((prev) => ({ ...prev, [entry.threadId]: entry.status }));
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

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
            setFileSessions((prev) => ({
              ...prev,
              [stream.id]: setLoadedFileContent(prev[stream.id] ?? createEmptyFileSession(), file.path, file.content),
            }));
            return;
          case "replace-draft":
            setFileSessions((prev) => ({
              ...prev,
              [stream.id]: markFileSaved(prev[stream.id] ?? createEmptyFileSession(), file.path, file.content),
            }));
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
  const [leftDockActivate, setLeftDockActivate] = useState<{ id: string; token: number } | undefined>(undefined);
  const [planNewRequest, setPlanNewRequest] = useState(0);
  const [planEditRequest, setPlanEditRequest] = useState<{ itemId: string; token: number } | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const commandState = useMemo(
    () => ({
      hasStream: !!stream,
      hasSelectedFile: !!selectedFilePath,
      canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
      hasThread: !!selectedThread,
      activeTab: centerActive.startsWith("file:") ? "editor" : "agent",
      canCommit: !!stream && !!workspaceContext.gitEnabled,
    } as const),
    [centerActive, currentFile, currentFileDirty, selectedThread, selectedFilePath, stream, workspaceContext.gitEnabled],
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
    showAgentPane() {
      setCenterActive("agent");
    },
    showEditorPane() {
      if (selectedFilePath) setCenterActive(`file:${selectedFilePath}`);
    },
    newWorkItem() {
      setLeftDockActivate((prev) => ({ id: "plan", token: (prev?.token ?? 0) + 1 }));
      setPlanNewRequest((prev) => prev + 1);
    },
    newStream() {
      setStreamCreateRequest((n) => n + 1);
    },
    newThread() {
      if (!stream) return;
      setThreadCreateRequest((n) => n + 1);
    },
    openHistory() {
      setBottomActivate({ id: "history", token: Date.now() });
    },
    openSnapshots() {
      setBottomActivate({ id: "snapshots", token: Date.now() });
    },
    commitFiles() {
      if (!stream || !workspaceContext.gitEnabled) return;
      setLeftDockActivate((prev) => ({ id: "project", token: (prev?.token ?? 0) + 1 }));
      setCommitFilesRequest((n) => n + 1);
    },
  }), [stream, selectedFilePath, workspaceContext.gitEnabled]);
  const menuGroupSnapshots = useMemo(() => buildMenuGroupSnapshots(commandState), [commandState]);
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
    if (isElectron) return;
    function handleKeyDown(event: KeyboardEvent) {
      const commandId = getCommandIdForShortcut(event);
      if (!commandId) return;
      // Only "plan.newWorkItem" suppresses itself inside a text input — the
      // rest (save, find, quick-open) are explicitly useful while editing.
      // Rationale: a user in the middle of typing a description shouldn't
      // lose focus to a New-Work-Item modal and drop their half-typed text.
      if (commandId === "plan.newWorkItem" && isEditableTarget(event.target)) return;
      const command = commandMap.get(commandId);
      if (!command || !command.enabled || !command.run) return;
      event.preventDefault();
      command.run();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [commandMap, isElectron]);

  useEffect(() => {
    if (!isElectron) return;
    void window.newdeApi.setNativeMenu(menuGroupSnapshots).catch((error) => {
      logUi("error", "failed to update native menu", { error: String(error) });
    });
  }, [isElectron, menuGroupSnapshots]);

  useEffect(() => {
    if (!isElectron) return;
    return window.newdeApi.onMenuCommand((commandId) => {
      // Same "don't yank a user out of a text field" rule as the non-Electron
      // keydown path, applied here because native-menu shortcuts fire
      // regardless of web-view focus.
      if (commandId === "plan.newWorkItem" && isEditableTarget(document.activeElement)) return;
      const command = commandMap.get(commandId);
      if (!command || !command.enabled || !command.run) return;
      command.run();
    });
  }, [commandMap, isElectron]);

  const availableCenterIds = useMemo(() => {
    const ids = new Set(["agent"]);
    for (const path of currentSession.openOrder) ids.add(`file:${path}`);
    for (const tab of diffTabs) ids.add(tab.id);
    return ids;
  }, [currentSession.openOrder, diffTabs]);
  const effectiveCenterActive = availableCenterIds.has(centerActive) ? centerActive : "agent";

  const handleOpenDiff = (request: DiffSpec) => {
    const rightKey = request.rightKind === "working" ? "working" : `ref:${request.rightKind.ref}`;
    // Including the labelOverride in the id lets snapshot diffs coexist with
    // git diffs for the same path without colliding.
    const labelKey = request.labelOverride ? `:${request.labelOverride}` : "";
    const id = `diff:${request.leftRef}:${rightKey}:${request.path}${labelKey}`;
    setDiffTabs((prev) => {
      if (prev.some((tab) => tab.id === id)) return prev;
      return [...prev, { id, spec: request }];
    });
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
    const id = `diff:clipboard:${Date.now()}:${path}`;
    const spec: DiffSpec = {
      path,
      leftRef: "",
      rightKind: "working",
      baseLabel: "clipboard",
      leftContent: selection,
      rightContent: clipboard,
      labelOverride: "selection vs clipboard",
    };
    setDiffTabs((prev) => [...prev, { id, spec }]);
    setCenterActive(id);
  };

  const handleOpenTurnDiff = async (turn: AgentTurn, path: string) => {
    try {
      if (!turn.end_snapshot_id) {
        setError(`Turn has no end snapshot yet for ${path}`);
        return;
      }
      const diff = await getSnapshotPairDiff(turn.start_snapshot_id, turn.end_snapshot_id, path);
      if (diff.beforeState === "absent" && diff.afterState === "absent") {
        setError(`No snapshot diff available for ${path} in this turn`);
        return;
      }
      handleOpenDiff({
        path,
        leftRef: "",
        rightKind: "working",
        baseLabel: `turn ${turn.id.slice(-6)}`,
        leftContent: renderDiffSide(diff.before, diff.beforeState),
        rightContent: renderDiffSide(diff.after, diff.afterState),
        labelOverride: `turn ${turn.id.slice(-6)}`,
      });
    } catch (err) {
      setError(`Open turn diff failed: ${String(err)}`);
    }
  };

  const handleRevealCommit = (sha: string) => {
    const token = Date.now();
    setHistoryReveal({ sha, token });
    setBottomActivate({ id: "history", token });
  };

  const handleRequestEditWorkItem = (itemId: string) => {
    const token = Date.now();
    setLeftDockActivate({ id: "plan", token });
    setPlanEditRequest({ itemId, token });
  };

  const handleShowWorkItemInHistory = async (itemId: string) => {
    const token = Date.now();
    // Open the Local History panel regardless of whether we find an effort —
    // the user asked to see history, so get them there.
    setBottomActivate({ id: "snapshots", token });
    try {
      const efforts = await listWorkItemEfforts(itemId);
      // Newest effort with an end snapshot wins.
      const newest = [...efforts].reverse().find((e) => e.effort.end_snapshot_id);
      const snapshotId = newest?.effort.end_snapshot_id ?? null;
      if (snapshotId) {
        setSnapshotsReveal({ snapshotId, token });
      }
    } catch (err) {
      logUi("warn", "show work item in history failed", { error: String(err) });
    }
  };

  const closeDiffTab = (id: string) => {
    setDiffTabs((prev) => prev.filter((tab) => tab.id !== id));
    setCenterActive((current) => (current === id ? "agent" : current));
  };

  const agentThreadStatus: AgentStatus = selectedThread ? agentStatuses[selectedThread.id] ?? "idle" : "idle";

  const centerTabs: CenterTab[] = useMemo(() => {
    const tabs: CenterTab[] = [
      {
        id: "agent",
        label: "Agent",
        closable: false,
        agentStatus: agentThreadStatus,
        render: () =>
          selectedThread ? (
            <TerminalPane paneTarget={selectedThread.pane_target} visible={effectiveCenterActive === "agent"} />
          ) : (
            <div style={{ padding: 12, color: "var(--muted)" }}>No thread selected.</div>
          ),
      },
    ];
    for (const path of currentSession.openOrder) {
      const basename = path.split("/").pop() ?? path;
      const file = currentSession.files[path];
      const dirty = !!file && file.draftContent !== file.savedContent;
      tabs.push({
        id: `file:${path}`,
        label: `${dirty ? "● " : ""}${basename}`,
        closable: true,
        render: () => stream ? (
          // One shared EditorPane across all file tabs — React keeps the same
          // component instance as long as the element type in the same slot
          // is unchanged, so Monaco's editor stays alive and just swaps models
          // when `filePath` changes.
          <EditorPane
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
            onSelectOpenFile={handleSelectOpenFile}
            onCloseOpenFile={handleCloseOpenFile}
            onRevealCommit={handleRevealCommit}
            onRevealWorkItem={handleRequestEditWorkItem}
            onCompareWithClipboard={handleCompareWithClipboard}
          />
        ) : null,
      });
    }
    for (const diff of diffTabs) {
      const label = diff.spec.path.split("/").pop() ?? diff.spec.path;
      const suffix = diff.spec.labelOverride ?? "diff";
      tabs.push({
        id: diff.id,
        label: `${label} (${suffix})`,
        closable: true,
        render: () => stream ? (
          <DiffPane
            stream={stream}
            spec={diff.spec}
            visible={effectiveCenterActive === diff.id}
            onJumpToSource={(path) => {
              void handleOpenFile(path);
              closeDiffTab(diff.id);
            }}
          />
        ) : null,
      });
    }
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
  ]);

  const leftToolWindows: ToolWindow[] = useMemo(() => [
    {
      id: "plan",
      label: "Work",
      render: () => (
        <PlanPane
          thread={selectedThread}
          activeThreadId={currentThreadState.activeThreadId}
          threadWork={selectedThreadWork}
          backlog={backlogState}
          onCreateWorkItem={handleCreateWorkItem}
          onUpdateWorkItem={handleUpdateWorkItem}
          onDeleteWorkItem={handleDeleteWorkItem}
          onReorderWorkItems={handleReorderWorkItems}
          onCreateBacklogItem={handleCreateBacklogItem}
          onUpdateBacklogItem={handleUpdateBacklogItem}
          onDeleteBacklogItem={handleDeleteBacklogItem}
          onReorderBacklog={handleReorderBacklog}
          onMoveItemToBacklog={handleMoveItemToBacklog}
          openNewRequest={planNewRequest}
          editRequest={planEditRequest}
          onOpenFile={handleOpenFile}
          onShowInHistory={handleShowWorkItemInHistory}
        />
      ),
    },
    {
      id: "project",
      label: "Files",
      render: () => (
        <ProjectPanel
          stream={stream}
          gitEnabled={workspaceContext.gitEnabled}
          selectedFilePath={selectedFilePath}
          currentThreadTurns={selectedThreadTurns}
          generatedDirs={generatedDirs}
          onOpenFile={handleOpenFile}
          onOpenDiff={handleOpenDiff}
          onCreateFile={handleCreateFile}
          onCreateDirectory={handleCreateDirectory}
          onRenamePath={handleRenamePath}
          onDeletePath={handleDeletePath}
          onToggleGeneratedDir={handleToggleGeneratedDir}
          commitRequest={commitFilesRequest}
        />
      ),
    },
    {
      id: "activity",
      label: "Activity",
      render: () => (
        <Activity
          agentTurns={selectedThreadTurns}
          workItems={selectedThreadWork?.items ?? []}
          onOpenFile={handleOpenFile}
          onOpenTurnDiff={handleOpenTurnDiff}
        />
      ),
    },
  ], [
    stream,
    selectedFilePath,
    workspaceContext.gitEnabled,
    selectedThread,
    selectedThreadWork,
    selectedThreadTurns,
  ]);

  const bottomToolWindows: ToolWindow[] = [
    {
      id: "hook-events",
      label: "Hook events",
      render: () => <BottomPanel streamId={stream?.id ?? null} />,
    },
    {
      id: "history",
      label: "Git history",
      render: () => <HistoryPanel stream={stream} onOpenDiff={handleOpenDiff} revealSha={historyReveal} />,
    },
    {
      id: "snapshots",
      label: "Local history",
      render: () => <SnapshotsPanel stream={stream} onOpenDiff={handleOpenDiff} revealSnapshotId={snapshotsReveal} onRequestEditWorkItem={handleRequestEditWorkItem} />,
    },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden" }}>
      <div style={{ borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        {!isElectron ? <Menubar groups={menuGroups} /> : null}
        <StreamRail
          stream={stream}
          streams={streams}
          streamStatuses={streamStatuses}
          streamActiveThreadIds={streamActiveThreadIds}
          gitEnabled={workspaceContext.gitEnabled}
          onSwitch={handleSwitch}
          onStreamCreated={handleStreamCreated}
          onRenameStream={(id, title) => void handleRenameStreamById(id, title)}
          onRequestCreateThread={stream ? () => setThreadCreateRequest((n) => n + 1) : undefined}
          onOpenSettings={() => setSettingsOpen(true)}
          onDropWorkItemOnStream={(targetStreamId, itemId, fromThreadId) => void handleDropWorkItemOnStream(targetStreamId, itemId, fromThreadId)}
          onReorderStreams={handleReorderStreams}
          createRequest={streamCreateRequest}
        />
        {stream ? (
          <ThreadRail
            streamId={stream.id}
            threads={currentThreadState.threads}
            activeThreadId={currentThreadState.activeThreadId}
            selectedThreadId={currentThreadState.selectedThreadId}
            agentStatuses={agentStatuses}
            threadWorkStates={threadWorkStates}
            agentTurns={agentTurns}
            onSelectThread={handleSelectThread}
            onCreateThread={handleCreateThread}
            onPromoteThread={handlePromoteThread}
            onCompleteThread={handleCompleteThread}
            onMoveWorkItem={handleMoveWorkItemToThread}
            onMoveBacklogItemToThread={handleMoveBacklogItemToThread}
            onRenameThread={handleRenameThreadById}
            onReorderThreads={handleReorderThreads}
            onRequestCreateStream={() => setStreamCreateRequest((n) => n + 1)}
            createRequest={threadCreateRequest}
          />
        ) : null}
        {error ? (
          <div style={{ padding: "2px 12px", background: "var(--bg-2)", color: "#ff6b6b", fontSize: 11, minHeight: 22, borderBottom: "1px solid var(--border)" }}>{error}</div>
        ) : null}
      </div>
      <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <DockShell
          side="left"
          toolWindows={leftToolWindows}
          storageKey="left"
          defaultSize={300}
          minSize={220}
          maxSize={640}
          railMode="always"
          activateRequest={leftDockActivate}
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
                if (id.startsWith("file:")) handleCloseOpenFile(id.slice("file:".length));
                else if (id.startsWith("diff:")) closeDiffTab(id);
              }}
            />
          ) : <div style={{ padding: 12 }}>loading…</div>}
        </div>
      </div>
      <DockShell
        side="bottom"
        toolWindows={bottomToolWindows}
        storageKey="bottom"
        defaultOpen={false}
        defaultSize={200}
        minSize={120}
        maxSize={480}
        railMode="always"
        activateRequest={bottomActivate}
      />
      <QuickOpenOverlay
        open={quickOpenVisible}
        stream={stream}
        selectedFilePath={selectedFilePath}
        onClose={() => setQuickOpenVisible(false)}
        onOpenFile={(path) => {
          void handleOpenFile(path);
        }}
      />
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      {stream && externalFilePrompt ? (
        <ExternalFileChangedDialog
          path={externalFilePrompt.path}
          onReload={() => {
            setFileSessions((prev) => ({
              ...prev,
              [stream.id]: markFileSaved(prev[stream.id] ?? createEmptyFileSession(), externalFilePrompt.path, externalFilePrompt.content),
            }));
            setExternalFilePrompt(null);
          }}
          onKeepMine={() => {
            setFileSessions((prev) => ({
              ...prev,
              [stream.id]: setLoadedFileContent(
                prev[stream.id] ?? createEmptyFileSession(),
                externalFilePrompt.path,
                externalFilePrompt.content,
              ),
            }));
            setExternalFilePrompt(null);
          }}
        />
      ) : null}
      {daemonUnavailable ? <DaemonDownDialog /> : null}
      {paletteOpen ? (
        <CommandPalette menuGroups={menuGroups} onClose={() => setPaletteOpen(false)} />
      ) : null}
      {pendingDirtyClose ? (
        <ConfirmDialog
          message={`"${pendingDirtyClose.basename}" has unsaved changes. Close and discard them?`}
          confirmLabel="Discard"
          destructive
          onConfirm={() => {
            const { path } = pendingDirtyClose;
            setPendingDirtyClose(null);
            closeOpenFileNow(path);
          }}
          onCancel={() => setPendingDirtyClose(null)}
        />
      ) : null}
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

function renderDiffSide(
  content: string | null,
  state: "absent" | "present" | "oversize",
): string {
  if (content !== null) return content;
  switch (state) {
    case "absent":
      return "// (file not tracked at this snapshot)";
    case "oversize":
      return "// (file too large to snapshot — size/mtime tracked only)";
    case "present":
      // "present" with null content = blob read failed.
      return "// (snapshot blob unreadable)";
  }
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
