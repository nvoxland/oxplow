import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AgentStatus, BacklogState, FinishedEntry, ThreadWorkState, WorkItem } from "../../api.js";
import type { TabRef } from "../../tabs/tabState.js";
import { fileRef, noteRef, planWorkRef, uncommittedChangesRef, workItemRef } from "../../tabs/pageRefs.js";
import { computePagesDirectory, RAIL_PAGE_IDS } from "./sections.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { computeActiveItem, computeUpNext, sortRecentFiles, type RecentFileEntry } from "./sections.js";

export interface UncommittedSummary {
  added: number;
  modified: number;
  deleted: number;
  additions: number;
  deletions: number;
}

export interface BookmarkRailEntry {
  ref: TabRef;
  label: string;
  /** Single-letter scope marker rendered as a small badge (T/S/G). */
  scopeBadge: "T" | "S" | "G";
  /** Called when the user wants to remove the bookmark from this scope. */
  onRemove(): void;
}

export interface RailHudProps {
  threadId: string | null;
  threadWork: ThreadWorkState | null;
  backlog: BacklogState | null;
  agentStatus: AgentStatus;
  recentFiles: RecentFileEntry[];
  bookmarks?: BookmarkRailEntry[];
  /** Most recently finished work — closed work item efforts merged
   *  with updated wiki notes, sorted by timestamp DESC. */
  recentlyFinished?: FinishedEntry[];
  /** Working-tree uncommitted summary; section hidden when null or empty. */
  uncommitted?: UncommittedSummary | null;
  /** Mark all currently-finished entries as seen (clears the section). */
  onClearFinished?(): void;
  /** Open a page (or focus if already open) in the active thread's tab area. */
  onOpenPage(ref: TabRef): void;
  /** Optional: invoked when the user clicks the search affordance. */
  onOpenSearch?(): void;
}

/**
 * Heads-up display rail. Always visible on the left; passive by design —
 * never auto-opens tabs. Sections only render when they have content.
 *
 * - Search button (placeholder for ⌘K palette)
 * - Active item summary
 * - Since you last looked  (TBD; placeholder for now)
 * - Up next
 * - Recent files
 * - Pages directory
 */
export function RailHud({
  threadId,
  threadWork,
  backlog,
  agentStatus,
  recentFiles,
  bookmarks,
  recentlyFinished,
  uncommitted,
  onClearFinished,
  onOpenPage,
  onOpenSearch,
}: RailHudProps) {
  const activeItem = useMemo(() => computeActiveItem(threadWork), [threadWork]);
  const upNext = useMemo(() => computeUpNext(threadWork, 3), [threadWork]);
  const recents = useMemo(() => sortRecentFiles(recentFiles, 6), [recentFiles]);
  const backlogReadyCount = backlog?.items.filter((i) => i.status === "ready").length ?? 0;
  const width = useRailWidth();

  return (
    <aside
      data-testid="rail-hud"
      style={{
        width: width.value,
        flexShrink: 0,
        height: "100%",
        background: "var(--surface-rail)",
        borderRight: "1px solid var(--border-subtle)",
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        overflow: "hidden",
        position: "relative",
      }}
    >
      <div style={{ flex: 1, overflow: "auto", display: "flex", flexDirection: "column", minHeight: 0 }}>
      <SearchTrigger onOpenSearch={onOpenSearch} />

      {threadId ? (
        <ActiveItemSection
          item={activeItem}
          agentStatus={agentStatus}
          onOpenPage={onOpenPage}
        />
      ) : null}

      {uncommitted && (uncommitted.added + uncommitted.modified + uncommitted.deleted) > 0 ? (
        <UncommittedSection summary={uncommitted} onOpenPage={onOpenPage} />
      ) : null}

      {upNext.length > 0 ? (
        <UpNextSection items={upNext} onOpenPage={onOpenPage} />
      ) : null}

      {recents.length > 0 ? (
        <RecentFilesSection entries={recents} onOpenPage={onOpenPage} />
      ) : null}

      {recentlyFinished && recentlyFinished.length > 0 ? (
        <FinishedSection
          entries={recentlyFinished}
          onOpenPage={onOpenPage}
          onClear={onClearFinished}
        />
      ) : null}

      {bookmarks && bookmarks.length > 0 ? (
        <BookmarksSection entries={bookmarks} onOpenPage={onOpenPage} />
      ) : null}

      <PagesDirectory onOpenPage={onOpenPage} backlogReadyCount={backlogReadyCount} />
      </div>
      <RailResizeHandle onChange={width.setFromDelta} />
    </aside>
  );
}

