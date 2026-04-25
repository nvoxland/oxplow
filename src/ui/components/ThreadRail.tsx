import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentStatus,
  Thread,
  ThreadWorkState,
} from "../api.js";
import { setThreadPrompt } from "../api.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { ContextMenu } from "./ContextMenu.js";
import type { MenuItem } from "../menu.js";
import { logUi } from "../logger.js";

interface Props {
  streamId: string;
  threads: Thread[];
  activeThreadId: string | null;
  selectedThreadId: string | null;
  agentStatuses: Record<string, AgentStatus>;
  threadWorkStates: Record<string, ThreadWorkState>;
  onSelectThread(threadId: string): void | Promise<void>;
  onCreateThread(title: string): Promise<void>;
  onPromoteThread(threadId: string): void | Promise<void>;
  onCompleteThread(threadId: string): void | Promise<void>;
  onMoveWorkItem?(itemId: string, fromThreadId: string, toThreadId: string): Promise<void>;
  onMoveBacklogItemToThread?(itemId: string, toThreadId: string): Promise<void>;
  onRenameThread?(threadId: string, newTitle: string): Promise<void> | void;
  onReorderThreads?(orderedThreadIds: string[]): Promise<void> | void;
  onRequestCreateStream?(): void;
  onRequestCreateThread?(): void;
  /** Bumping this number opens the "new thread" modal. */
  createRequest?: number;
}

export const WORK_ITEM_DRAG_MIME = "application/x-oxplow-work-item";
export const THREAD_DRAG_MIME = "application/x-oxplow-thread";

