import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentStatus,
  Thread,
  ThreadWorkState,
} from "../api.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { Kebab } from "./Kebab.js";
import type { MenuItem } from "../menu.js";

interface Props {
  threads: Thread[];
  activeThreadId: string | null;
  selectedThreadId: string | null;
  agentStatuses: Record<string, AgentStatus>;
  threadWorkStates: Record<string, ThreadWorkState>;
  onSelectThread(threadId: string): void | Promise<void>;
  onCreateThread(title: string): Promise<void>;
  onPromoteThread(threadId: string): void | Promise<void>;
  onCloseThread(threadId: string): void | Promise<void>;
  onOpenClosedThreads?(): void;
  onMoveWorkItem?(itemId: string, fromThreadId: string, toThreadId: string): Promise<void>;
  onMoveBacklogItemToThread?(itemId: string, toThreadId: string): Promise<void>;
  onRenameThread?(threadId: string, newTitle: string): Promise<void> | void;
  onReorderThreads?(orderedThreadIds: string[]): Promise<void> | void;
  onRequestCreateStream?(): void;
  onRequestCreateThread?(): void;
  /** Open the per-thread settings page. The kebab "Settings" item
   *  routes here. */
  onOpenThreadSettings?(threadId: string): void;
  /** Bumping this number opens the "new thread" modal. */
  createRequest?: number;
}

export const WORK_ITEM_DRAG_MIME = "application/x-oxplow-work-item";
export const THREAD_DRAG_MIME = "application/x-oxplow-thread";

