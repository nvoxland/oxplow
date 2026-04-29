import { useRef } from "react";
import type { GitFileStatus, WorkspaceEntry, WorkspaceIndexedFile } from "../../api.js";
import { basename, StatusBadge, type ContextMenuTarget } from "./shared.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { fileRef } from "../../tabs/pageRefs.js";

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
  onOpenMenu,
}: {
  files: WorkspaceIndexedFile[];
  selectedFilePath: string | null;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
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
  generatedSet,
  onToggleDirectory,
  onOpenFile,
  onOpenMenu,
}: {
  parentPath: string;
  entries: WorkspaceEntry[];
  entriesByDir: Record<string, WorkspaceEntry[]>;
  expandedDirs: Record<string, boolean>;
  loadingDirs: Record<string, boolean>;
  selectedFilePath: string | null;
  generatedSet: Set<string>;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
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
            <TreeEntryRow
              entry={entry}
              expanded={expanded}
              insideGenerated={insideGenerated}
              markedSelf={markedSelf}
              selected={selectedFilePath === entry.path}
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
                    generatedSet={generatedSet}
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
  onToggleDirectory,
  onOpenFile,
  onOpenMenu,
}: {
  entry: WorkspaceEntry;
  expanded: boolean;
  insideGenerated: boolean;
  markedSelf: boolean;
  selected: boolean;
  onToggleDirectory(path: string): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenMenu(target: ContextMenuTarget | null): void;
}) {
  // Hook is called unconditionally; for directory rows the dispatch
  // ref is unused. fileRef is cheap to construct.
  const { handlers } = useRouteDispatch(fileRef(entry.path), {
    onNavigate: (_ref, opts) => onOpenFile(entry.path, opts),
  });
  const isFile = entry.kind === "file";
  const isOpenable = isFile && entry.gitStatus !== "deleted";
  return (
    <div
      data-testid={`file-tree-entry-${entry.path}`}
      data-kind={entry.kind}
      data-expanded={entry.kind === "directory" ? String(expanded) : undefined}
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
        if (isOpenable) handlers.onContextMenu(e);
      }}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
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
        onClick={(rect) => onOpenMenu({
          path: entry.path,
          kind: entry.kind,
          name: entry.name,
          x: rect.right,
          y: rect.bottom + 4,
        })}
      />
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
        onClick(e);
      }}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
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
      <span>📄</span>
      <span style={{ flex: 1, whiteSpace: "nowrap" }}>{path}</span>
      {gitStatus ? <StatusBadge status={gitStatus} /> : null}
      <KebabButton
        onClick={(rect) => onOpenMenu({
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
