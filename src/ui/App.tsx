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
  reorderWorkItems,
  subscribeAgentStatus,
  type AgentStatus,
  createWorkspaceFile,
  deleteWorkspacePath,
  getCurrentStream,
  getWorkspaceContext,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameWorkspacePath,
  renameCurrentStream,
  subscribeWorkItemEvents,
  subscribeWorkspaceEvents,
  selectBatch,
  promoteBatch,
  reorderBatch,
  switchStream,
  updateWorkItem,
  writeWorkspaceFile,
  type BatchWorkState,
  type BatchState,
  type Stream,
  type WorkspaceContext,
} from "./api.js";
import {
  closeOpenFile,
  createEmptyFileSession,
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
import { TopBar } from "./components/TopBar.js";
import { LeftPanel, type SidebarTab } from "./components/LeftPanel/index.js";
import { Menubar } from "./components/Menubar.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { MainTabs, type TabId } from "./components/MainTabs.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { getCommandIdForShortcut } from "./keybindings.js";
import { logUi } from "./logger.js";

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [batchStates, setBatchStates] = useState<Record<string, BatchState>>({});
  const [batchWorkStates, setBatchWorkStates] = useState<Record<string, BatchWorkState>>({});
  const [agentStatuses, setAgentStatuses] = useState<Record<string, AgentStatus>>({});
  const [stream, setStream] = useState<Stream | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("agent");
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
  const [sidebarTab, setSidebarTab] = useState<SidebarTab>("batches");
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() => readInitialSidebarWidth());
  const [sidebarDragging, setSidebarDragging] = useState(false);
  const daemonDownLogged = useRef(false);
  const daemonProbeState = useRef(INITIAL_DAEMON_PROBE_STATE);
  const isElectron = !!window.newdeDesktop?.isElectron;

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
      setActiveTab(nextSession.selectedPath ? "editor" : "agent");
      setError(null);
      setDaemonUnavailable(false);
      logUi("info", "switched stream", { streamId: next.id, title: next.title });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to switch stream", { streamId: id, error: String(e) });
    }
  }

  async function handleRename(title: string) {
    try {
      const updated = await renameCurrentStream(title);
      setStream(updated);
      setStreams((prev) =>
        prev
          .map((candidate) => (candidate.id === updated.id ? updated : candidate))
          .sort((a, b) => a.created_at.localeCompare(b.created_at)),
      );
      setError(null);
      setDaemonUnavailable(false);
      logUi("info", "renamed current stream", { streamId: updated.id, title: updated.title });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to rename current stream", { error: String(e), title });
      throw e;
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
    setActiveTab(nextSession.selectedPath ? "editor" : "agent");
    setError(null);
    setDaemonUnavailable(false);
    logUi("info", "stream created in ui", { streamId: next.id, title: next.title, branch: next.branch });
  }

  async function handleOpenFile(path: string) {
    if (!stream) return;
    const currentSession = fileSessions[stream.id] ?? createEmptyFileSession();
    const existing = currentSession.files[path];
    setFileSessions((prev) => ({
      ...prev,
      [stream.id]: existing
        ? selectOpenFile(prev[stream.id] ?? createEmptyFileSession(), path)
        : setOpenFileLoading(openFileInSession(prev[stream.id] ?? createEmptyFileSession(), path, "", true), path, true),
    }));
    setActiveTab("editor");
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
    setActiveTab("editor");
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
    setActiveTab("editor");
  }

  function handleCloseOpenFile(path: string) {
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
      setSidebarTab("batches");
      setActiveTab("plan");
      setError(null);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  async function handleReorderBatch(batchId: string, targetIndex: number) {
    if (!stream) return;
    try {
      const next = await reorderBatch(stream.id, batchId, targetIndex);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handlePromoteBatch(batchId: string) {
    if (!stream) return;
    try {
      const next = await promoteBatch(stream.id, batchId);
      setBatchStates((prev) => ({ ...prev, [stream.id]: next }));
      setActiveTab("agent");
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
      setActiveTab("agent");
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateWorkItem(input: {
    kind: "epic" | "task" | "subtask" | "bug" | "note";
    title: string;
    description?: string;
    parentId?: string | null;
    status?: "waiting" | "ready" | "in_progress" | "blocked" | "done" | "canceled";
    priority?: "low" | "medium" | "high" | "urgent";
  }) {
    if (!stream || !selectedBatch) return;
    try {
      const next = await createWorkItem(stream.id, selectedBatch.id, input);
      setBatchWorkStates((prev) => ({ ...prev, [selectedBatch.id]: next }));
      setActiveTab("plan");
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
      parentId?: string | null;
      status?: "waiting" | "ready" | "in_progress" | "blocked" | "done" | "canceled";
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

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const activeBatchId = batchStates[s.id]?.activeBatchId;
      if (activeBatchId) out[s.id] = agentStatuses[activeBatchId] ?? "idle";
      else out[s.id] = "idle";
    }
    return out;
  }, [streams, batchStates, agentStatuses]);
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
  const commandState = useMemo(
    () => ({
      hasStream: !!stream,
      hasSelectedFile: !!selectedFilePath,
      canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
      activeTab,
      sidebarTab,
    }),
    [activeTab, currentFile, currentFileDirty, selectedFilePath, sidebarTab, stream],
  );
  const commandHandlers = {
    save() {
      void handleEditorSave();
    },
    quickOpen() {
      if (!stream) return;
      setQuickOpenVisible(true);
    },
    find() {
      if (!selectedFilePath) return;
      setActiveTab("editor");
      setEditorFindRequest((current) => current + 1);
    },
    showFilesSidebar() {
      setSidebarTab("files");
    },
    showBatchesSidebar() {
      setSidebarTab("batches");
    },
    showStreamSidebar() {
      setSidebarTab("stream");
    },
    showAgentPane() {
      setActiveTab("agent");
    },
    showPlanPane() {
      setActiveTab("plan");
    },
    showEditorPane() {
      setActiveTab("editor");
    },
  };
  const menuGroupSnapshots = useMemo(() => buildMenuGroupSnapshots(commandState), [commandState]);
  const menuGroups = buildMenuGroups(commandState, commandHandlers);
  const commandMap = useMemo(
    () => new Map(menuGroups.flatMap((group) => group.items.map((item) => [item.id, item] as const))),
    [menuGroups],
  );

  useEffect(() => {
    if (isElectron) return;
    function handleKeyDown(event: KeyboardEvent) {
      const commandId = getCommandIdForShortcut(event);
      if (!commandId) return;
      const command = commandMap.get(commandId);
      if (!command || !command.enabled) return;
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
      const command = commandMap.get(commandId);
      if (!command || !command.enabled) return;
      command.run();
    });
  }, [commandMap, isElectron]);

  useEffect(() => {
    window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    function handleResize() {
      setSidebarWidth((current) => clampSidebarWidth(current));
    }

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  useEffect(() => {
    if (!sidebarDragging) return;

    function handlePointerMove(event: PointerEvent) {
      setSidebarWidth(clampSidebarWidth(event.clientX));
    }

    function stopDragging() {
      setSidebarDragging(false);
    }

    const previousCursor = document.body.style.cursor;
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", stopDragging);
    window.addEventListener("pointercancel", stopDragging);
    return () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousUserSelect;
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", stopDragging);
      window.removeEventListener("pointercancel", stopDragging);
    };
  }, [sidebarDragging]);

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        gridTemplateColumns: `${sidebarWidth}px 6px minmax(0, 1fr)`,
        height: "100vh",
        gap: 0,
      }}
    >
      <div style={{ gridColumn: "1 / 4", borderBottom: "1px solid var(--border)" }}>
        {!isElectron ? <Menubar groups={menuGroups} /> : null}
        <TopBar
          stream={stream}
          streams={streams}
          streamStatuses={streamStatuses}
          gitEnabled={workspaceContext.gitEnabled}
          error={error}
          onSwitch={handleSwitch}
          onRename={handleRename}
          onStreamCreated={handleStreamCreated}
        />
      </div>
      <div style={{ overflow: "auto", minWidth: 0 }}>
        <LeftPanel
          stream={stream}
          batches={currentBatchState.batches}
          batchWorkStates={batchWorkStates}
          agentStatuses={agentStatuses}
          selectedBatchId={currentBatchState.selectedBatchId}
          activeBatchId={currentBatchState.activeBatchId}
          activeTab={sidebarTab}
          onActiveTabChange={setSidebarTab}
          selectedFilePath={selectedFilePath}
          onOpenFile={handleOpenFile}
          onCreateFile={handleCreateFile}
          onCreateDirectory={handleCreateDirectory}
          onRenamePath={handleRenamePath}
          onDeletePath={handleDeletePath}
          onSelectBatch={handleSelectBatch}
          onCreateBatch={handleCreateBatch}
          onReorderBatch={handleReorderBatch}
          onPromoteBatch={handlePromoteBatch}
          onCompleteBatch={handleCompleteBatch}
        />
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize sidebar"
        onPointerDown={(event) => {
          event.preventDefault();
          setSidebarDragging(true);
          setSidebarWidth(clampSidebarWidth(event.clientX));
        }}
        style={{
          cursor: "col-resize",
          background: sidebarDragging ? "var(--accent)" : "var(--border)",
          transition: sidebarDragging ? "none" : "background 120ms ease",
        }}
      />
      <div style={{ overflow: "hidden", minHeight: 0, minWidth: 0 }}>
        {stream ? (
          <MainTabs
            key={stream.id}
            stream={stream}
            batch={selectedBatch}
            activeBatchId={currentBatchState.activeBatchId}
            batchWork={selectedBatchWork}
            active={activeTab}
            onActiveChange={setActiveTab}
            onCreateWorkItem={handleCreateWorkItem}
            onUpdateWorkItem={handleUpdateWorkItem}
            onDeleteWorkItem={handleDeleteWorkItem}
            onReorderWorkItems={handleReorderWorkItems}
            openFileOrder={currentSession.openOrder}
            openFiles={currentSession.files}
            currentFilePath={selectedFilePath}
            currentFileContent={currentFile?.draftContent ?? ""}
            currentFileDirty={currentFileDirty}
            onEditorChange={handleEditorChange}
            onEditorSave={() => {
              void handleEditorSave();
            }}
            editorFindRequest={editorFindRequest}
            editorNavigationTarget={editorNavigationTarget}
            onNavigateToLocation={handleNavigateToLocation}
            onSelectOpenFile={handleSelectOpenFile}
            onCloseOpenFile={handleCloseOpenFile}
          />
        ) : <div style={{ padding: 12 }}>loading…</div>}
      </div>
      <div style={{ gridColumn: "1 / 4", borderTop: "1px solid var(--border)" }}>
        <BottomPanel streamId={stream?.id ?? null} />
      </div>
      {sidebarDragging ? (
        <div
          style={{
            position: "fixed",
            inset: 0,
            cursor: "col-resize",
            zIndex: 100,
          }}
        />
      ) : null}
      <QuickOpenOverlay
        open={quickOpenVisible}
        stream={stream}
        selectedFilePath={selectedFilePath}
        onClose={() => setQuickOpenVisible(false)}
        onOpenFile={(path) => {
          void handleOpenFile(path);
        }}
      />
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
    </div>
  );
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
        <button
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

const SIDEBAR_WIDTH_STORAGE_KEY = "newde.sidebarWidth";
const DEFAULT_SIDEBAR_WIDTH = 240;
const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 560;
const MIN_MAIN_CONTENT_WIDTH = 320;

function readInitialSidebarWidth() {
  const stored = Number(window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY));
  if (!Number.isFinite(stored)) return DEFAULT_SIDEBAR_WIDTH;
  return clampSidebarWidth(stored);
}

function clampSidebarWidth(width: number) {
  const maxAllowed = Math.max(
    MIN_SIDEBAR_WIDTH,
    Math.min(MAX_SIDEBAR_WIDTH, window.innerWidth - MIN_MAIN_CONTENT_WIDTH),
  );
  return Math.min(Math.max(width, MIN_SIDEBAR_WIDTH), maxAllowed);
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
          <button
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
          <button
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