export function ThreadRail({
  threads,
  activeThreadId,
  selectedThreadId,
  agentStatuses,
  threadWorkStates,
  onSelectThread,
  onCreateThread,
  onPromoteThread,
  onCloseThread,
  onOpenClosedThreads,
  onMoveWorkItem,
  onMoveBacklogItemToThread,
  onRenameThread,
  onReorderThreads,
  onRequestCreateStream,
  onRequestCreateThread,
  onOpenThreadSettings,
  createRequest,
}: Props) {
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overThreadId, setOverThreadId] = useState<string | null>(null);

  const ordered = useMemo(() => {
    // Closed threads are filtered out of the rail by the store; what
    // arrives here is the active + queued set, ordered by sort_index.
    return threads.slice().sort((a, b) => a.sort_index - b.sort_index);
  }, [threads]);

  const hasQueued = threads.some((b) => b.status === "queued");
  const [showCreate, setShowCreate] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    setShowCreate(true);
  }, [createRequest]);

  function buildThreadMenu(thread: Thread): MenuItem[] {
    const isWriter = thread.id === activeThreadId;
    const work = threadWorkStates[thread.id];
    const openItemCount = work
      ? work.items.filter(
          (i) => i.status === "ready" || i.status === "blocked" || i.status === "in_progress",
        ).length
      : 0;
    const closeReason = isWriter
      ? "Promote another thread to writer first"
      : openItemCount > 0
      ? `Finish or move ${openItemCount} open work item${openItemCount === 1 ? "" : "s"} first`
      : null;
    return [
      {
        id: "thread.promote",
        label: "Promote to writer",
        enabled: !isWriter,
        run: () => onPromoteThread(thread.id),
      },
      {
        id: "thread.close",
        label: closeReason ? `Close thread (${closeReason})` : "Close thread",
        enabled: closeReason === null,
        run: () => onCloseThread(thread.id),
      },
      {
        id: "thread.rename",
        label: "Rename…",
        enabled: !!onRenameThread,
        run: () => setRenamingId(thread.id),
      },
      {
        id: "thread.settings",
        label: "Settings",
        enabled: !!onOpenThreadSettings,
        run: () => onOpenThreadSettings?.(thread.id),
      },
      {
        id: "thread.add-thread",
        label: "Add thread",
        enabled: !!onRequestCreateThread,
        run: () => (onRequestCreateThread ? onRequestCreateThread() : setShowCreate(true)),
      },
      {
        id: "thread.add-stream",
        label: "Add stream",
        enabled: !!onRequestCreateStream,
        run: () => onRequestCreateStream?.(),
      },
    ];
  }

  return (
    <div style={railStyle}>
      <div className="oxplow-rail-scroll" style={{ display: "flex", alignItems: "flex-end", gap: 2, flex: 1, minWidth: 0, overflowX: "auto" }}>
        {ordered.length === 0 ? (
          <span style={{ color: "var(--muted)", fontSize: 11, padding: "8px 12px", alignSelf: "center" }}>No threads yet.</span>
        ) : null}
        {ordered.map((thread) => (
          <ThreadChip
            key={thread.id}
            thread={thread}
            isActive={thread.id === activeThreadId}
            isSelected={thread.id === selectedThreadId}
            agentStatus={agentStatuses[thread.id] ?? "waiting"}
            workState={threadWorkStates[thread.id]}
            hasQueued={hasQueued}
            isRenaming={renamingId === thread.id}
            isDragTarget={overThreadId === thread.id && draggingId !== null && draggingId !== thread.id}
            onSelect={() => void onSelectThread(thread.id)}
            onPromote={() => void onPromoteThread(thread.id)}
            onClose={() => void onCloseThread(thread.id)}
            onCancelRename={() => setRenamingId(null)}
            onSubmitRename={async (newTitle) => {
              const trimmed = newTitle.trim();
              setRenamingId(null);
              if (!trimmed || trimmed === thread.title) return;
              await onRenameThread?.(thread.id, trimmed);
            }}
            menuItems={buildThreadMenu(thread)}
            onDropWorkItem={(onMoveWorkItem || onMoveBacklogItemToThread) ? (payload) => {
              if (payload.fromThreadId === null) {
                if (onMoveBacklogItemToThread) void onMoveBacklogItemToThread(payload.itemId, thread.id);
              } else {
                if (onMoveWorkItem) void onMoveWorkItem(payload.itemId, payload.fromThreadId, thread.id);
              }
            } : undefined}
            onDragStart={onReorderThreads ? () => setDraggingId(thread.id) : undefined}
            onDragEnd={onReorderThreads ? () => { setDraggingId(null); setOverThreadId(null); } : undefined}
            onDragOver={onReorderThreads ? () => setOverThreadId(thread.id) : undefined}
            onDrop={onReorderThreads ? () => {
              if (!draggingId || draggingId === thread.id) return;
              const ids = ordered.map((b) => b.id);
              const fromIdx = ids.indexOf(draggingId);
              const toIdx = ids.indexOf(thread.id);
              if (fromIdx < 0 || toIdx < 0) return;
              const next = ids.slice();
              const [moved] = next.splice(fromIdx, 1);
              next.splice(toIdx, 0, moved);
              void onReorderThreads(next);
              setDraggingId(null);
              setOverThreadId(null);
            } : undefined}
          />
        ))}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, flexShrink: 0, paddingLeft: 8 }}>
        {showCreate ? (
          <InlineCreateThreadRow
            nextIndex={threads.length + 1}
            onCancel={() => setShowCreate(false)}
            onSubmit={async (title) => {
              await onCreateThread(title);
              setShowCreate(false);
            }}
          />
        ) : (
          <button type="button" data-testid="thread-rail-new" style={smallBtn} onClick={() => setShowCreate(true)} title="Create thread">
            + New thread
          </button>
        )}
        {onOpenClosedThreads ? (
          <button
            type="button"
            data-testid="thread-rail-closed-threads"
            style={smallBtn}
            onClick={() => onOpenClosedThreads()}
            title="View threads you have closed"
          >
            Closed threads
          </button>
        ) : null}
      </div>
    </div>
  );
}