const RAIL_MIN_WIDTH = 260;
const RAIL_MAX_WIDTH = 600;
const RAIL_WIDTH_KEY = "oxplow.railWidth";

function useRailWidth() {
  const [value, setValue] = useState<number>(() => {
    if (typeof window === "undefined") return RAIL_MIN_WIDTH;
    const raw = window.localStorage.getItem(RAIL_WIDTH_KEY);
    const parsed = raw ? parseInt(raw, 10) : NaN;
    return Number.isFinite(parsed) ? clampRailWidth(parsed) : RAIL_MIN_WIDTH;
  });
  const startRef = useRef<{ start: number; base: number } | null>(null);
  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(RAIL_WIDTH_KEY, String(value));
  }, [value]);
  const setFromDelta = useCallback((phase: "start" | "move" | "end", clientX: number) => {
    if (phase === "start") {
      startRef.current = { start: clientX, base: value };
      return;
    }
    if (!startRef.current) return;
    if (phase === "move") {
      const next = clampRailWidth(startRef.current.base + (clientX - startRef.current.start));
      setValue(next);
    } else if (phase === "end") {
      startRef.current = null;
    }
  }, [value]);
  return { value, setFromDelta };
}

function clampRailWidth(n: number) {
  return Math.max(RAIL_MIN_WIDTH, Math.min(RAIL_MAX_WIDTH, Math.round(n)));
}

function RailResizeHandle({ onChange }: { onChange(phase: "start" | "move" | "end", clientX: number): void }) {
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
    onChange("start", e.clientX);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.buttons === 0) return;
    onChange("move", e.clientX);
  };
  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    onChange("end", e.clientX);
    try { (e.currentTarget as HTMLDivElement).releasePointerCapture(e.pointerId); } catch {}
  };
  return (
    <div
      data-testid="rail-resize-handle"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      style={{
        position: "absolute",
        top: 0,
        right: 0,
        width: 6,
        height: "100%",
        cursor: "col-resize",
        userSelect: "none",
        zIndex: 5,
      }}
    />
  );
}

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: "12px 14px 4px",
        fontSize: 11,
        fontWeight: 600,
        color: "var(--text-secondary)",
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {children}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 14px",
  fontSize: 13,
  color: "var(--text-primary)",
  cursor: "pointer",
  border: "none",
  background: "transparent",
  textAlign: "left",
  width: "100%",
  borderRadius: 0,
};

function rowHoverStyle(): CSSProperties {
  return { ...rowStyle };
}

function SearchTrigger({ onOpenSearch }: { onOpenSearch?: () => void }) {
  return (
    <div style={{ padding: "12px 12px 8px" }}>
      <button
        type="button"
        data-testid="rail-search"
        onClick={onOpenSearch}
        style={{
          width: "100%",
          padding: "8px 10px",
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          color: "var(--text-secondary)",
          fontSize: 13,
          textAlign: "left",
          cursor: onOpenSearch ? "pointer" : "default",
          display: "flex",
          alignItems: "center",
          gap: 8,
        }}
      >
        <span aria-hidden style={{ opacity: 0.7 }}>🔍</span>
        <span style={{ flex: 1 }}>Search…</span>
        <kbd
          style={{
            fontSize: 10,
            color: "var(--text-muted)",
            background: "var(--surface-tab-inactive)",
            padding: "1px 5px",
            borderRadius: 3,
            border: "1px solid var(--border-subtle)",
          }}
        >
          ⌘K
        </kbd>
      </button>
    </div>
  );
}

