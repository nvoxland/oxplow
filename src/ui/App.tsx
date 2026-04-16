import { useEffect, useMemo, useRef, useState } from "react";
import {
  getCurrentStream,
  getWorkspaceContext,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameCurrentStream,
  subscribeWorkspaceEvents,
  switchStream,
  writeWorkspaceFile,
  type Stream,
  type WorkspaceContext,
} from "./api.js";
import {
  closeOpenFile,
  createEmptyFileSession,
  markFileSaved,
  openFileInSession,
  selectOpenFile,
  setLoadedFileContent,
  setOpenFileLoading,
  updateFileDraft,
  type FileSessionState,
} from "../file-session.js";
import { buildMenuGroups } from "./commands.js";
import { externalFileSyncAction } from "./external-file-sync.js";
import type { EditorNavigationTarget } from "./lsp.js";
import { TopBar } from "./components/TopBar.js";
import { LeftPanel, type SidebarTab } from "./components/LeftPanel.js";
import { Menubar } from "./components/Menubar.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { MainTabs, type TabId } from "./components/MainTabs.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { shouldRefreshAfterDaemonRecovery } from "./daemon-recovery.js";
import { getCommandIdForShortcut } from "./keybindings.js";
import { logUi } from "./logger.js";

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [stream, setStream] = useState<Stream | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("working");
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
  const [sidebarTab, setSidebarTab] = useState<SidebarTab>("files");
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [editorFindRequest, setEditorFindRequest] = useState(0);
  const [editorNavigationTarget, setEditorNavigationTarget] = useState<EditorNavigationTarget | null>(null);
  const [externalFilePrompt, setExternalFilePrompt] = useState<{ path: string; content: string } | null>(null);
  const daemonDownLogged = useRef(false);
  const daemonWasUnavailable = useRef(false);

  useEffect(() => {
    Promise.all([listStreams(), getCurrentStream(), getWorkspaceContext()])
      .then(([allStreams, current, context]) => {
        setStreams(allStreams);
        setStream(current);
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
      if (shouldRefreshAfterDaemonRecovery(daemonWasUnavailable.current, alive)) {
        logUi("info", "daemon recovered, refreshing ui");
        window.location.reload();
        return;
      }
      setDaemonUnavailable(!alive);
      if (!alive && !daemonDownLogged.current) {
        logUi("warn", "daemon probe failed");
        daemonDownLogged.current = true;
      }
      if (alive) {
        daemonDownLogged.current = false;
      }
      daemonWasUnavailable.current = !alive;
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
      setStream(next);
      const nextSession = fileSessions[next.id] ?? createEmptyFileSession();
      setActiveTab(nextSession.selectedPath ? "editor" : "working");
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
    setStreams((prev) => {
      const others = prev.filter((stream) => stream.id !== next.id);
      return [...others, next].sort((a, b) => a.created_at.localeCompare(b.created_at));
    });
    setStream(next);
    const nextSession = fileSessions[next.id] ?? createEmptyFileSession();
    setActiveTab(nextSession.selectedPath ? "editor" : "working");
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

  const currentSession = useMemo(
    () => (stream ? fileSessions[stream.id] ?? createEmptyFileSession() : createEmptyFileSession()),
    [fileSessions, stream],
  );
  const selectedFilePath = currentSession.selectedPath;
  const currentFile = selectedFilePath ? currentSession.files[selectedFilePath] ?? null : null;
  const currentFileDirty = !!currentFile && currentFile.draftContent !== currentFile.savedContent;
  const currentFileRef = useRef(currentFile);
  currentFileRef.current = currentFile;

  useEffect(() => {
    setExternalFilePrompt(null);
  }, [stream?.id, selectedFilePath]);

  useEffect(() => {
    if (!stream || !selectedFilePath) return;
    let cancelled = false;
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
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
  const menuGroups = useMemo(
    () =>
      buildMenuGroups(
        {
          hasStream: !!stream,
          hasSelectedFile: !!selectedFilePath,
          canSave: !!currentFile && !currentFile.isLoading && currentFileDirty,
          activeTab,
          sidebarTab,
        },
        {
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
          showStreamSidebar() {
            setSidebarTab("stream");
          },
          showWorkingPane() {
            setActiveTab("working");
          },
          showTalkingPane() {
            setActiveTab("talking");
          },
          showEditorPane() {
            setActiveTab("editor");
          },
        },
      ),
    [activeTab, currentFile, currentFileDirty, selectedFilePath, sidebarTab, stream],
  );
  const commandMap = useMemo(
    () => new Map(menuGroups.flatMap((group) => group.items.map((item) => [item.id, item] as const))),
    [menuGroups],
  );

  useEffect(() => {
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
  }, [commandMap]);

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        gridTemplateColumns: "240px 1fr",
        height: "100vh",
        gap: 0,
      }}
    >
      <div style={{ gridColumn: "1 / 3", borderBottom: "1px solid var(--border)" }}>
        <Menubar groups={menuGroups} />
        <TopBar
          stream={stream}
          streams={streams}
          gitEnabled={workspaceContext.gitEnabled}
          error={error}
          onSwitch={handleSwitch}
          onRename={handleRename}
          onStreamCreated={handleStreamCreated}
        />
      </div>
      <div style={{ borderRight: "1px solid var(--border)", overflow: "auto" }}>
        <LeftPanel
          stream={stream}
          activeTab={sidebarTab}
          onActiveTabChange={setSidebarTab}
          selectedFilePath={selectedFilePath}
          onOpenFile={handleOpenFile}
        />
      </div>
      <div style={{ overflow: "hidden", minHeight: 0 }}>
        {stream ? (
          <MainTabs
            key={stream.id}
            stream={stream}
            active={activeTab}
            onActiveChange={setActiveTab}
            openFileOrder={currentSession.openOrder}
            openFiles={currentSession.files}
            currentFilePath={selectedFilePath}
            currentFileContent={currentFile?.draftContent ?? ""}
            onEditorChange={handleEditorChange}
            editorFindRequest={editorFindRequest}
            editorNavigationTarget={editorNavigationTarget}
            onNavigateToLocation={handleNavigateToLocation}
            onSelectOpenFile={handleSelectOpenFile}
            onCloseOpenFile={handleCloseOpenFile}
          />
        ) : <div style={{ padding: 12 }}>loading…</div>}
      </div>
      <div style={{ gridColumn: "1 / 3", borderTop: "1px solid var(--border)" }}>
        <BottomPanel streamId={stream?.id ?? null} />
      </div>
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
