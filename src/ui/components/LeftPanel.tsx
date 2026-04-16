import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  listWorkspaceEntries,
  listWorkspaceFiles,
  subscribeWorkspaceEvents,
  type GitFileStatus,
  type Stream,
  type WorkspaceEntry,
  type WorkspaceIndexedFile,
  type WorkspaceStatusSummary,
} from "../api.js";

export type SidebarTab = "files" | "stream";

interface Props {
  stream: Stream | null;
  activeTab: SidebarTab;
  onActiveTabChange(tab: SidebarTab): void;
  selectedFilePath: string | null;
  onOpenFile(path: string): void;
}

export function LeftPanel({ stream, activeTab, onActiveTabChange, selectedFilePath, onOpenFile }: Props) {
  const [expandedDirs, setExpandedDirs] = useState<Record<string, boolean>>({ "": true });
  const [entriesByDir, setEntriesByDir] = useState<Record<string, WorkspaceEntry[]>>({});
  const [loadingDirs, setLoadingDirs] = useState<Record<string, boolean>>({});
  const [indexedFiles, setIndexedFiles] = useState<WorkspaceIndexedFile[]>([]);
  const [statusSummary, setStatusSummary] = useState<WorkspaceStatusSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadDir = useCallback(async (path: string) => {
    if (!stream || loadingDirs[path]) return;
    setLoadingDirs((prev) => ({ ...prev, [path]: true }));
    try {
      const entries = await listWorkspaceEntries(stream.id, path);
      setEntriesByDir((prev) => ({ ...prev, [path]: entries }));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingDirs((prev) => ({ ...prev, [path]: false }));
    }
  }, [loadingDirs, stream]);

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
    setLoadingDirs({});
    setIndexedFiles([]);
    setStatusSummary(null);
    setError(null);
  }, [stream?.id]);

  useEffect(() => {
    if (!stream) return;
    void loadDir("");
    void loadWorkspaceIndex();
  }, [stream?.id, loadDir, loadWorkspaceIndex]);

  useEffect(() => {
    if (!stream) return;
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const refresh = () => {
      void loadWorkspaceIndex();
      for (const [path, expanded] of Object.entries(expandedDirs)) {
        if (expanded) {
          void loadDir(path);
        }
      }
    };
    const unsubscribe = subscribeWorkspaceEvents(stream.id, () => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(refresh, 75);
    });
    return () => {
      unsubscribe();
      if (refreshTimer) clearTimeout(refreshTimer);
    };
  }, [expandedDirs, loadDir, loadWorkspaceIndex, stream]);

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

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12 }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" }}>
        <SidebarButton active={activeTab === "files"} onClick={() => onActiveTabChange("files")}>Files</SidebarButton>
        <SidebarButton active={activeTab === "stream"} onClick={() => onActiveTabChange("stream")}>Stream</SidebarButton>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: 12 }}>
        {activeTab === "files" ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
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
            <Section title="Claude resume">
              <Row label="Working" value={stream.resume.working_session_id || "not started yet"} />
              <Row label="Talking" value={stream.resume.talking_session_id || "not started yet"} />
            </Section>
            <Section title="Panes">
              <Row label="Working" value={stream.panes.working} />
              <Row label="Talking" value={stream.panes.talking} />
            </Section>
          </div>
        )}
      </div>
    </div>
  );
}

function ChangedFilesSection({
  files,
  selectedFilePath,
  onOpenFile,
}: {
  files: WorkspaceIndexedFile[];
  selectedFilePath: string | null;
  onOpenFile(path: string): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>
        Changed
      </div>
      {files.map((file) => (
        <FileRow
          key={file.path}
          path={file.path}
          gitStatus={file.gitStatus}
          active={selectedFilePath === file.path}
          onClick={() => onOpenFile(file.path)}
        />
      ))}
    </div>
  );
}

