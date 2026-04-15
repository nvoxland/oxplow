import { useEffect, useMemo, useRef, useState } from "react";
import {
  getCurrentStream,
  getWorkspaceContext,
  listStreams,
  probeDaemon,
  readWorkspaceFile,
  renameCurrentStream,
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
import { TopBar } from "./components/TopBar.js";
import { LeftPanel } from "./components/LeftPanel.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { MainTabs, type TabId } from "./components/MainTabs.js";
import { shouldRefreshAfterDaemonRecovery } from "./daemon-recovery.js";
import { logUi } from "./logger.js";

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [stream, setStream] = useState<Stream | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("working");
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ gitEnabled: false });
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
  }

  const currentSession = useMemo(
    () => (stream ? fileSessions[stream.id] ?? createEmptyFileSession() : createEmptyFileSession()),
    [fileSessions, stream],
  );
  const selectedFilePath = currentSession.selectedPath;
  const currentFile = selectedFilePath ? currentSession.files[selectedFilePath] ?? null : null;
  const currentFileDirty = !!currentFile && currentFile.draftContent !== currentFile.savedContent;

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
            currentFileDirty={currentFileDirty}
            currentFileLoading={currentFile?.isLoading ?? false}
            onEditorChange={handleEditorChange}
            onEditorSave={handleEditorSave}
            onSelectOpenFile={handleSelectOpenFile}
            onCloseOpenFile={handleCloseOpenFile}
          />
        ) : <div style={{ padding: 12 }}>loading…</div>}
      </div>
      <div style={{ gridColumn: "1 / 3", borderTop: "1px solid var(--border)" }}>
        <BottomPanel streamId={stream?.id ?? null} />
      </div>
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
