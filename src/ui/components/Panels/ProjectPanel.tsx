import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
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
import { ChangedFilesSection, TreeEntries } from "../LeftPanel/FileTree.js";
import { GitSummary } from "../LeftPanel/GitSummary.js";
import { copyText, dirname, joinChildPath, type ContextMenuTarget } from "../LeftPanel/shared.js";

interface Props {
  stream: Stream | null;
  gitEnabled: boolean;
  selectedFilePath: string | null;
  onOpenFile(path: string): void;
  onCreateFile(path: string): Promise<void>;
  onCreateDirectory(path: string): Promise<void>;
  onRenamePath(fromPath: string, toPath: string): Promise<void>;
  onDeletePath(path: string): Promise<void>;
}

export function ProjectPanel({
  stream,
  gitEnabled,
  selectedFilePath,
  onOpenFile,
  onCreateFile,
  onCreateDirectory,
  onRenamePath,
  onDeletePath,
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

  const [changedOnly, setChangedOnly] = useState(false);
  const rootEntries = useMemo(() => entriesByDir[""] ?? [], [entriesByDir]);
  const changedFiles = useMemo(() => indexedFiles.filter((file) => file.gitStatus !== null), [indexedFiles]);
  const changedPathSet = useMemo(() => {
    const set = new Set<string>();
    for (const file of changedFiles) {
      set.add(file.path);
      // Mark every ancestor dir as "has changes" so we can keep them visible
      // when the user toggles the filter.
      let dir = dirname(file.path);
      while (dir) {
        if (set.has(dir)) break;
        set.add(dir);
        dir = dirname(dir);
      }
    }
    return set;
  }, [changedFiles]);
  const effectiveChangedOnly = gitEnabled && changedOnly;
  const visibleEntriesByDir = useMemo(() => {
    if (!effectiveChangedOnly) return entriesByDir;
    const out: Record<string, typeof entriesByDir[string]> = {};
    for (const [dir, entries] of Object.entries(entriesByDir)) {
      out[dir] = entries.filter((entry) => changedPathSet.has(entry.path));
    }
    return out;
  }, [effectiveChangedOnly, entriesByDir, changedPathSet]);
  const visibleRootEntries = visibleEntriesByDir[""] ?? [];

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
          const suggested = contextMenu.kind === "directory"
            ? joinChildPath(contextMenu.path, "new-file.txt")
            : joinChildPath(dirname(contextMenu.path), "new-file.txt");
          const nextPath = window.prompt("New file path", suggested)?.trim();
          if (!nextPath) return;
          await onCreateFile(nextPath);
          break;
        }
        case "new-folder": {
          const suggested = contextMenu.kind === "directory"
            ? joinChildPath(contextMenu.path, "new-folder")
            : joinChildPath(dirname(contextMenu.path), "new-folder");
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
        ? [{ id: "files.open", label: "Open", enabled: true, run: () => handleContextAction("open") }]
        : []),
      { id: "files.new-file", label: "New File…", enabled: true, run: () => handleContextAction("new-file") },
      { id: "files.new-folder", label: "New Folder…", enabled: true, run: () => handleContextAction("new-folder") },
      { id: "files.rename", label: "Rename…", enabled: true, run: () => handleContextAction("rename") },
      { id: "files.delete", label: "Delete…", enabled: true, run: () => handleContextAction("delete") },
      { id: "files.copy-path", label: "Copy Path", enabled: true, run: () => handleContextAction("copy-path") },
    ]
    : [];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12, overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 12px", borderBottom: "1px solid var(--border)" }}>
        <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>{stream.branch}</div>
        {gitEnabled ? (
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, color: "var(--muted)", fontSize: 11, cursor: "pointer" }}>
            <input type="checkbox" checked={changedOnly} onChange={(e) => setChangedOnly(e.target.checked)} />
            Changed only
          </label>
        ) : null}
      </div>
      <div style={{ flex: 1, overflowX: "auto", overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 8, minWidth: "100%", width: "max-content" }}>
        {gitEnabled && statusSummary ? <GitSummary summary={statusSummary} /> : null}
        {error ? <div style={{ color: "#ff6b6b" }}>{error}</div> : null}
        <div style={{ color: "var(--muted)", fontSize: 11 }}>Use File → Quick Open or Ctrl/Cmd+P to search by path.</div>
        {rootEntries.length === 0 && !loadingDirs[""] ? (
          <div style={{ color: "var(--muted)" }}>No files loaded yet.</div>
        ) : (
          <>
            {gitEnabled && !effectiveChangedOnly && changedFiles.length > 0 ? (
              <ChangedFilesSection
                files={changedFiles.slice(0, 12)}
                selectedFilePath={selectedFilePath}
                onOpenFile={onOpenFile}
                onContextMenu={setContextMenu}
              />
            ) : null}
            {effectiveChangedOnly && changedFiles.length === 0 ? (
              <div style={{ color: "var(--muted)" }}>No git-changed files.</div>
            ) : null}
            <TreeEntries
              parentPath=""
              entries={visibleRootEntries}
              entriesByDir={visibleEntriesByDir}
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
