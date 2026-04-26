import { useRef } from "react";
import type { GitFileStatus, WorkspaceEntry, WorkspaceIndexedFile } from "../../api.js";
import { basename, StatusBadge, type ContextMenuTarget } from "./shared.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";

/**
 * `requestMenu` opens a menu anchored at the kebab's bottom-right
 * corner. The parent (ProjectPanel) renders the actual menu using
 * its existing ContextMenuTarget-keyed `contextMenuItems` builder.
 *
 * Phase 5 of the IA redesign retired the right-click trigger here in
 * favor of a visible kebab `⋯` button on every row — discovery beats
 * convention, and screen-reader users (or anyone without a real
 * mouse) can now reach every file action from the same affordance.
 */
function KebabButton({ onClick, label = "More actions" }: { onClick(rect: DOMRect): void; label?: string }) {
  const ref = useRef<HTMLButtonElement>(null);
  return (
    <button
      ref={ref}
      type="button"
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation();
        const rect = ref.current?.getBoundingClientRect();
        if (rect) onClick(rect);
      }}
      style={{
        background: "transparent",
        border: "none",
        color: "var(--muted)",
        cursor: "pointer",
        padding: "0 4px",
        fontSize: 14,
        lineHeight: 1,
        flexShrink: 0,
      }}
    >
      ⋯
    </button>
  );
}

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
            <div
              data-testid={`file-tree-entry-${entry.path}`}
              data-kind={entry.kind}
              data-expanded={entry.kind === "directory" ? String(expanded) : undefined}
              draggable={entry.kind === "file"}
              onDragStart={entry.kind === "file"
                ? (e) => setContextRefDrag(e, { kind: "file", path: entry.path })
                : undefined}
              onClick={() => {
                if (entry.kind === "directory") {
                  void onToggleDirectory(entry.path);
                } else if (entry.gitStatus === "deleted") {
                  // Deleted files no longer exist on disk; opening would 404.
                } else {
                  onOpenFile(entry.path);
                }
              }}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  if (entry.kind === "directory") void onToggleDirectory(entry.path);
                  else if (entry.gitStatus !== "deleted") onOpenFile(entry.path);
                }
              }}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                width: "100%",
                minWidth: "100%",
                padding: "7px 8px",
                border: "none",
                borderRadius: 4,
                background: selectedFilePath === entry.path ? "var(--accent-soft-bg)" : "transparent",
                color: selectedFilePath === entry.path ? "var(--text-primary)" : "inherit",
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
              <KebabButton
                onClick={(rect) => onContextMenu({
                  path: entry.path,
                  kind: entry.kind,
                  name: entry.name,
                  x: rect.right,
                  y: rect.bottom + 4,
                })}
              />
            </div>
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
    <div
      onClick={onClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      draggable
      onDragStart={(e) => setContextRefDrag(e, { kind: "file", path })}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        width: "100%",
        minWidth: "100%",
        padding: "7px 8px",
        border: "none",
        borderRadius: 4,
        background: active ? "var(--accent-soft-bg)" : "transparent",
        color: active ? "var(--text-primary)" : "inherit",
        cursor: "pointer",
        fontFamily: "inherit",
        textAlign: "left",
        whiteSpace: "nowrap",
      }}
    >
      <span>📄</span>
      <span style={{ flex: 1, whiteSpace: "nowrap" }}>{path}</span>
      {gitStatus ? <StatusBadge status={gitStatus} /> : null}
      <KebabButton
        onClick={(rect) => onContextMenu({
          path,
          kind: "file",
          name: basename(path),
          x: rect.right,
          y: rect.bottom + 4,
        })}
      />
    </div>
  );
}