export function ThreadRail({
  streamId,
  threads,
  activeThreadId,
  selectedThreadId,
  agentStatuses,
  threadWorkStates,
  onSelectThread,
  onCreateThread,
  onPromoteThread,
  onCompleteThread,
  onMoveWorkItem,
  onMoveBacklogItemToThread,
  onRenameThread,
  onReorderThreads,
  onRequestCreateStream,
  onRequestCreateThread,
  createRequest,
}: Props) {
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overThreadId, setOverThreadId] = useState<string | null>(null);
  const [settingsThread, setSettingsThread] = useState<Thread | null>(null);
  const [settingsPrompt, setSettingsPrompt] = useState("");
  const [settingsSaving, setSettingsSaving] = useState(false);

  function openThreadSettings(thread: Thread) {
    setSettingsThread(thread);
    setSettingsPrompt(thread.custom_prompt ?? "");
    setSettingsSaving(false);
  }

  async function saveThreadSettings() {
    if (!settingsThread) return;
    setSettingsSaving(true);
    try {
      await setThreadPrompt(streamId, settingsThread.id, settingsPrompt.trim() || null);
      setSettingsThread(null);
    } catch (e) {
      logUi("error", "failed to save thread prompt", { threadId: settingsThread.id, error: String(e) });
    } finally {
      setSettingsSaving(false);
    }
  }

  const { ordered, completed } = useMemo(() => {
    // All non-completed threads share a single sort_index sequence and are
    // user-orderable via drag. The writer thread gets a visible badge (see
    // ThreadChip) but its position is whatever the user chose.
    const ordered = threads
      .filter((b) => b.status !== "completed")
      .sort((a, b) => a.sort_index - b.sort_index);
    const completed = threads
      .filter((b) => b.status === "completed")
      .sort((a, b) => b.sort_index - a.sort_index);
    return { ordered, completed };
  }, [threads]);

  const hasQueued = threads.some((b) => b.status === "queued");
  const [showOverflow, setShowOverflow] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; thread: Thread } | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    setShowCreate(true);
  }, [createRequest]);

  const contextMenuItems: MenuItem[] = contextMenu
    ? [
        {
          id: "thread.promote",
          label: "Promote to writer",
          enabled: contextMenu.thread.id !== activeThreadId,
          run: () => onPromoteThread(contextMenu.thread.id),
        },
        {
          id: "thread.complete",
          label: "Mark complete",
          enabled: hasQueued,
          run: () => onCompleteThread(contextMenu.thread.id),
        },
        {
          id: "thread.rename",
          label: "Rename…",
          enabled: !!onRenameThread,
          run: () => setRenamingId(contextMenu.thread.id),
        },
        {
          id: "thread.settings",
          label: "Settings",
          enabled: true,
          run: () => openThreadSettings(contextMenu.thread),
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
      ]
    : [];

  return (
    <div style={railStyle}>
      <div className="oxplow-rail-scroll" style={{ display: "flex", alignItems: "flex-end", gap: 2, flex: 1, minWidth: 0, overflowX: "auto" }}>
        {ordered.length === 0 && completed.length === 0 ? (
          <span style={{ color: "var(--muted)", fontSize: 11, padding: "8px 12px", alignSelf: "center" }}>No threads yet.</span>
        ) : null}
        {ordered.map((thread) => (
          <ThreadChip
            key={thread.id}
            thread={thread}
            isActive={thread.id === activeThreadId}
            isSelected={thread.id === selectedThreadId}
            agentStatus={agentStatuses[thread.id] ?? "idle"}
            workState={threadWorkStates[thread.id]}
            hasQueued={hasQueued}
            isRenaming={renamingId === thread.id}
            isDragTarget={overThreadId === thread.id && draggingId !== null && draggingId !== thread.id}
            onSelect={() => void onSelectThread(thread.id)}
            onPromote={() => void onPromoteThread(thread.id)}
            onComplete={() => void onCompleteThread(thread.id)}
            onCancelRename={() => setRenamingId(null)}
            onSubmitRename={async (newTitle) => {
              const trimmed = newTitle.trim();
              setRenamingId(null);
              if (!trimmed || trimmed === thread.title) return;
              await onRenameThread?.(thread.id, trimmed);
            }}
            onContextMenu={(event) => {
              event.preventDefault();
              setContextMenu({ x: event.clientX, y: event.clientY, thread });
            }}
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
        <button type="button" data-testid="thread-rail-new" style={smallBtn} onClick={() => setShowCreate(true)} title="Create thread">
          + New thread
        </button>
        {completed.length > 0 ? (
          <div style={{ position: "relative" }}>
            <button type="button" style={smallBtn} onClick={() => setShowOverflow((v) => !v)}>
              … {completed.length} done ▾
            </button>
            {showOverflow ? (
              <OverflowDropdown
                threads={completed}
                selectedThreadId={selectedThreadId}
                onSelect={(id) => {
                  setShowOverflow(false);
                  void onSelectThread(id);
                }}
                onClose={() => setShowOverflow(false)}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      {showCreate ? (
        <CreateThreadModal
          nextIndex={threads.length + 1}
          onCancel={() => setShowCreate(false)}
          onSubmit={async (title) => {
            await onCreateThread(title);
            setShowCreate(false);
          }}
        />
      ) : null}
      {contextMenu ? (
        <ContextMenu
          items={contextMenuItems}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={180}
        />
      ) : null}
      {settingsThread ? (
        <div style={backdropStyle}>
          <div style={settingsModalStyle}>
            <div style={settingsModalHeaderStyle}>
              <span>Thread settings — {settingsThread.title}</span>
              <button type="button" onClick={() => setSettingsThread(null)} style={closeBtnStyle} aria-label="Close">×</button>
            </div>
            <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <label style={settingsLabelStyle}>
                <span>Custom prompt</span>
                <span style={{ color: "var(--muted)", fontSize: 11 }}>
                  This prompt is appended to the agent's system prompt for this thread.
                </span>
                <textarea
                  value={settingsPrompt}
                  onChange={(e) => setSettingsPrompt(e.target.value)}
                  rows={6}
                  style={{ ...settingsInputStyle, resize: "vertical", fontFamily: "inherit" }}
                  placeholder="Enter standing instructions for this thread…"
                />
              </label>
              <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
                <button type="button" onClick={() => setSettingsThread(null)} style={settingsBtnStyle}>Cancel</button>
                <button type="button" onClick={() => void saveThreadSettings()} style={settingsBtnStyle} disabled={settingsSaving}>
                  {settingsSaving ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : null}
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
  onComplete,
  onContextMenu,
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
  onComplete(): void;
  onContextMenu?(event: React.MouseEvent): void;
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

  // Count items as "done-ish" when status is done, canceled, or
  // human_check (the latter is waiting on user review but the agent's
  // work is complete).
  const total = workState?.items.length ?? 0;
  const done = workState
    ? workState.items.filter((i) => i.status === "done" || i.status === "canceled" || i.status === "archived" || i.status === "human_check").length
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
        onContextMenu={onContextMenu}
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
      </div>
      {hovered && !isDragging ? (
        <HoverCard
          thread={thread}
          isActive={isActive}
          agentStatus={agentStatus}
          workState={workState}
          hasQueued={hasQueued}
          onPromote={onPromote}
          onComplete={onComplete}
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
  hasQueued,
  onPromote,
  onComplete,
}: {
  thread: Thread;
  isActive: boolean;
  agentStatus: AgentStatus;
  workState: ThreadWorkState | undefined;
  hasQueued: boolean;
  onPromote(): void;
  onComplete(): void;
}) {
  const total = workState?.items.length ?? 0;
  const waiting = workState?.waiting.length ?? 0;
  const inProgress = workState?.inProgress ?? [];
  const done = workState?.done.length ?? 0;
  // "writer" means this thread is the one allowed to commit changes; every
  // other live thread stays read-only. "completed" threads are archived.
  const statusLabel = isActive ? "writer" : thread.status === "completed" ? "completed" : "read-only";
  const statusColor =
    statusLabel === "writer" ? "#86efac" : statusLabel === "read-only" ? "#7dd3fc" : "#c4b5fd";

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
        {isActive ? (
          <button type="button" data-testid={`thread-chip-complete-${thread.id}`} style={smallBtn} onClick={onComplete} disabled={!hasQueued} title="Mark this thread done and hand the writer role to the next queued thread">
            Complete thread
          </button>
        ) : thread.status !== "completed" ? (
          <button type="button" data-testid={`thread-chip-promote-${thread.id}`} style={smallBtn} onClick={onPromote} title="Make this thread the writer — only one thread can write at a time">
            Make writer
          </button>
        ) : null}
      </div>
    </div>
  );
}

function OverflowDropdown({
  threads,
  selectedThreadId,
  onSelect,
  onClose,
}: {
  threads: Thread[];
  selectedThreadId: string | null;
  onSelect(id: string): void;
  onClose(): void;
}) {
  useEffect(() => {
    function handler(e: MouseEvent) {
      const target = e.target as HTMLElement | null;
      if (target && target.closest("[data-thread-overflow]")) return;
      onClose();
    }
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [onClose]);
  return (
    <div data-thread-overflow style={overflowStyle}>
      {threads.map((thread) => (
        <button type="button"
          key={thread.id}
          onClick={() => onSelect(thread.id)}
          style={{
            ...overflowItemStyle,
            background: thread.id === selectedThreadId ? "rgba(74, 158, 255, 0.18)" : "transparent",
          }}
        >
          <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {thread.title}
          </span>
          <span style={{ color: "var(--muted)", fontSize: 10 }}>completed</span>
        </button>
      ))}
    </div>
  );
}

function CreateThreadModal({
  nextIndex,
  onSubmit,
  onCancel,
}: {
  nextIndex: number;
  onSubmit(title: string): Promise<void>;
  onCancel(): void;
}) {
  const [title, setTitle] = useState(`Thread ${nextIndex}`);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    function handler(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    }
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onCancel]);

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
    <div role="dialog" aria-modal="true" data-testid="thread-rail-create-modal" style={backdropStyle}>
      <form
        style={createModalStyle}
        onSubmit={(e) => {
          e.preventDefault();
          if (!canSubmit) return;
          void submit();
        }}
      >
        <div style={settingsModalHeaderStyle}>
          <span>New thread</span>
          <button type="button" onClick={onCancel} style={closeBtnStyle} aria-label="Close">×</button>
        </div>
        <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
          <label style={settingsLabelStyle}>
            <span>Title</span>
            <input
              ref={inputRef}
              value={title}
              autoFocus
              onChange={(e) => setTitle(e.target.value)}
              style={settingsInputStyle}
              placeholder="Thread title"
              data-testid="thread-rail-create-input"
            />
          </label>
          {error ? <span style={{ color: "#ff6b6b", fontSize: 11 }}>{error}</span> : null}
          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <button type="button" onClick={onCancel} style={settingsBtnStyle}>Cancel</button>
            <button
              type="submit"
              data-testid="thread-rail-create-submit"
              style={settingsBtnStyle}
              disabled={!canSubmit}
            >
              {submitting ? "Creating…" : "Create"}
            </button>
          </div>
        </div>
      </form>
    </div>
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

const overflowStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  right: 0,
  zIndex: 20,
  minWidth: 220,
  maxHeight: 300,
  overflowY: "auto",
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 6,
  boxShadow: "0 6px 24px rgba(0,0,0,0.4)",
  display: "flex",
  flexDirection: "column",
  padding: 4,
};

const overflowItemStyle: CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  gap: 8,
  padding: "6px 8px",
  border: "none",
  borderRadius: 4,
  color: "inherit",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
  textAlign: "left",
};

const createModalStyle: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 24px 60px rgba(0,0,0,0.5)",
  minWidth: 420,
  maxWidth: 560,
  display: "flex",
  flexDirection: "column",
};

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1100,
};

const settingsModalStyle: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 24px 60px rgba(0,0,0,0.5)",
  minWidth: 480,
  maxWidth: 640,
  maxHeight: "80vh",
  display: "flex",
  flexDirection: "column",
};

const settingsModalHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "10px 14px",
  borderBottom: "1px solid var(--border)",
  fontSize: 13,
  fontWeight: 600,
  background: "var(--bg-1, var(--bg-2))",
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

const settingsBtnStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "6px 12px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
};

const closeBtnStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--muted)",
  fontSize: 20,
  lineHeight: 1,
  cursor: "pointer",
};

const settingsLabelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: 12 };
