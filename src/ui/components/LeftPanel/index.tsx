import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type AgentStatus,
  type Batch,
  type BatchWorkState,
  listWorkspaceEntries,
  listWorkspaceFiles,
  subscribeWorkspaceEvents,
  type Stream,
  type WorkspaceEntry,
  type WorkspaceIndexedFile,
  type WorkspaceStatusSummary,
} from "../../api.js";
import type { MenuItem } from "../../menu.js";
import { ContextMenu } from "../ContextMenu.js";
import { BatchQueueSection } from "./BatchQueueSection.js";
import { ChangedFilesSection, TreeEntries } from "./FileTree.js";
import { GitSummary } from "./GitSummary.js";
import {
  copyText,
  dirname,
  joinChildPath,
  Row,
  Section,
  SidebarButton,
  type ContextMenuTarget,
} from "./shared.js";

export type SidebarTab = "batches" | "files" | "stream";

interface Props {
  stream: Stream | null;
  batches: Batch[];
  batchWorkStates: Record<string, BatchWorkState>;
  agentStatuses: Record<string, AgentStatus>;
  selectedBatchId: string | null;
  activeBatchId: string | null;
  activeTab: SidebarTab;
  onActiveTabChange(tab: SidebarTab): void;
  selectedFilePath: string | null;
  onOpenFile(path: string): void;
  onCreateFile(path: string): Promise<void>;
  onCreateDirectory(path: string): Promise<void>;
  onRenamePath(fromPath: string, toPath: string): Promise<void>;
  onDeletePath(path: string): Promise<void>;
  onSelectBatch(batchId: string): Promise<void>;
  onCreateBatch(title: string): Promise<void>;
  onReorderBatch(batchId: string, targetIndex: number): Promise<void>;
  onPromoteBatch(batchId: string): Promise<void>;
  onCompleteBatch(batchId: string): Promise<void>;
}