function ThreadChip({
  thread,
  isActive,
  isSelected,
  agentStatus,
  workState,
  hasQueued,
  onSelect,
  onPromote,
  onClose,
  menuItems,
  onDropWorkItem,
  isRenaming,
  onSubmitRename,
  onCancelRename,
  isDragTarget,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
}: {
  thread: Thread;
  isActive: boolean;
  isSelected: boolean;
  agentStatus: AgentStatus;
  workState: ThreadWorkState | undefined;
  hasQueued: boolean;
  onSelect(): void;
  onPromote(): void;
  onClose(): void;
  menuItems?: MenuItem[];
  onDropWorkItem?(payload: { itemId: string; fromThreadId: string | null }): void;
  isRenaming?: boolean;
  onSubmitRename?(newTitle: string): void | Promise<void>;
  onCancelRename?(): void;
  isDragTarget?: boolean;
  onDragStart?(): void;
  onDragEnd?(): void;
  onDragOver?(): void;
  onDrop?(): void;
}) {
  const [hovered, setHovered] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tabRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  function scheduleShow() {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setHovered(true), 250);
  }
  function cancelShow() {
    if (timerRef.current) clearTimeout(timerRef.current);
    setHovered(false);
  }

  // Count items as "done-ish" when status is done, canceled, or archived.
  const total = workState?.items.length ?? 0;
  const done = workState
    ? workState.items.filter((i) => i.status === "done" || i.status === "canceled" || i.status === "archived").length
    : 0;
  // Two orthogonal states on each tab:
  //  - `isSelected`: what the user is viewing (main bg, accent underline)
  //  - `isActive`:   the designated *writer* thread (the one allowed to write
  //                  to disk). All queued threads can have agents running;
  //                  only the writer can commit changes.
  // The writer is visualised with a pencil badge so it's obvious even when
  // another thread is selected.
  const background = isSelected ? "var(--bg)" : "var(--bg-tab-inactive)";
  const color = isSelected ? "var(--fg)" : "var(--muted)";

  const handleDragOver = (event: React.DragEvent) => {
    const types = Array.from(event.dataTransfer.types);
    if (types.includes(THREAD_DRAG_MIME)) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      onDragOver?.();
      return;
    }
    if (!onDropWorkItem) return;
    if (!types.includes(WORK_ITEM_DRAG_MIME)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    if (!dragOver) setDragOver(true);
  };
  const handleDragLeave = () => {
    if (dragOver) setDragOver(false);
  };
  const handleDrop = (event: React.DragEvent) => {
    if (event.dataTransfer.types && Array.from(event.dataTransfer.types).includes(THREAD_DRAG_MIME)) {
      event.preventDefault();
      onDrop?.();
      return;
    }
    if (!onDropWorkItem) return;
    const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
    if (!raw) return;
    event.preventDefault();
    setDragOver(false);
    try {
      const payload = JSON.parse(raw) as {
        itemId?: string;
        itemIds?: string[];
        fromThreadId?: string | null;
      };
      const fromThreadId = payload.fromThreadId ?? null;
      if (fromThreadId === thread.id) return;
      // `itemIds` carries the full mark set when the drag originated on a
      // marked row; fall back to the single `itemId` for regular drags so
      // older payloads still work.
      const ids = payload.itemIds && payload.itemIds.length > 0
        ? payload.itemIds
        : payload.itemId ? [payload.itemId] : [];
      for (const id of ids) {
        onDropWorkItem({ itemId: id, fromThreadId });
      }
    } catch {
      // ignore malformed payload
    }
  };

  return (
    <div
      data-testid={`thread-chip-${thread.id}`}
      style={{ position: "relative", flexShrink: 0, borderLeft: isDragTarget ? "2px solid var(--accent)" : undefined }}
      onMouseEnter={scheduleShow}
      onMouseLeave={cancelShow}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div
        ref={tabRef}
        role="button"
        tabIndex={0}
        draggable={!!onDragStart}
        onDragStart={onDragStart ? (event) => {
          event.dataTransfer.setData(THREAD_DRAG_MIME, JSON.stringify({ threadId: thread.id }));
          event.dataTransfer.effectAllowed = "move";
          // Force the drag image to just the tab element — otherwise the
          // browser captures the wrapper (including the absolutely-positioned
          // hover card region), producing a "detached square" artifact.
          if (tabRef.current) {
            const rect = tabRef.current.getBoundingClientRect();
            event.dataTransfer.setDragImage(
              tabRef.current,
              event.clientX - rect.left,
              event.clientY - rect.top,
            );
          }
          setIsDragging(true);
          onDragStart();
        } : undefined}
        onDragEnd={onDragEnd ? () => { setIsDragging(false); onDragEnd(); } : undefined}
        onClick={isRenaming ? undefined : onSelect}
        onKeyDown={isRenaming ? undefined : (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onSelect(); } }}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 14px 6px 14px",
          // Non-selected tabs keep a visible 1px frame so they still read as
          // tabs (vs. floating text). Selected tab drops the frame to blend
          // with the content below and carries the 2px accent underline.
          borderTop: isSelected ? "1px solid transparent" : "1px solid var(--border-strong)",
          borderLeft: isSelected ? "1px solid transparent" : "1px solid var(--border-strong)",
          borderRight: isSelected ? "1px solid transparent" : "1px solid var(--border-strong)",
          borderBottom: isSelected ? "2px solid var(--accent)" : "2px solid transparent",
          borderTopLeftRadius: 6,
          borderTopRightRadius: 6,
          background,
          color,
          cursor: isRenaming ? "text" : "pointer",
          fontFamily: "inherit",
          fontSize: 12,
          whiteSpace: "nowrap",
          flexShrink: 0,
          marginBottom: -1, // overlap the rail's bottom border so the tab looks connected to the content below when selected
          boxShadow: dragOver ? "inset 0 0 0 2px var(--accent)" : undefined,
        }}
        title={isActive ? `${thread.title} · writer (can commit)` : `${thread.title} (read-only)`}
      >
        <AgentStatusDot status={agentStatus} />
        {isActive ? (
          <span
            aria-label="Writer thread"
            title="Writer thread — only this one can commit changes"
            style={{
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              width: 16,
              height: 16,
              borderRadius: 999,
              background: "var(--accent)",
              color: "#fff",
              fontSize: 10,
              lineHeight: 1,
            }}
          >✎</span>
        ) : null}
        {isRenaming ? (
          <input
            autoFocus
            defaultValue={thread.title}
            data-testid={`thread-chip-rename-input-${thread.id}`}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") {
                void onSubmitRename?.((e.target as HTMLInputElement).value);
              } else if (e.key === "Escape") {
                onCancelRename?.();
              }
            }}
            onBlur={(e) => void onSubmitRename?.(e.target.value)}
            style={{
              background: "var(--bg)",
              color: "var(--fg)",
              border: "1px solid var(--border)",
              borderRadius: 4,
              padding: "2px 6px",
              fontFamily: "inherit",
              fontSize: 12,
              minWidth: 120,
            }}
          />
        ) : (
          <span style={{ fontWeight: isSelected ? 600 : isActive ? 500 : 400 }}>{thread.title}</span>
        )}
        {total > 0 ? (
          <span style={{ fontSize: 10, opacity: 0.75 }}>
            {done}/{total}
          </span>
        ) : null}
        {menuItems && menuItems.length > 0 ? (
          <span onClick={(e) => e.stopPropagation()}>
            <Kebab items={menuItems} testId={`thread-chip-kebab-${thread.id}`} size={14} />
          </span>
        ) : null}
      </div>
      {hovered && !isDragging ? (
        <HoverCard
          thread={thread}
          isActive={isActive}
          agentStatus={agentStatus}
          workState={workState}
          hasQueued={hasQueued}
          onPromote={onPromote}
          onClose={onClose}
        />
      ) : null}
    </div>
  );
}

