import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { BacklogState, FinishedEntry, ThreadWorkState, Task } from "../../api.js";
import { PageKindIcon } from "../../pageKinds.js";
import type { TabRef } from "../../tabs/tabState.js";
import { fileRef, wikiPageRef, opErrorRef, tasksRef, uncommittedChangesRef, commentsRef, taskRef, refFromTabId } from "../../tabs/pageRefs.js";
import { computePagesDirectory, RAIL_PAGE_IDS } from "./sections.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { computeActiveEpicContext, computeActiveItem, computeUpNext, sortRecentFiles, type RecentFileEntry } from "./sections.js";
import type { OpError } from "../opErrorsStore.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "./history.js";
import {
  listCommentsForStream,
  listRecentPageVisits,
  listWikiPages,
  subscribeCommentEvents,
  subscribePageVisitEvents,
  subscribeWikiPageEvents,
  topVisitedPages,
  type PageVisitApi,
  type TopVisitedRowApi,
} from "../../api.js";

export interface UncommittedSummary {
  added: number;
  modified: number;
  deleted: number;
  additions: number;
  deletions: number;
  conflictedCount?: number;
  gitOperation?: "merge" | "rebase" | "cherry-pick" | "revert" | null;
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
  /** Current stream — scopes the open-comments section. */
  streamId?: string | null;
  threadWork: ThreadWorkState | null;
  backlog: BacklogState | null;
  recentFiles: RecentFileEntry[];
  bookmarks?: BookmarkRailEntry[];
  /** Most recently finished work — closed tasks efforts merged
   *  with updated wiki notes, sorted by timestamp DESC. */
  recentlyFinished?: FinishedEntry[];
  /** Working-tree uncommitted summary; section hidden when null or empty. */
  uncommitted?: UncommittedSummary | null;
  /** Recent failed async operations. Section hidden when empty. */
  opErrors?: readonly OpError[];
  /** Dismiss a single op error from the in-memory store. */
  onDismissOpError?(id: string): void;
  /** Clear all recorded op errors. */
  onClearOpErrors?(): void;
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
 * - Ready
 * - Recent files
 * - Pages directory
 */
export function RailHud({
  threadId,
  streamId,
  threadWork,
  backlog,
  recentFiles,
  bookmarks,
  recentlyFinished,
  uncommitted,
  opErrors,
  onDismissOpError,
  onClearOpErrors,
  onClearFinished,
  onOpenPage,
  onOpenSearch,
}: RailHudProps) {
  const activeItem = useMemo(() => computeActiveItem(threadWork), [threadWork]);
  const activeEpic = useMemo(() => computeActiveEpicContext(threadWork, activeItem), [threadWork, activeItem]);
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
          epicContext={activeEpic}
          onOpenPage={onOpenPage}
        />
      ) : null}

      {uncommitted && (
        uncommitted.added + uncommitted.modified + uncommitted.deleted > 0
        || (uncommitted.conflictedCount ?? 0) > 0
        || uncommitted.gitOperation
      ) ? (
        <UncommittedSection summary={uncommitted} onOpenPage={onOpenPage} />
      ) : null}

      <CommentsSection streamId={streamId ?? null} onOpenPage={onOpenPage} />

      {opErrors && opErrors.length > 0 ? (
        <OpErrorsSection
          entries={opErrors}
          onOpenPage={onOpenPage}
          onDismiss={onDismissOpError}
          onClear={onClearOpErrors}
        />
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

      <HistorySection onOpenPage={onOpenPage} threadId={threadId} />

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
  fontSize: "var(--text-sm)",
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
          fontSize: "var(--text-sm)",
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

function statusIcon(status: Task["status"]): string {
  switch (status) {
    case "done": return "✓";
    case "in_progress": return "◐";
    case "blocked": return "⚠";
    case "canceled": return "✗";
    case "archived": return "▣";
    case "ready":
    default: return "☐";
  }
}

function statusIconColor(status: Task["status"]): string {
  switch (status) {
    case "done": return "var(--diff-add-fg, #2ea043)";
    case "in_progress": return "var(--accent-fg, #58a6ff)";
    case "blocked": return "var(--diff-del-fg, #f85149)";
    case "canceled": return "var(--text-muted)";
    default: return "var(--text-secondary)";
  }
}

function ActiveItemSection({
  item,
  epicContext,
  onOpenPage,
}: {
  item: Task | null;
  epicContext: { epic: Task; children: Task[] } | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [expanded, setExpanded] = useState(true);
  if (!item) {
    return null;
  }

  if (epicContext) {
    const { epic, children } = epicContext;
    return (
      <>
        <SectionHeading>Current Work</SectionHeading>
        <div
          data-testid="rail-active-epic"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 4,
            padding: "4px 8px 4px 14px",
          }}
        >
          <button
            type="button"
            data-testid="rail-active-epic-toggle"
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? "Collapse" : "Expand"}
            aria-expanded={expanded}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-secondary)",
              cursor: "pointer",
              padding: "0 2px",
              fontSize: 14,
              width: 16,
            }}
          >
            {expanded ? "▾" : "▸"}
          </button>
          <button
            type="button"
            data-testid="rail-active-epic-row"
            onClick={() => onOpenPage(taskRef(epic.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "task",
              itemId: epic.id,
              title: epic.title,
              status: epic.status,
            })}
            style={{
              ...rowStyle,
              padding: "2px 6px",
              flex: 1,
            }}
          >
            <span aria-hidden style={{ fontSize: 11, color: "var(--text-secondary)" }}>📚</span>
            <span
              style={{
                flex: 1,
                color: "var(--text-primary)",
                fontWeight: "var(--weight-medium)",
                fontSize: "var(--text-sm)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {epic.title}
            </span>
          </button>
        </div>
        {expanded ? (
          <div
            data-testid="rail-active-epic-children"
            style={{ paddingBottom: 8 }}
          >
            {children.map((child) => {
              const isActive = child.id === item.id;
              return (
                <button
                  key={child.id}
                  type="button"
                  data-testid={`rail-active-epic-child-${child.id}`}
                  onClick={() => onOpenPage(taskRef(child.id))}
                  draggable
                  onDragStart={(ev) => setContextRefDrag(ev, {
                    kind: "task",
                    itemId: child.id,
                    title: child.title,
                    status: child.status,
                  })}
                  style={{
                    ...rowStyle,
                    padding: "4px 14px 4px 32px",
                    background: isActive ? "var(--surface-card)" : "transparent",
                    fontWeight: isActive ? 500 : 400,
                  }}
                  title={child.title}
                >
                  <span
                    aria-hidden
                    style={{
                      width: 14,
                      display: "inline-flex",
                      justifyContent: "center",
                      color: statusIconColor(child.status),
                      fontSize: "var(--text-xs)",
                    }}
                  >
                    {statusIcon(child.status)}
                  </span>
                  <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {child.title}
                  </span>
                </button>
              );
            })}
          </div>
        ) : null}
      </>
    );
  }

  return (
    <>
      <SectionHeading>Current Work</SectionHeading>
      <button
        type="button"
        data-testid="rail-active-item"
        onClick={() => onOpenPage(taskRef(item.id))}
        draggable
        onDragStart={(ev) => setContextRefDrag(ev, {
          kind: "task",
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
            fontWeight: "var(--weight-medium)",
            fontSize: "var(--text-sm)",
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
  const conflictedCount = summary.conflictedCount ?? 0;
  const op = summary.gitOperation ?? null;
  const hasFileSummary = parts.length > 0;
  const hasConflictRow = conflictedCount > 0 || op !== null;
  return (
    <>
      <SectionHeading>Uncommitted</SectionHeading>
      {hasFileSummary ? (
        <button
          type="button"
          data-testid="rail-uncommitted"
          onClick={() => onOpenPage(uncommittedChangesRef())}
          title="Open uncommitted changes"
          style={{
            ...rowStyle,
            padding: hasConflictRow ? "4px 14px 4px" : "4px 14px 12px",
            gap: 8,
          }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>
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
      ) : null}
      {hasConflictRow ? (
        <button
          type="button"
          data-testid="rail-uncommitted-conflicts"
          onClick={() => onOpenPage(uncommittedChangesRef())}
          title={
            op
              ? `${op} in progress${conflictedCount > 0 ? ` — ${conflictedCount} conflicted file${conflictedCount === 1 ? "" : "s"}` : ""}`
              : `${conflictedCount} conflicted file${conflictedCount === 1 ? "" : "s"}`
          }
          style={{
            ...rowStyle,
            padding: "4px 14px 12px",
            gap: 8,
          }}
        >
          <span style={{ color: "var(--diff-del-fg, #f85149)", fontSize: "var(--text-xs)" }}>
            {op ? `${op} in progress` : `${conflictedCount} conflict${conflictedCount === 1 ? "" : "s"}`}
          </span>
          {op && conflictedCount > 0 ? (
            <>
              <span style={{ flex: 1 }} />
              <span style={{ color: "var(--diff-del-fg, #f85149)", fontSize: 11 }}>
                {conflictedCount} conflict{conflictedCount === 1 ? "" : "s"}
              </span>
            </>
          ) : null}
        </button>
      ) : null}
    </>
  );
}

/// Open-comments summary: counts of unresolved comments in the current
/// stream, split by intent — "for me" (notes-to-self) and "for the
/// agent" (follow-ups). Self-fetching + live like HistorySection;
/// hidden when there are none. Each row opens the Comments inbox.
function CommentsSection({
  streamId,
  onOpenPage,
}: {
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [notes, setNotes] = useState(0);
  const [followups, setFollowups] = useState(0);

  useEffect(() => {
    if (!streamId) {
      setNotes(0);
      setFollowups(0);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listCommentsForStream(streamId).then((threads) => {
        if (cancelled) return;
        let n = 0;
        let f = 0;
        for (const t of threads) {
          if (t.comment.status !== "open") continue;
          if (t.comment.intent === "followup") f += 1;
          else n += 1;
        }
        setNotes(n);
        setFollowups(f);
      });
    };
    refresh();
    const off = subscribeCommentEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [streamId]);

  if (notes === 0 && followups === 0) return null;

  return (
    <>
      <SectionHeading>Comments</SectionHeading>
      {notes > 0 ? (
        <button
          type="button"
          data-testid="rail-comments-notes"
          onClick={() => onOpenPage(commentsRef())}
          title="Open the Comments inbox"
          style={{ ...rowStyle, padding: "4px 14px 4px", gap: 8 }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>
            For me
          </span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{notes}</span>
        </button>
      ) : null}
      {followups > 0 ? (
        <button
          type="button"
          data-testid="rail-comments-followups"
          onClick={() => onOpenPage(commentsRef())}
          title="Open the Comments inbox"
          style={{ ...rowStyle, padding: "4px 14px 12px", gap: 8 }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>
            For the agent
          </span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--accent)", fontSize: 11 }}>{followups}</span>
        </button>
      ) : null}
    </>
  );
}

function OpErrorsSection({
  entries,
  onOpenPage,
  onDismiss,
  onClear,
}: {
  entries: readonly OpError[];
  onOpenPage(ref: TabRef): void;
  onDismiss?(id: string): void;
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
        <span
          style={{
            flex: 1,
            fontSize: 11,
            fontWeight: 600,
            color: "var(--diff-del-fg, #f85149)",
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          Errors
        </span>
        {onClear ? (
          <button
            type="button"
            data-testid="rail-op-errors-clear"
            onClick={(e) => { e.stopPropagation(); onClear(); }}
            title="Clear all errors"
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
      <div data-testid="rail-op-errors" style={{ paddingBottom: 8 }}>
        {entries.map((entry) => (
          <div
            key={entry.id}
            style={{ display: "flex", alignItems: "center", gap: 4, paddingRight: 6 }}
          >
            <button
              type="button"
              data-testid={`rail-op-error-${entry.id}`}
              title={entry.stderr || entry.message || entry.label}
              onClick={() => onOpenPage(opErrorRef(entry.id))}
              style={{ ...rowHoverStyle(), flex: 1 }}
            >
              <span
                aria-hidden
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: entry.seen ? "transparent" : "var(--diff-del-fg, #f85149)",
                  border: entry.seen ? "1px solid var(--diff-del-fg, #f85149)" : "none",
                  flexShrink: 0,
                }}
              />
              <span
                style={{
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: entry.seen ? "var(--text-secondary)" : "var(--text-primary)",
                }}
              >
                {entry.label}
              </span>
            </button>
            {onDismiss ? (
              <button
                type="button"
                data-testid={`rail-op-error-dismiss-${entry.id}`}
                title="Dismiss"
                onClick={(e) => { e.stopPropagation(); onDismiss(entry.id); }}
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
            ) : null}
          </div>
        ))}
      </div>
    </>
  );
}

function UpNextSection({
  items,
  onOpenPage,
}: {
  items: Task[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Ready</SectionHeading>
      <div data-testid="rail-up-next" style={{ paddingBottom: 8 }}>
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            data-testid={`rail-up-next-item-${item.id}`}
            title={item.title}
            onClick={() => onOpenPage(taskRef(item.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "task",
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
              <PageKindIcon kind={entry.ref.kind} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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
          onClick={() => onOpenPage(tasksRef())}
          title="Open Tasks"
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
          const ref = e.kind === "task" ? taskRef(e.itemId) : wikiPageRef(e.slug);
          return (
            <button
              key={`${e.kind}:${e.kind === "task" ? e.itemId : e.slug}`}
              type="button"
              data-testid={`rail-finished-${e.kind === "task" ? e.itemId : e.slug}`}
              title={e.title}
              onClick={() => onOpenPage(ref)}
              style={rowHoverStyle()}
            >
              <PageKindIcon kind={e.kind === "task" ? "task" : "wiki"} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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

function HistorySection({
  onOpenPage,
  threadId,
}: {
  onOpenPage(ref: TabRef): void;
  threadId: string | null;
}) {
  const [expanded, setExpanded] = useState(false);
  const [mode, setMode] = useState<"recent" | "top">("recent");
  const [recent, setRecent] = useState<PageVisitApi[]>([]);
  const [top, setTop] = useState<TopVisitedRowApi[]>([]);
  // Wiki visit rows carry the title that was current when the page
  // was activated. That snapshot can be stale ("" for pages activated
  // before their summary loaded; outdated when titles change later).
  // Resolve fresh slug → title here and prefer it over `e.label`
  // whenever the entry is a wiki page.
  const [wikiTitles, setWikiTitles] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listRecentPageVisits({
        threadId,
        limit: 10,
        dedupeByRef: true,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      }).then((rows) => {
        if (!cancelled) setRecent(rows);
      });
      const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
      void topVisitedPages({
        threadId,
        sinceT: since,
        limit: 10,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      }).then((rows) => {
        if (!cancelled) setTop(rows);
      });
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [threadId]);

  // Maintain the slug → title map, refreshed on wiki-page events
  // (creation, title rename, deletion) so a renamed page updates in
  // the history list without waiting for the next visit.
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listWikiPages("").then((pages) => {
        if (cancelled) return;
        const map: Record<string, string> = {};
        for (const p of pages) map[p.slug] = p.title;
        setWikiTitles(map);
      });
    };
    refresh();
    const off = subscribeWikiPageEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  // Hide the entire section when there is nothing to show in either
  // mode — no header, no toggle, no flicker. If the user has data in
  // only one of the two modes, fall back to that one so the section
  // doesn't appear "broken" when toggled.
  if (recent.length === 0 && top.length === 0) return null;
  const effectiveMode = mode === "recent" && recent.length === 0 && top.length > 0
    ? "top"
    : mode === "top" && top.length === 0 && recent.length > 0
    ? "recent"
    : mode;
  const source = effectiveMode === "recent" ? recent : top;
  const limit = expanded ? 10 : 5;
  const entries = source.slice(0, limit);
  const hasMore = source.length > 5;

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "12px 14px 4px",
        }}
      >
        <span
          style={{
            flex: 1,
            fontSize: 11,
            fontWeight: 600,
            color: "var(--text-secondary)",
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          {effectiveMode === "recent" ? "History" : "Most visited"}
        </span>
        <button
          type="button"
          data-testid="rail-history-mode"
          onClick={() => setMode((m) => (m === "recent" ? "top" : "recent"))}
          title={effectiveMode === "recent" ? "Show most visited (last 30d)" : "Show recent"}
          style={{
            background: "transparent",
            border: "none",
            color: "var(--text-secondary)",
            cursor: "pointer",
            fontSize: 10,
            padding: "0 4px",
          }}
        >
          {effectiveMode === "recent" ? "top" : "recent"}
        </button>
      </div>
      <div data-testid="rail-history" style={{ paddingBottom: 4 }}>
        {entries.map((e) => {
          // Reconstruct the full ref (with payload) from the id —
          // page-visit rows don't persist payload, so a file ref needs
          // its `path` rebuilt or it won't open. See refFromTabId.
          const ref: TabRef = refFromTabId(e.refId);
          const trailing = effectiveMode === "top" ? (e as TopVisitedRowApi).count : null;
          // Wiki: prefer the live title over the stored visit label.
          // Falls back to a non-empty stored label, then to the slug,
          // so the row always renders something.
          const liveWikiTitle =
            ref.kind === "wiki" ? wikiTitles[e.refId]?.trim() : null;
          const display =
            liveWikiTitle || (e.label?.trim() ?? "") || e.refId;
          return (
            <button
              key={e.refId}
              type="button"
              data-testid={`rail-history-${e.refId}`}
              title={display}
              onClick={() => onOpenPage(ref)}
              style={rowHoverStyle()}
            >
              <PageKindIcon kind={ref.kind} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {display}
              </span>
              {trailing != null ? (
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-secondary)",
                    background: "var(--surface-tab-inactive)",
                    padding: "1px 6px",
                    borderRadius: 999,
                  }}
                >
                  {trailing}
                </span>
              ) : null}
            </button>
          );
        })}
        {hasMore ? (
          <button
            type="button"
            data-testid="rail-history-toggle"
            onClick={() => setExpanded((v) => !v)}
            style={{
              ...rowHoverStyle(),
              color: "var(--text-secondary)",
              fontSize: 11,
              padding: "4px 14px 8px",
            }}
          >
            {expanded ? "show less" : `show more (${Math.min(10, source.length)})`}
          </button>
        ) : null}
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
            <PageKindIcon kind={entry.ref.kind} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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