export function LeftPanel({
  stream,
  batches,
  batchWorkStates,
  agentStatuses,
  selectedBatchId,
  activeBatchId,
  activeTab,
  onActiveTabChange,
  selectedFilePath,
  onOpenFile,
  onCreateFile,
  onCreateDirectory,
  onRenamePath,
  onDeletePath,
  onSelectBatch,
  onCreateBatch,
  onReorderBatch,
  onPromoteBatch,
  onCompleteBatch,
}: Props) {
  const [expandedDirs, setExpandedDirs] = useState<Record<string, boolean>>({ "": true });
  const [entriesByDir, setEntriesByDir] = useState<Record<string, WorkspaceEntry[]>>({});
  const [loadingDirs, setLoadingDirs] = useState<Record<string, boolean>>({});
  const [indexedFiles, setIndexedFiles] = useState<WorkspaceIndexedFile[]>([]);
  const [statusSummary, setStatusSummary] = useState<WorkspaceStatusSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuTarget | null>(null);
  const loadingDirsRef = useRef<Record<string, boolean>>({});

  const loadDir = useCallback(async (path: string) => {
    if (!stream || loadingDirsRef.current[path]) return;
    loadingDirsRef.current = { ...loadingDirsRef.current, [path]: true };
    setLoadingDirs((prev) => ({ ...prev, [path]: true }));
    try {
      const entries = await listWorkspaceEntries(stream.id, path);
      setEntriesByDir((prev) => ({ ...prev, [path]: entries }));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      loadingDirsRef.current = { ...loadingDirsRef.current, [path]: false };
      setLoadingDirs((prev) => ({ ...prev, [path]: false }));
    }
  }, [stream]);

  const loadWorkspaceIndex = useCallback(async () => {
    if (!stream) return;
    try {
      const result = await listWorkspaceFiles(stream.id);
      setIndexedFiles(result.files);
      setStatusSummary(result.summary);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [stream]);

  useEffect(() => {
    setExpandedDirs({ "": true });
    setEntriesByDir({});
    loadingDirsRef.current = {};
    setLoadingDirs({});
    setIndexedFiles([]);
    setStatusSummary(null);
    setError(null);
    setContextMenu(null);
  }, [stream?.id]);

  useEffect(() => {
    if (!stream) return;
    void loadDir("");
    void loadWorkspaceIndex();
  }, [stream?.id, loadDir, loadWorkspaceIndex]);

  useEffect(() => {
    if (!stream) return;
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const refreshForPath = (path: string) => {
      const parentDir = dirname(path);
      void loadWorkspaceIndex();
      void loadDir(parentDir);
    };
    const prunePath = (path: string) => {
      setEntriesByDir((prev) => Object.fromEntries(
        Object.entries(prev).filter(([candidate]) => candidate !== path && !candidate.startsWith(path + "/")),
      ));
      setExpandedDirs((prev) => Object.fromEntries(
        Object.entries(prev).filter(([candidate]) => candidate !== path && !candidate.startsWith(path + "/")),
      ));
    };
    const unsubscribe = subscribeWorkspaceEvents(stream.id, (event) => {
      if (event.kind === "updated") return;
      if (event.kind === "deleted") {
        prunePath(event.path);
      }
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => refreshForPath(event.path), 75);
    });
    return () => {
      unsubscribe();
      if (refreshTimer) clearTimeout(refreshTimer);
    };
  }, [loadDir, loadWorkspaceIndex, stream]);

  const rootEntries = useMemo(() => entriesByDir[""] ?? [], [entriesByDir]);
  const changedFiles = useMemo(() => indexedFiles.filter((file) => file.gitStatus !== null), [indexedFiles]);

  if (!stream) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>loading stream…</div>;
  }

  async function toggleDirectory(path: string) {
    const nextExpanded = !expandedDirs[path];
    setExpandedDirs((prev) => ({ ...prev, [path]: nextExpanded }));
    if (nextExpanded && !entriesByDir[path]) {
      await loadDir(path);
    }
  }

  async function handleContextAction(action: "open" | "new-file" | "new-folder" | "rename" | "delete" | "copy-path") {
    if (!contextMenu) return;
    try {
      switch (action) {
        case "open":
          onOpenFile(contextMenu.path);
          break;
        case "new-file": {
          const suggested = contextMenu.kind === "directory" ? joinChildPath(contextMenu.path, "new-file.txt") : joinChildPath(dirname(contextMenu.path), "new-file.txt");
          const nextPath = window.prompt("New file path", suggested)?.trim();
          if (!nextPath) return;
          await onCreateFile(nextPath);
          break;
        }
        case "new-folder": {
          const suggested = contextMenu.kind === "directory" ? joinChildPath(contextMenu.path, "new-folder") : joinChildPath(dirname(contextMenu.path), "new-folder");
          const nextPath = window.prompt("New folder path", suggested)?.trim();
          if (!nextPath) return;
          await onCreateDirectory(nextPath);
          break;
        }
        case "rename": {
          const nextPath = window.prompt("Rename path", contextMenu.path)?.trim();
          if (!nextPath || nextPath === contextMenu.path) return;
          await onRenamePath(contextMenu.path, nextPath);
          break;
        }
        case "delete":
          if (!window.confirm(`Delete ${contextMenu.path}?`)) return;
          await onDeletePath(contextMenu.path);
          break;
        case "copy-path":
          await copyText(contextMenu.path);
          break;
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setContextMenu(null);
    }
  }

  const contextMenuItems: MenuItem[] = contextMenu
    ? [
      ...(contextMenu.kind === "file"
        ? [{
            id: "files.open",
            label: "Open",
            enabled: true,
            run: () => handleContextAction("open"),
          }]
        : []),
      {
        id: "files.new-file",
        label: "New File…",
        enabled: true,
        run: () => handleContextAction("new-file"),
      },
      {
        id: "files.new-folder",
        label: "New Folder…",
        enabled: true,
        run: () => handleContextAction("new-folder"),
      },
      {
        id: "files.rename",
        label: "Rename…",
        enabled: true,
        run: () => handleContextAction("rename"),
      },
      {
        id: "files.delete",
        label: "Delete…",
        enabled: true,
        run: () => handleContextAction("delete"),
      },
      {
        id: "files.copy-path",
        label: "Copy Path",
        enabled: true,
        run: () => handleContextAction("copy-path"),
      },
    ]
    : [];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12 }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" }}>
        <SidebarButton active={activeTab === "batches"} onClick={() => onActiveTabChange("batches")}>Batches</SidebarButton>
        <SidebarButton active={activeTab === "files"} onClick={() => onActiveTabChange("files")}>Files</SidebarButton>
        <SidebarButton active={activeTab === "stream"} onClick={() => onActiveTabChange("stream")}>Stream</SidebarButton>
      </div>
      <div style={{ flex: 1, overflowX: "auto", overflowY: "auto", padding: 12 }}>
        {activeTab === "batches" ? (
          <BatchQueueSection
            batches={batches}
            batchWorkStates={batchWorkStates}
            agentStatuses={agentStatuses}
            selectedBatchId={selectedBatchId}
            activeBatchId={activeBatchId}
            onSelectBatch={onSelectBatch}
            onCreateBatch={onCreateBatch}
            onReorderBatch={onReorderBatch}
            onPromoteBatch={onPromoteBatch}
            onCompleteBatch={onCompleteBatch}
          />
        ) : activeTab === "files" ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: "100%", width: "max-content" }}>
            <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>
              {stream.branch}
            </div>
            {statusSummary ? <GitSummary summary={statusSummary} /> : null}
            {error ? <div style={{ color: "#ff6b6b" }}>{error}</div> : null}
            <div style={{ color: "var(--muted)", fontSize: 11 }}>Use File → Quick Open or Ctrl/Cmd+P to search by path.</div>
            {rootEntries.length === 0 && !loadingDirs[""] ? (
              <div style={{ color: "var(--muted)" }}>No files loaded yet.</div>
            ) : (
              <>
                {changedFiles.length > 0 ? (
                  <ChangedFilesSection
                    files={changedFiles.slice(0, 12)}
                    selectedFilePath={selectedFilePath}
                    onOpenFile={onOpenFile}
                    onContextMenu={setContextMenu}
                  />
                ) : null}
                <TreeEntries
                  parentPath=""
                  entries={rootEntries}
                  entriesByDir={entriesByDir}
                  expandedDirs={expandedDirs}
                  loadingDirs={loadingDirs}
                  selectedFilePath={selectedFilePath}
                  onToggleDirectory={toggleDirectory}
                  onOpenFile={onOpenFile}
                  onContextMenu={setContextMenu}
                />
              </>
            )}
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <Section title="Stream">
              <Row label="Name" value={stream.title} />
              <Row label="Branch" value={stream.branch} />
              <Row label="Source" value={stream.branch_source} />
              <Row label="Worktree" value={stream.worktree_path} />
            </Section>
            <Section title="Batches">
              <Row label="Count" value={String(batches.length)} />
              <Row label="Active" value={batches.find((batch) => batch.id === activeBatchId)?.title ?? "none"} />
              <Row label="Selected" value={batches.find((batch) => batch.id === selectedBatchId)?.title ?? "none"} />
            </Section>
          </div>
        )}
      </div>
      {contextMenu ? (
        <ContextMenu
          items={contextMenuItems}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={180}
        />
      ) : null}
    </div>
  );
}