function HoverCard({
  thread,
  isActive,
  agentStatus,
  workState,
  onPromote,
  onClose,
}: {
  thread: Thread;
  isActive: boolean;
  agentStatus: AgentStatus;
  workState: ThreadWorkState | undefined;
  hasQueued: boolean;
  onPromote(): void;
  onClose(): void;
}) {
  const total = workState?.items.length ?? 0;
  const waiting = workState?.waiting.length ?? 0;
  const inProgress = workState?.inProgress ?? [];
  const done = workState?.done.length ?? 0;
  const openCount = workState
    ? workState.items.filter(
        (i) => i.status === "ready" || i.status === "blocked" || i.status === "in_progress",
      ).length
    : 0;
  const canClose = !isActive && openCount === 0;
  const statusLabel = isActive ? "writer" : "read-only";
  const statusColor = isActive ? "#86efac" : "#7dd3fc";

  return (
    <div style={hoverCardStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <strong style={{ fontSize: 13 }}>{thread.title}</strong>
        <span
          style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 0.6,
            border: `1px solid ${statusColor}`,
            color: statusColor,
            borderRadius: 999,
            padding: "1px 6px",
          }}
        >
          {statusLabel}
        </span>
      </div>
      <div style={{ color: "var(--muted)", fontSize: 11 }}>agent: {agentStatus}</div>
      {inProgress.length > 0 ? (
        <div style={{ fontSize: 12, lineHeight: 1.4 }}>
          <div style={metaLabel}>In progress</div>
          <div style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
            {inProgress[0].title}
          </div>
        </div>
      ) : null}
      <div style={{ display: "flex", gap: 10, fontSize: 11, color: "var(--muted)", flexWrap: "wrap" }}>
        <span>
          {done}/{total} done
        </span>
        <span>{waiting} waiting</span>
      </div>
      <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
        {isActive ? null : (
          <>
            <button type="button" data-testid={`thread-chip-promote-${thread.id}`} style={smallBtn} onClick={onPromote} title="Make this thread the writer — only one thread can write at a time">
              Make writer
            </button>
            <button
              type="button"
              data-testid={`thread-chip-close-${thread.id}`}
              style={smallBtn}
              onClick={onClose}
              disabled={!canClose}
              title={canClose
                ? "Close this thread; you can reopen it later from the Closed threads page"
                : openCount > 0
                ? `Close blocked: ${openCount} open work item${openCount === 1 ? "" : "s"}`
                : "Cannot close the writer thread"}
            >
              Close thread
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Inline-new-row that retires `CreateThreadModal`. Mounts at the end of
 * the thread chip strip when the user clicks "+ New thread"; Enter
 * submits, Escape cancels. Existing `data-testid="thread-rail-create-input"`
 * and `thread-rail-create-submit` testids stay so e2e probes (and the
 * Cmd+K palette workflow) keep working.
 */
/**
 * `nextThreadTitle` — pure helper for picking the default placeholder
 * title for the inline-create row. When "another after submit" is on
 * the form re-mounts in place, so we bump the index forward to avoid
 * shipping two identical "Thread N" titles in a row.
 *
 * Exported for unit tests so the carry-forward logic is verified
 * without mounting the renderer.
 */
export function nextThreadTitle(currentIndex: number): string {
  return `Thread ${currentIndex}`;
}

function InlineCreateThreadRow({
  nextIndex,
  onSubmit,
  onCancel,
}: {
  nextIndex: number;
  onSubmit(title: string): Promise<void>;
  onCancel(): void;
}) {
  const [title, setTitle] = useState(nextThreadTitle(nextIndex));
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.select();
  }, []);

  async function submit() {
    const trimmed = title.trim();
    if (!trimmed) {
      setError("Title is required");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(trimmed);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  }

  const trimmed = title.trim();
  const canSubmit = !submitting && trimmed.length > 0;

  return (
    <form
      style={{ display: "flex", alignItems: "center", gap: 6 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (!canSubmit) return;
        void submit();
      }}
    >
      <input
        ref={inputRef}
        value={title}
        autoFocus
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        style={{ ...settingsInputStyle, width: 200, padding: "3px 6px" }}
        placeholder="Thread title"
        data-testid="thread-rail-create-input"
        aria-label="New thread title"
      />
      <button
        type="submit"
        data-testid="thread-rail-create-submit"
        style={smallBtn}
        disabled={!canSubmit}
      >
        {submitting ? "Creating…" : "Create"}
      </button>
      <button
        type="button"
        onClick={onCancel}
        style={smallBtn}
      >
        Cancel
      </button>
      {error ? <span style={{ color: "#ff6b6b", fontSize: 11 }}>{error}</span> : null}
    </form>
  );
}

const railStyle: CSSProperties = {
  display: "flex",
  alignItems: "stretch",
  gap: 0,
  paddingLeft: 0,
  paddingRight: 8,
  paddingTop: 6,
  background: "var(--bg-2)",
  borderBottom: "1px solid var(--border)",
  flexWrap: "wrap",
  minHeight: 32,
};

const smallBtn: CSSProperties = {
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  borderRadius: 6,
  padding: "3px 8px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
};

const hoverCardStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 6px)",
  left: 0,
  zIndex: 20,
  minWidth: 260,
  maxWidth: 360,
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: 10,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  boxShadow: "0 6px 24px rgba(0,0,0,0.4)",
  pointerEvents: "auto",
};

const metaLabel: CSSProperties = {
  color: "var(--muted)",
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.6,
};

const settingsInputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "6px 8px",
  fontSize: 12,
  width: "100%",
  boxSizing: "border-box",
};
