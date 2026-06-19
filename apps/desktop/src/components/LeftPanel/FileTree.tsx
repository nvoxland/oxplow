import type { GitFileStatus, WorkspaceEntry, WorkspaceIndexedFile } from "../../api.js";
import { PageKindIcon } from "../../pageKinds.js";
import { basename, StatusBadge, type ContextMenuTarget } from "./shared.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { fileRef } from "../../tabs/pageRefs.js";
import type { NavSiblings } from "../../tabs/PageNavigationContext.js";

/**
 * Mirror of the Rust `WorkspaceFilter` matching semantics for the
 * UI's visual "this row is generated" hint. Segment entries (no `/`)
 * match anywhere by component name; path entries (containing `/`)
 * match the exact path or a directory prefix.
 *
 * `selfOnly = true` returns true only when the row's exact path /
 * name is the marked entry (i.e. don't count "marked by ancestor").
 * `selfOnly = false` returns true if either the row itself or any
 * ancestor matches.
 */
function matchesGenerated(
  rowPath: string,
  rowName: string,
  generated: readonly string[],
  selfOnly: boolean,
): boolean {
  const segments = rowPath.split("/");
  for (const entry of generated) {
    if (!entry.includes("/")) {
      if (selfOnly) {
        if (entry === rowName) return true;
      } else if (segments.includes(entry)) {
        return true;
      }
    } else {
      if (selfOnly) {
        if (entry === rowPath) return true;
      } else if (rowPath === entry || rowPath.startsWith(entry + "/")) {
        return true;
      }
    }
  }
  return false;
}