function ActiveItemSection({
  item,
  agentStatus,
  onOpenPage,
}: {
  item: WorkItem | null;
  agentStatus: AgentStatus;
  onOpenPage(ref: TabRef): void;
}) {
  if (!item) {
    const working = agentStatus === "working";
    return (
      <>
        <SectionHeading>Current Work</SectionHeading>
        <div
          data-testid="rail-active-empty"
          style={{
            padding: "4px 14px 12px",
            fontSize: 12,
            color: "var(--text-muted)",
            display: "flex",
            alignItems: "center",
            gap: 6,
          }}
        >
          <AgentStatusDot status={agentStatus} />
          <span>{working ? "Working..." : "No item in progress."}</span>
        </div>
      </>
    );
  }
  return (
    <>
      <SectionHeading>Current Work</SectionHeading>
      <button
        type="button"
        data-testid="rail-active-item"
        onClick={() => onOpenPage(workItemRef(item.id))}
        draggable
        onDragStart={(ev) => setContextRefDrag(ev, {
          kind: "work-item",
          itemId: item.id,
          title: item.title,
          status: item.status,
        })}
        style={{
          ...rowStyle,
          flexDirection: "column",
          alignItems: "stretch",
          padding: "4px 14px 12px",
        }}
      >
        <span
          style={{
            color: "var(--text-primary)",
            fontWeight: 500,
            fontSize: 13,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {item.title}
        </span>
      </button>
    </>
  );
}

function UncommittedSection({
  summary,
  onOpenPage,
}: {
  summary: UncommittedSummary;
  onOpenPage(ref: TabRef): void;
}) {
  const parts: string[] = [];
  if (summary.added > 0) parts.push(`${summary.added}A`);
  if (summary.modified > 0) parts.push(`${summary.modified}M`);
  if (summary.deleted > 0) parts.push(`${summary.deleted}D`);
  return (
    <>
      <SectionHeading>Uncommitted</SectionHeading>
      <button
        type="button"
        data-testid="rail-uncommitted"
        onClick={() => onOpenPage(uncommittedChangesRef())}
        title="Open uncommitted changes"
        style={{
          ...rowStyle,
          padding: "4px 14px 12px",
          gap: 8,
        }}
      >
        <span style={{ color: "var(--text-primary)", fontSize: 12 }}>
          {parts.join(" · ")}
        </span>
        <span style={{ flex: 1 }} />
        <span style={{ color: "var(--diff-add-fg, #2ea043)", fontSize: 11 }}>
          +{summary.additions}
        </span>
        <span style={{ color: "var(--diff-del-fg, #f85149)", fontSize: 11 }}>
          −{summary.deletions}
        </span>
      </button>
    </>
  );
}

function UpNextSection({
  items,
  onOpenPage,
}: {
  items: WorkItem[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Up next</SectionHeading>
      <div data-testid="rail-up-next" style={{ paddingBottom: 8 }}>
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            data-testid={`rail-up-next-item-${item.id}`}
            title={item.title}
            onClick={() => onOpenPage(workItemRef(item.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "work-item",
              itemId: item.id,
              title: item.title,
              status: item.status,
            })}
            style={rowHoverStyle()}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {item.title}
            </span>
          </button>
        ))}
      </div>
    </>
  );
}

