import { useEffect, useMemo, useRef, useState } from "react";
import {
  createWorkItem,
  completeBatch,
  createBatch,
  deleteWorkItem,
  getBatchWorkState,
  getBatchState,
  createWorkspaceDirectory,
  listAgentStatuses,
  listAgentTurns,
  listBatchFileChanges,
  reorderWorkItems,
  moveWorkItemToBatch,
  getBacklogState,
  createBacklogItem,
  updateBacklogItem,
  deleteBacklogItem,
  reorderBacklog,
  moveWorkItemToBacklog,
  moveBacklogItemToBatch,
  subscribeBacklogEvents,
  subscribeAgentStatus,
  subscribeFileChangeEvents,
  subscribeTurnEvents,
  type AgentStatus,
  type AgentTurn,
  type BatchFileChange,
  createWorkspaceFile,
  deleteWorkspacePath,
  getCurrentStream,
  getWorkspaceContext,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameWorkspacePath,
  renameBatch,
  renameStream,
  subscribeNewdeEvents,
  subscribeWorkItemEvents,
  subscribeWorkspaceContext,
  subscribeWorkspaceEvents,
  getConfig,
  getTurnFileDiff,
  setGeneratedDirs,
  selectBatch,
  promoteBatch,
  reorderBatches,
  reorderStreams,
  switchStream,
  updateWorkItem,
  writeWorkspaceFile,
  type BacklogState,
  type BatchWorkState,
  type BatchState,
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
import { BatchRail } from "./components/BatchRail.js";
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

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [batchStates, setBatchStates] = useState<Record<string, BatchState>>({});
  const [batchWorkStates, setBatchWorkStates] = useState<Record<string, BatchWorkState>>({});
  const [backlogState, setBacklogState] = useState<BacklogState | null>(null);
  const [agentTurns, setAgentTurns] = useState<Record<string, AgentTurn[]>>({});
  const [fileChanges, setFileChanges] = useState<Record<string, BatchFileChange[]>>({});
  const [agentStatuses, setAgentStatuses] = useState<Record<string, AgentStatus>>({});
  const [stream, setStream] = useState<Stream | null>(null);
  const [centerActive, setCenterActive] = useState<string>("agent");
  const [diffTabs, setDiffTabs] = useState<Array<{ id: string; spec: DiffSpec }>>([]);
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const [pendingDirtyClose, setPendingDirtyClose] = useState<{ path: string; basename: string } | null>(null);
  const [historyReveal, setHistoryReveal] = useState<{ sha: string; token: number } | null>(null);
  const [bottomActivate, setBottomActivate] = useState<{ id: string; token: number } | undefined>(undefined);
  const [streamCreateRequest, setStreamCreateRequest] = useState(0);
  const [batchCreateRequest, setBatchCreateRequest] = useState(0);
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
        const initialBatchState = await getBatchState(current.id);
        const initialBatch = initialBatchState.batches.find((batch) => batch.id === initialBatchState.selectedBatchId);
        if (initialBatch) {
          const initialWork = await getBatchWorkState(current.id, initialBatch.id);
          setBatchWorkStates((prev) => ({ ...prev, [initialBatch.id]: initialWork }));
        }
        setStreams(allStreams);
        setStream(current);
        setBatchStates((prev) => ({ ...prev, [current.id]: initialBatchState }));
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
      const nextBatchState = batchStates[next.id] ?? await getBatchState(next.id);
      const nextBatch = nextBatchState.batches.find((batch) => batch.id === nextBatchState.selectedBatchId);
      if (nextBatch && !batchWorkStates[nextBatch.id]) {
        const nextWork = await getBatchWorkState(next.id, nextBatch.id);
        setBatchWorkStates((prev) => ({ ...prev, [nextBatch.id]: nextWork }));
      }
      setBatchStates((prev) => ({ ...prev, [next.id]: nextBatchState }));
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

  async function handleRenameBatchById(batchId: string, newTitle: string) {
    if (!stream) return;
    try {
      await renameBatch(stream.id, batchId, newTitle);
      const refreshed = await getBatchState(stream.id);
      setBatchStates((prev) => ({ ...prev, [stream.id]: refreshed }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  function handleStreamCreated(next: Stream) {
    void getBatchState(next.id).then((state) => {
      setBatchStates((prev) => ({ ...prev, [next.id]: state }));
      const batch = state.batches.find((candidate) => candidate.id === state.selectedBatchId);
      if (batch) {
        void getBatchWorkState(next.id, batch.id).then((work) => {
          setBatchWorkStates((prev) => ({ ...prev, [batch.id]: work }));
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

  async function handleSelectBatch(batchId: string) {
    if (!stream) return;
    try {
      const next = await selectBatch(stream.id, batchId);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      const batch = next.batches.find((candidate) => candidate.id === batchId);
      if (batch) {
        const work = await getBatchWorkState(stream.id, batch.id);
        setBatchWorkStates((prev) => ({ ...prev, [batch.id]: work }));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateBatch(title: string) {
    if (!stream) return;
    try {
      const next = await createBatch(stream.id, title);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      const batch = next.batches.find((candidate) => candidate.id === next.selectedBatchId);
      if (batch) {
        const work = await getBatchWorkState(stream.id, batch.id);
        setBatchWorkStates((prev) => ({ ...prev, [batch.id]: work }));
      }
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handlePromoteBatch(batchId: string) {
    if (!stream) return;
    try {
      const next = await promoteBatch(stream.id, batchId);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      setCenterActive("agent");
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCompleteBatch(batchId: string) {
    if (!stream) return;
    try {
      const next = await completeBatch(stream.id, batchId);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      setCenterActive("agent");
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleReorderBatches(orderedBatchIds: string[]) {
    if (!stream) return;
    try {
      await reorderBatches(stream.id, orderedBatchIds);
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
    if (!stream || !selectedBatch) return;
    try {
      const next = await createWorkItem(stream.id, selectedBatch.id, input);
      setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
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
    if (!stream || !selectedBatch) return;
    try {
      const next = await updateWorkItem(stream.id, selectedBatch.id, itemId, changes);
      setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleDeleteWorkItem(itemId: string) {
    if (!stream || !selectedBatch) return;
    try {
      const next = await deleteWorkItem(stream.id, selectedBatch.id, itemId);
      setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderWorkItems(orderedItemIds: string[]) {
    if (!stream || !selectedBatch) return;
    try {
      const next = await reorderWorkItems(stream.id, selectedBatch.id, orderedItemIds);
      setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveWorkItemToBatch(itemId: string, fromBatchId: string, toBatchId: string) {
    if (!stream || fromBatchId === toBatchId) return;
    try {
      const { from, to } = await moveWorkItemToBatch(stream.id, fromBatchId, itemId, toBatchId);
      setBatchWorkStates((prev) => ({ ...prev, [fromBatchId]: from, [toBatchId]: to }));
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveItemToBacklog(itemId: string, fromBatchId: string) {
    if (!stream) return;
    try {
      const { from, backlog } = await moveWorkItemToBacklog(stream.id, fromBatchId, itemId);
      setBatchWorkStates((prev) => ({ ...prev, [fromBatchId]: from }));
      setBacklogState(backlog);
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleMoveBacklogItemToBatch(itemId: string, toBatchId: string) {
    if (!stream) return;
    try {
      const { backlog, to } = await moveBacklogItemToBatch(stream.id, itemId, toBatchId);
      setBacklogState(backlog);
      setBatchWorkStates((prev) => ({ ...prev, [toBatchId]: to }));
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
  const currentBatchState = useMemo(
    () => (stream ? batchStates[stream.id] ?? { selectedBatchId: null, activeBatchId: null, batches: [] } : { selectedBatchId: null, activeBatchId: null, batches: [] }),
    [batchStates, stream],
  );
  const selectedBatch = currentBatchState.batches.find((batch) => batch.id === currentBatchState.selectedBatchId) ?? null;
  const selectedBatchWork = selectedBatch ? batchWorkStates[selectedBatch.id] ?? null : null;
  const selectedBatchTurns = selectedBatch ? agentTurns[selectedBatch.id] ?? null : null;
  const selectedBatchFileChanges = selectedBatch ? fileChanges[selectedBatch.id] ?? null : null;

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const activeBatchId = batchStates[s.id]?.activeBatchId;
      if (activeBatchId) out[s.id] = agentStatuses[activeBatchId] ?? "idle";
      else out[s.id] = "idle";
    }
    return out;
  }, [streams, batchStates, agentStatuses]);
  const streamActiveBatchIds = useMemo<Record<string, string | null>>(() => {
    const out: Record<string, string | null> = {};
    for (const s of streams) out[s.id] = batchStates[s.id]?.activeBatchId ?? null;
    return out;
  }, [streams, batchStates]);

  async function handleDropWorkItemOnStream(targetStreamId: string, itemId: string, fromBatchId: string | null) {
    if (!stream || !fromBatchId) return;
    const toBatchId = streamActiveBatchIds[targetStreamId];
    if (!toBatchId || toBatchId === fromBatchId) return;
    try {
      const { from, to } = await moveWorkItemToBatch(stream.id, fromBatchId, itemId, toBatchId, targetStreamId);
      setBatchWorkStates((prev) => ({ ...prev, [fromBatchId]: from, [toBatchId]: to }));
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

  useEffect(() => {
    if (!stream || !selectedBatch || batchWorkStates[selectedBatch.id]) return;
    void getBatchWorkState(stream.id, selectedBatch.id)
      .then((next) => {
        setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
      })
      .catch((e) => {
        setError(String(e));
      });
  }, [batchWorkStates, selectedBatch, stream]);

  useEffect(() => {
    if (!stream) return;
    const missing = currentBatchState.batches.filter((batch) => !batchWorkStates[batch.id]);
    if (missing.length === 0) return;
    let cancelled = false;
    void Promise.all(
      missing.map(async (batch) => [batch.id, await getBatchWorkState(stream.id, batch.id)] as const),
    )
      .then((results) => {
        if (cancelled) return;
        setBatchWorkStates((prev) => {
          const next = { ...prev };
          for (const [batchId, work] of results) next[batchId] = work;
          return next;
        });
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [batchWorkStates, currentBatchState.batches, stream]);

  useEffect(() => {
    if (!stream || !selectedBatch || agentTurns[selectedBatch.id]) return;
    void listAgentTurns(stream.id, selectedBatch.id)
      .then((turns) => {
        setAgentTurns((prev) => ({ ...prev, [selectedBatch.id]: turns }));
      })
      .catch((e) => {
        logUi("warn", "failed to load agent turns", {
          streamId: stream.id,
          batchId: selectedBatch.id,
          error: String(e),
        });
      });
  }, [agentTurns, selectedBatch, stream]);

  useEffect(() => {
    if (!stream || !selectedBatch || fileChanges[selectedBatch.id]) return;
    void listBatchFileChanges(stream.id, selectedBatch.id)
      .then((changes) => {
        setFileChanges((prev) => ({ ...prev, [selectedBatch.id]: changes }));
      })
      .catch((e) => {
        logUi("warn", "failed to load batch file changes", {
          streamId: stream.id,
          batchId: selectedBatch.id,
          error: String(e),
        });
      });
  }, [fileChanges, selectedBatch, stream]);

  useEffect(() => {
    const unsubscribe = subscribeFileChangeEvents("all", (event) => {
      void listBatchFileChanges(event.streamId, event.batchId)
        .then((changes) => {
          setFileChanges((prev) => ({ ...prev, [event.batchId]: changes }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh batch file changes", {
            streamId: event.streamId,
            batchId: event.batchId,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    const unsubscribe = subscribeTurnEvents("all", (event) => {
      void listAgentTurns(event.streamId, event.batchId)
        .then((turns) => {
          setAgentTurns((prev) => ({ ...prev, [event.batchId]: turns }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh agent turns after change event", {
            streamId: event.streamId,
            batchId: event.batchId,
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
    for (const [streamId, state] of Object.entries(batchStates)) {
      for (const batch of state.batches) {
        if (batchWorkStates[batch.id]) continue;
        void getBatchWorkState(streamId, batch.id)
          .then((work) => setBatchWorkStates((prev) => (prev[batch.id] ? prev : { ...prev, [batch.id]: work })))
          .catch((error) => logUi("warn", "failed to preload batch work state", { streamId, batchId: batch.id, error: String(error) }));
      }
    }
  }, [batchStates]);

  useEffect(() => {
    const unsubscribe = subscribeWorkItemEvents("all", (event) => {
      void getBatchWorkState(event.streamId, event.batchId)
        .then((workState) => {
          setBatchWorkStates((prev) => ({ ...prev, [event.batchId]: workState }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh batch work state after change event", {
            streamId: event.streamId,
            batchId: event.batchId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    const unsubscribe = subscribeNewdeEvents((event) => {
      if (event.type !== "batch.changed") return;
      void getBatchState(event.streamId)
        .then((state) => {
          setBatchStates((prev) => ({ ...prev, [event.streamId]: state }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh batch state after change event", {
            streamId: event.streamId,
            batchId: event.batchId,
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
        for (const entry of entries) next[entry.batchId] = entry.status;
        setAgentStatuses(next);
      })
      .catch((error) => {
        logUi("warn", "failed to seed agent statuses", { error: String(error) });
      });
    const unsubscribe = subscribeAgentStatus("all", (entry) => {
      setAgentStatuses((prev) => ({ ...prev, [entry.batchId]: entry.status }));
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
  const [paletteOpen, setPaletteOpen] = useState(false);
  const commandState = useMemo(
    () => ({
      hasStream: !!stream,
      hasSelectedFile: !!selectedFilePath,
      canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
      hasBatch: !!selectedBatch,
      activeTab: centerActive.startsWith("file:") ? "editor" : "agent",
      canCommit: !!stream && !!workspaceContext.gitEnabled,
    } as const),
    [centerActive, currentFile, currentFileDirty, selectedBatch, selectedFilePath, stream, workspaceContext.gitEnabled],
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
    newBatch() {
      if (!stream) return;
      setBatchCreateRequest((n) => n + 1);
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

  const handleOpenTurnDiff = async (turnId: string, path: string) => {
    try {
      const diff = await getTurnFileDiff(turnId, path);
      if (diff.beforeState === "absent" && diff.afterState === "absent") {
        setError(`No snapshot diff available for ${path} in this turn`);
        return;
      }
      handleOpenDiff({
        path,
        leftRef: "",
        rightKind: "working",
        baseLabel: `turn ${turnId.slice(-6)}`,
        leftContent: renderDiffSide(diff.before, diff.beforeState),
        rightContent: renderDiffSide(diff.after, diff.afterState),
        labelOverride: `turn ${turnId.slice(-6)}`,
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

  const closeDiffTab = (id: string) => {
    setDiffTabs((prev) => prev.filter((tab) => tab.id !== id));
    setCenterActive("agent");
  };

  const agentBatchStatus: AgentStatus = selectedBatch ? agentStatuses[selectedBatch.id] ?? "idle" : "idle";

  const centerTabs: CenterTab[] = useMemo(() => {
    const tabs: CenterTab[] = [
      {
        id: "agent",
        label: selectedBatch ? selectedBatch.title : "Agent",
        closable: false,
        agentStatus: agentBatchStatus,
        render: () =>
          selectedBatch ? (
            <TerminalPane paneTarget={selectedBatch.pane_target} visible={effectiveCenterActive === "agent"} />
          ) : (
            <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>
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
          <DiffPane stream={stream} spec={diff.spec} visible={effectiveCenterActive === diff.id} />
        ) : null,
      });
    }
    return tabs;
  }, [
    selectedBatch,
    agentBatchStatus,
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
          batch={selectedBatch}
          activeBatchId={currentBatchState.activeBatchId}
          batchWork={selectedBatchWork}
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
          currentBatchTurns={selectedBatchTurns}
          currentBatchFileChanges={selectedBatchFileChanges}
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
          agentTurns={selectedBatchTurns}
          batchFileChanges={selectedBatchFileChanges}
          workItems={selectedBatchWork?.items ?? []}
          onOpenFile={handleOpenFile}
          onOpenTurnDiff={handleOpenTurnDiff}
        />
      ),
    },
  ], [
    stream,
    selectedFilePath,
    workspaceContext.gitEnabled,
    selectedBatch,
    selectedBatchWork,
    selectedBatchTurns,
    selectedBatchFileChanges,
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
      render: () => <SnapshotsPanel stream={stream} onOpenDiff={handleOpenDiff} />,
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
          streamActiveBatchIds={streamActiveBatchIds}
          gitEnabled={workspaceContext.gitEnabled}
          onSwitch={handleSwitch}
          onStreamCreated={handleStreamCreated}
          onRenameStream={(id, title) => void handleRenameStreamById(id, title)}
          onRequestCreateBatch={stream ? () => setBatchCreateRequest((n) => n + 1) : undefined}
          onOpenSettings={() => setSettingsOpen(true)}
          onDropWorkItemOnStream={(targetStreamId, itemId, fromBatchId) => void handleDropWorkItemOnStream(targetStreamId, itemId, fromBatchId)}
          onReorderStreams={handleReorderStreams}
          createRequest={streamCreateRequest}
        />
        {stream ? (
          <BatchRail
            streamId={stream.id}
            batches={currentBatchState.batches}
            activeBatchId={currentBatchState.activeBatchId}
            selectedBatchId={currentBatchState.selectedBatchId}
            agentStatuses={agentStatuses}
            batchWorkStates={batchWorkStates}
            agentTurns={agentTurns}
            fileChanges={fileChanges}
            onSelectBatch={handleSelectBatch}
            onCreateBatch={handleCreateBatch}
            onPromoteBatch={handlePromoteBatch}
            onCompleteBatch={handleCompleteBatch}
            onMoveWorkItem={handleMoveWorkItemToBatch}
            onMoveBacklogItemToBatch={handleMoveBacklogItemToBatch}
            onRenameBatch={handleRenameBatchById}
            onReorderBatches={handleReorderBatches}
            onRequestCreateStream={() => setStreamCreateRequest((n) => n + 1)}
            createRequest={batchCreateRequest}
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
  state: "absent" | "present" | "deleted" | "oversize",
): string {
  if (content !== null) return content;
  switch (state) {
    case "absent":
      return "// (file not tracked at this snapshot)";
    case "deleted":
      return "// (file did not exist at this snapshot)";
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
          boxShadow: "0 12px 40px rgba(0, 0, 0, 0.4)",
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
          boxShadow: "0 12px 40px rgba(0, 0, 0, 0.4)",
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
