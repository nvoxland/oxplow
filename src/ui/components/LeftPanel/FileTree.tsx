import type { GitFileStatus, WorkspaceEntry, WorkspaceIndexedFile } from "../../api.js";
import { basename, StatusBadge, type ContextMenuTarget } from "./shared.js";

export function ChangedFilesSection({
  files,
  selectedFilePath,
  onOpenFile,
  onContextMenu,
}: {
  files: WorkspaceIndexedFile[];
  selectedFilePath: string | null;
  onOpenFile(path: string): void;
  onContextMenu(target: ContextMenuTarget | null): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: "100%" }}>
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
          onContextMenu={onContextMenu}
        />
      ))}
    </div>
  );
}

export function TreeEntries({
  entries,
  entriesByDir,
  expandedDirs,
  loadingDirs,
  selectedFilePath,
  onToggleDirectory,
  onOpenFile,
  onContextMenu,
}: {
  parentPath: string;
  entries: WorkspaceEntry[];
  entriesByDir: Record<string, WorkspaceEntry[]>;
  expandedDirs: Record<string, boolean>;
  loadingDirs: Record<string, boolean>;
  selectedFilePath: string | null;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string): void;
  onContextMenu(target: ContextMenuTarget | null): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: "100%", width: "max-content" }}>
      {entries.map((entry) => {
        const expanded = !!expandedDirs[entry.path];
        const children = entriesByDir[entry.path] ?? [];
        return (
          <div key={entry.path}>
            <button
              onClick={() => entry.kind === "directory" ? void onToggleDirectory(entry.path) : onOpenFile(entry.path)}
              onContextMenu={(event) => {
                event.preventDefault();
                onContextMenu({
                  path: entry.path,
                  kind: entry.kind,
                  name: entry.name,
                  x: event.clientX,
                  y: event.clientY,
                });
              }}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                width: "100%",
                minWidth: "100%",
                padding: "4px 6px",
                border: "none",
                borderRadius: 4,
                background: selectedFilePath === entry.path ? "rgba(74, 158, 255, 0.18)" : "transparent",
                color: selectedFilePath === entry.path ? "var(--fg)" : "inherit",
                cursor: "pointer",
                fontFamily: "inherit",
                textAlign: "left",
                whiteSpace: "nowrap",
              }}
            >
              <span style={{ width: 12, color: "var(--muted)" }}>
                {entry.kind === "directory" ? (expanded ? "▾" : "▸") : ""}
              </span>
              <span>{entry.kind === "directory" ? "📁" : "📄"}</span>
              <span style={{ flex: 1, whiteSpace: "nowrap" }}>{entry.name}</span>
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
                    onContextMenu={onContextMenu}
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
  onContextMenu,
}: {
  path: string;
  gitStatus: GitFileStatus | null;
  active: boolean;
  onClick(): void;
  onContextMenu(target: ContextMenuTarget | null): void;
}) {
  return (
    <button
      onClick={onClick}
      onContextMenu={(event) => {
        event.preventDefault();
        onContextMenu({
          path,
          kind: "file",
          name: basename(path),
          x: event.clientX,
          y: event.clientY,
        });
      }}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        width: "100%",
        minWidth: "100%",
        padding: "4px 6px",
        border: "none",
        borderRadius: 4,
        background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
        color: active ? "var(--fg)" : "inherit",
        cursor: "pointer",
        fontFamily: "inherit",
        textAlign: "left",
        whiteSpace: "nowrap",
      }}
    >
      <span>📄</span>
      <span style={{ flex: 1, whiteSpace: "nowrap" }}>{path}</span>
      {gitStatus ? <StatusBadge status={gitStatus} /> : null}
    </button>
  );
}