export function ChangedFilesSection({
  files,
  selectedFilePath,
  onOpenFile,
  onOpenMenu,
}: {
  files: WorkspaceIndexedFile[];
  selectedFilePath: string | null;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
}) {
  // ChangedFilesSection lives inside the rail HUD's ProjectPanel, which
  // has no PageNavigationContext, so siblings would be a no-op. Skipped
  // until/unless this section moves into a Page.
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
          onClick={(e: React.MouseEvent | React.KeyboardEvent) => {
            const newTab =
              ("metaKey" in e && (e.metaKey || e.ctrlKey)) ||
              ("button" in e && e.button === 1) ||
              ("type" in e && e.type === "contextmenu");
            onOpenFile(file.path, { newTab });
          }}
          onOpenMenu={onOpenMenu}
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
  generated,
  onToggleDirectory,
  onOpenFile,
  onOpenMenu,
  parentPath,
}: {
  parentPath: string;
  entries: WorkspaceEntry[];
  entriesByDir: Record<string, WorkspaceEntry[]>;
  expandedDirs: Record<string, boolean>;
  loadingDirs: Record<string, boolean>;
  selectedFilePath: string | null;
  generated: readonly string[];
  onToggleDirectory(path: string): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
}) {
  // Siblings for file-row navigation: every file at this directory
  // level. Directories don't participate (they're not destinations).
  const siblingEntries = entries
    .filter((e) => e.kind === "file" && e.gitStatus !== "deleted")
    .map((e) => ({ ref: fileRef(e.path), label: e.path }));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: "100%", width: "max-content" }}>
      {entries.map((entry) => {
        const expanded = !!expandedDirs[entry.path];
        const children = entriesByDir[entry.path] ?? [];
        const fileSiblingIdx = entry.kind === "file"
          ? siblingEntries.findIndex((s) => s.ref.id === fileRef(entry.path).id)
          : -1;
        const rowSiblings: NavSiblings | undefined = fileSiblingIdx >= 0
          ? {
              entries: siblingEntries,
              index: fileSiblingIdx,
              title: parentPath ? `Files in ${parentPath}` : "Files at the project root",
            }
          : undefined;
        // "Marked" = this row's path (or, for legacy segment entries,
        // its name) is itself in the config list.
        // "Inside" = some ancestor matches, so this row is being
        // ignored by inheritance even if it isn't a marked path itself.
        // Mirrors the Rust WorkspaceFilter semantics: segment entries
        // (no `/`) match anywhere by component name; path entries
        // (containing `/`) match the exact path or a directory prefix.
        const markedSelf = entry.kind === "directory" && matchesGenerated(entry.path, entry.name, generated, true);
        const insideGenerated = matchesGenerated(entry.path, entry.name, generated, false);
        return (
          <div key={entry.path}>
            <TreeEntryRow
              entry={entry}
              expanded={expanded}
              insideGenerated={insideGenerated}
              markedSelf={markedSelf}
              selected={selectedFilePath === entry.path}
              siblings={rowSiblings}
              onToggleDirectory={onToggleDirectory}
              onOpenFile={onOpenFile}
              onOpenMenu={onOpenMenu}
            />
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
                    generated={generated}
                    onToggleDirectory={onToggleDirectory}
                    onOpenFile={onOpenFile}
                    onOpenMenu={onOpenMenu}
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

/**
 * Tree row for a file or directory entry. File rows route their click
 * through `useRouteDispatch` so plain-click does in-tab navigation
 * inside a page (Files page → file tab) and modifier/middle/right-
 * click open the file in a new tab. Outside a page (left rail), the
 * dispatch falls back to `onOpenFile`. Directory rows just toggle.
 */
function TreeEntryRow({
  entry,
  expanded,
  insideGenerated,
  markedSelf,
  selected,
  siblings,
  onToggleDirectory,
  onOpenFile,
  onOpenMenu,
}: {
  entry: WorkspaceEntry;
  expanded: boolean;
  insideGenerated: boolean;
  markedSelf: boolean;
  selected: boolean;
  siblings?: NavSiblings;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
}) {
  // Hook is called unconditionally; for directory rows the dispatch
  // ref is unused. fileRef is cheap to construct.
  const { handlers } = useRouteDispatch(fileRef(entry.path), {
    onNavigate: (_ref, opts) => onOpenFile(entry.path, opts),
    siblings,
  });
  const isFile = entry.kind === "file";
  const isOpenable = isFile && entry.gitStatus !== "deleted";
  return (
    <div
      data-testid={`file-tree-entry-${entry.path}`}
      data-kind={entry.kind}
      data-expanded={entry.kind === "directory" ? String(expanded) : undefined}
      // Typed context node so the right-click "Comment" action (rows are
      // draggable, so a drag-select can't start here) can anchor to this
      // file/directory. entry.kind is already "file" | "directory".
      data-ref-kind={entry.kind}
      data-ref-id={entry.path}
      title={entry.path}
      draggable={isFile}
      onDragStart={isFile
        ? (e) => setContextRefDrag(e, { kind: "file", path: entry.path })
        : undefined}
      onClick={(e) => {
        if (entry.kind === "directory") {
          void onToggleDirectory(entry.path);
          return;
        }
        if (!isOpenable) return;
        handlers.onClick(e);
      }}
      onAuxClick={(e) => {
        if (isOpenable) handlers.onAuxClick(e);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenMenu({ path: entry.path, kind: entry.kind, name: entry.name, x: e.clientX, y: e.clientY });
      }}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          onOpenMenu({ path: entry.path, kind: entry.kind, name: entry.name, x: rect.left, y: rect.bottom + 2 });
          return;
        }
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          if (entry.kind === "directory") void onToggleDirectory(entry.path);
          else if (isOpenable) onOpenFile(entry.path);
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
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        color: selected ? "var(--text-primary)" : "inherit",
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
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path d="M3 1.5 L7 5 L3 8.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : null}
      </span>
      <PageKindIcon
        kind={entry.kind === "directory" ? "directory" : "file"}
        size={13}
        style={{ color: "var(--text-secondary)", flexShrink: 0 }}
      />
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
    </div>
  );
}

function FileRow({
  path,
  gitStatus,
  active,
  onClick,
  onOpenMenu,
}: {
  path: string;
  gitStatus: GitFileStatus | null;
  active: boolean;
  onClick(e: React.MouseEvent | React.KeyboardEvent): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
}) {
  return (
    <div
      onClick={onClick}
      title={path}
      onAuxClick={(e) => {
        if (e.button === 1) {
          e.preventDefault();
          onClick(e);
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenMenu({ path, kind: "file", name: basename(path), x: e.clientX, y: e.clientY });
      }}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          onOpenMenu({ path, kind: "file", name: basename(path), x: rect.left, y: rect.bottom + 2 });
          return;
        }
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick(e);
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
      <PageKindIcon kind="file" size={13} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
      <span style={{ flex: 1, whiteSpace: "nowrap" }}>{path}</span>
      {gitStatus ? <StatusBadge status={gitStatus} /> : null}
    </div>
  );
}