function TreeEntries({
  entries,
  entriesByDir,
  expandedDirs,
  loadingDirs,
  selectedFilePath,
  onToggleDirectory,
  onOpenFile,
}: {
  parentPath: string;
  entries: WorkspaceEntry[];
  entriesByDir: Record<string, WorkspaceEntry[]>;
  expandedDirs: Record<string, boolean>;
  loadingDirs: Record<string, boolean>;
  selectedFilePath: string | null;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {entries.map((entry) => {
        const expanded = !!expandedDirs[entry.path];
        const children = entriesByDir[entry.path] ?? [];
        return (
          <div key={entry.path}>
            <button
              onClick={() => entry.kind === "directory" ? void onToggleDirectory(entry.path) : onOpenFile(entry.path)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                width: "100%",
                padding: "4px 6px",
                border: "none",
                borderRadius: 4,
                background: selectedFilePath === entry.path ? "rgba(74, 158, 255, 0.18)" : "transparent",
                color: selectedFilePath === entry.path ? "var(--fg)" : "inherit",
                cursor: "pointer",
                fontFamily: "inherit",
                textAlign: "left",
              }}
            >
              <span style={{ width: 12, color: "var(--muted)" }}>
                {entry.kind === "directory" ? (expanded ? "▾" : "▸") : ""}
              </span>
              <span>{entry.kind === "directory" ? "📁" : "📄"}</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{entry.name}</span>
              {entry.hasChanges || entry.gitStatus ? <StatusBadge status={entry.gitStatus} /> : null}
            </button>
            {entry.kind === "directory" && expanded ? (
              <div style={{ paddingLeft: 18 }}>
                {loadingDirs[entry.path] && children.length === 0 ? (
                  <div style={{ color: "var(--muted)", padding: "2px 6px" }}>loading…</div>
                ) : (
                  <TreeEntries
                    parentPath={entry.path}
                    entries={children}
                    entriesByDir={entriesByDir}
                    expandedDirs={expandedDirs}
                    loadingDirs={loadingDirs}
                    selectedFilePath={selectedFilePath}
                    onToggleDirectory={onToggleDirectory}
                    onOpenFile={onOpenFile}
                  />
                )}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

function FileRow({
  path,
  gitStatus,
  active,
  onClick,
}: {
  path: string;
  gitStatus: GitFileStatus | null;
  active: boolean;
  onClick(): void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        width: "100%",
        padding: "4px 6px",
        border: "none",
        borderRadius: 4,
        background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
        color: active ? "var(--fg)" : "inherit",
        cursor: "pointer",
        fontFamily: "inherit",
        textAlign: "left",
      }}
    >
      <span>📄</span>
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{path}</span>
      {gitStatus ? <StatusBadge status={gitStatus} /> : null}
    </button>
  );
}

function StatusBadge({ status }: { status: GitFileStatus | null }) {
  const color = statusColor(status);
  return (
    <span style={{ color, fontSize: 11, flexShrink: 0 }}>
      {status === null ? "●" : shortStatus(status)}
    </span>
  );
}

function shortStatus(status: GitFileStatus): string {
  switch (status) {
    case "modified": return "M";
    case "added": return "A";
    case "deleted": return "D";
    case "renamed": return "R";
    case "untracked": return "U";
  }
}

function statusColor(status: GitFileStatus | null): string {
  switch (status) {
    case "modified": return "#fcd34d";
    case "added": return "#86efac";
    case "deleted": return "#fca5a5";
    case "renamed": return "#c4b5fd";
    case "untracked": return "#7dd3fc";
    default: return "#fcd34d";
  }
}

function GitSummary({ summary }: { summary: WorkspaceStatusSummary }) {
  return (
    <div style={{ display: "flex", gap: 8, flexWrap: "wrap", color: "var(--muted)", fontSize: 11 }}>
      <span>{summary.total} changed</span>
      {summary.modified > 0 ? <span style={{ color: statusColor("modified") }}>M {summary.modified}</span> : null}
      {summary.added > 0 ? <span style={{ color: statusColor("added") }}>A {summary.added}</span> : null}
      {summary.deleted > 0 ? <span style={{ color: statusColor("deleted") }}>D {summary.deleted}</span> : null}
      {summary.renamed > 0 ? <span style={{ color: statusColor("renamed") }}>R {summary.renamed}</span> : null}
      {summary.untracked > 0 ? <span style={{ color: statusColor("untracked") }}>U {summary.untracked}</span> : null}
    </div>
  );
}

function SidebarButton({ active, onClick, children }: { active: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        flex: 1,
        padding: "8px 12px",
        border: "none",
        borderBottom: active ? "2px solid var(--accent)" : "2px solid transparent",
        background: active ? "var(--bg)" : "transparent",
        color: active ? "var(--fg)" : "var(--muted)",
        cursor: "pointer",
        fontFamily: "inherit",
      }}
    >
      {children}
    </button>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.6, fontSize: 11 }}>
        {title}
      </div>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ color: "var(--muted)" }}>{label}</div>
      <div style={{ wordBreak: "break-word" }}>{value}</div>
    </div>
  );
}
