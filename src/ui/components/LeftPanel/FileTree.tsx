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
  generatedSet,
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
  generatedSet: Set<string>;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string): void;
  onContextMenu(target: ContextMenuTarget | null): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: "100%", width: "max-content" }}>
      {entries.map((entry) => {
        const expanded = !!expandedDirs[entry.path];
        const children = entriesByDir[entry.path] ?? [];
        // "Marked" = this directory's name itself is in the config list.
        // "Inside" = some ancestor segment matches, so this path is being
        // ignored by inheritance even if its own name isn't in the list.
        const markedSelf = entry.kind === "directory" && generatedSet.has(entry.name);
        const insideGenerated = entry.path.split("/").some((seg) => generatedSet.has(seg));
        return (
          <div key={entry.path}>
            <button type="button"
              data-testid={`file-tree-entry-${entry.path}`}
              data-kind={entry.kind}
              data-expanded={entry.kind === "directory" ? String(expanded) : undefined}
              onClick={() => {
                if (entry.kind === "directory") {
                  void onToggleDirectory(entry.path);
                } else if (entry.gitStatus === "deleted") {
                  // Deleted files no longer exist on disk; opening would 404.
                } else {
                  onOpenFile(entry.path);
                }
              }}
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
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 16,
                  height: 16,
                  color: "var(--muted)",
                  flexShrink: 0,
                  transition: "transform 120ms ease, color 120ms ease",
                  transform: entry.kind === "directory" && expanded ? "rotate(90deg)" : "rotate(0deg)",
                }}
              >
                {entry.kind === "directory" ? (
                  // Chevron — rotated via transform so the open/closed states
                  // share one glyph and animate smoothly.
                  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
                    <path d="M3 1.5 L7 5 L3 8.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                ) : null}
              </span>
              <span>{entry.kind === "directory" ? "📁" : "📄"}</span>
              <span
                style={{
                  flex: 1,
                  whiteSpace: "nowrap",
                  textDecoration: entry.gitStatus === "deleted" ? "line-through" : undefined,
                  color:
                    entry.gitStatus === "deleted"
                      ? "var(--muted)"
                      : insideGenerated
                        ? "var(--muted)"
                        : undefined,
                  fontStyle: insideGenerated ? "italic" : undefined,
                }}
              >{entry.name}</span>
              {markedSelf ? (
                <span
                  title="Marked as generated — excluded from fs-watch and snapshot tracking"
                  style={{
                    fontSize: 9,
                    fontWeight: 600,
                    letterSpacing: 0.4,
                    padding: "0 4px",
                    border: "1px solid #e5a06a",
                    color: "#e5a06a",
                    borderRadius: 3,
                    flexShrink: 0,
                  }}
                >
                  GEN
                </span>
              ) : null}
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
                    generatedSet={generatedSet}
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
    <button type="button"
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