function BookmarksSection({
  entries,
  onOpenPage,
}: {
  entries: BookmarkRailEntry[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Bookmarks</SectionHeading>
      <div data-testid="rail-bookmarks" style={{ paddingBottom: 8 }}>
        {entries.map((entry) => (
          <div
            key={entry.ref.id}
            style={{ display: "flex", alignItems: "center", gap: 4, paddingRight: 6 }}
          >
            <button
              type="button"
              data-testid={`rail-bookmark-${entry.ref.id}`}
              title={entry.label}
              onClick={() => onOpenPage(entry.ref)}
              style={{ ...rowHoverStyle(), flex: 1 }}
            >
              <span aria-hidden style={{ color: "var(--accent-fg)", fontSize: 11 }}>★</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {entry.label}
              </span>
              <span
                title={
                  entry.scopeBadge === "T" ? "Thread bookmark"
                    : entry.scopeBadge === "S" ? "Stream bookmark"
                    : "Global bookmark"
                }
                style={{
                  fontSize: 9,
                  fontWeight: 600,
                  color: "var(--text-secondary)",
                  background: "var(--surface-tab-inactive)",
                  padding: "1px 4px",
                  borderRadius: 3,
                }}
              >
                {entry.scopeBadge}
              </span>
            </button>
            <button
              type="button"
              data-testid={`rail-bookmark-remove-${entry.ref.id}`}
              title="Remove bookmark"
              onClick={(e) => {
                e.stopPropagation();
                entry.onRemove();
              }}
              style={{
                background: "transparent",
                border: "none",
                color: "var(--text-secondary)",
                cursor: "pointer",
                padding: "2px 4px",
                fontSize: 11,
              }}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </>
  );
}

function RecentFilesSection({
  entries,
  onOpenPage,
}: {
  entries: RecentFileEntry[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Recent files</SectionHeading>
      <div data-testid="rail-recent-files" style={{ paddingBottom: 8 }}>
        {entries.map((e) => {
          const basename = e.path.split("/").pop() ?? e.path;
          return (
            <button
              key={e.path}
              type="button"
              data-testid={`rail-recent-file-${e.path}`}
              title={e.path}
              onClick={() => onOpenPage(fileRef(e.path))}
              draggable
              onDragStart={(ev) => setContextRefDrag(ev, { kind: "file", path: e.path })}
              style={rowHoverStyle()}
            >
              <span style={{ color: "var(--text-muted)", fontSize: 11 }}>📄</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {basename}
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}

function FinishedSection({
  entries,
  onOpenPage,
  onClear,
}: {
  entries: FinishedEntry[];
  onOpenPage(ref: TabRef): void;
  onClear?(): void;
}) {
  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "12px 14px 4px",
        }}
      >
        <button
          type="button"
          data-testid="rail-finished-heading"
          onClick={() => onOpenPage(planWorkRef())}
          title="Open Plan work"
          style={{
            flex: 1,
            background: "transparent",
            border: "none",
            padding: 0,
            margin: 0,
            textAlign: "left",
            cursor: "pointer",
            fontSize: 11,
            fontWeight: 600,
            color: "var(--text-secondary)",
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          Finished
        </button>
        {onClear ? (
          <button
            type="button"
            data-testid="rail-finished-clear"
            onClick={(e) => { e.stopPropagation(); onClear(); }}
            title="Mark all as seen"
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-secondary)",
              cursor: "pointer",
              fontSize: 10,
              padding: "0 4px",
            }}
          >
            clear
          </button>
        ) : null}
      </div>
      <div data-testid="rail-finished" style={{ paddingBottom: 8 }}>
        {entries.map((e) => {
          const ref = e.kind === "work-item" ? workItemRef(e.itemId) : noteRef(e.slug);
          return (
            <button
              key={`${e.kind}:${e.kind === "work-item" ? e.itemId : e.slug}`}
              type="button"
              data-testid={`rail-finished-${e.kind === "work-item" ? e.itemId : e.slug}`}
              title={e.title}
              onClick={() => onOpenPage(ref)}
              style={rowHoverStyle()}
            >
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {e.title}
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}

function PagesDirectory({
  onOpenPage,
  backlogReadyCount,
}: {
  onOpenPage(ref: TabRef): void;
  backlogReadyCount: number;
}) {
  const entries = computePagesDirectory({ backlogReadyCount }).filter((e) => RAIL_PAGE_IDS.has(e.id));
  return (
    <>
      <SectionHeading>Pages</SectionHeading>
      <div data-testid="rail-pages" style={{ paddingBottom: 12 }}>
        {entries.map((entry) => (
          <button
            key={entry.id}
            type="button"
            data-testid={`rail-page-${entry.id}`}
            title={entry.label}
            onClick={() => onOpenPage(entry.ref)}
            style={rowHoverStyle()}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {entry.label}
            </span>
            {entry.badge ? (
              <span
                style={{
                  fontSize: 10,
                  color: "var(--text-secondary)",
                  background: "var(--surface-tab-inactive)",
                  padding: "1px 6px",
                  borderRadius: 999,
                }}
              >
                {entry.badge}
              </span>
            ) : null}
          </button>
        ))}
      </div>
    </>
  );
}

