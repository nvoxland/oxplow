import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentStatus,
  AgentTurn,
  Batch,
  BatchFileChange,
  BatchWorkState,
} from "../api.js";
import { AgentStatusDot } from "./AgentStatusDot.js";
import { ContextMenu } from "./ContextMenu.js";
import type { MenuItem } from "../menu.js";

interface Props {
  batches: Batch[];
  activeBatchId: string | null;
  selectedBatchId: string | null;
  agentStatuses: Record<string, AgentStatus>;
  batchWorkStates: Record<string, BatchWorkState>;
  agentTurns: Record<string, AgentTurn[]>;
  fileChanges: Record<string, BatchFileChange[]>;
  onSelectBatch(batchId: string): void | Promise<void>;
  onCreateBatch(title: string): Promise<void>;
  onPromoteBatch(batchId: string): void | Promise<void>;
  onCompleteBatch(batchId: string): void | Promise<void>;
  onMoveWorkItem?(itemId: string, fromBatchId: string, toBatchId: string): Promise<void>;
  onMoveBacklogItemToBatch?(itemId: string, toBatchId: string): Promise<void>;
  onRenameBatch?(batchId: string, currentTitle: string): void;
  onRequestCreateStream?(): void;
  onRequestCreateBatch?(): void;
  /** Bumping this number opens the inline "new batch" form. */
  createRequest?: number;
}

export const WORK_ITEM_DRAG_MIME = "application/x-newde-work-item";

export function BatchRail({
  batches,
  activeBatchId,
  selectedBatchId,
  agentStatuses,
  batchWorkStates,
  agentTurns,
  fileChanges,
  onSelectBatch,
  onCreateBatch,
  onPromoteBatch,
  onCompleteBatch,
  onMoveWorkItem,
  onMoveBacklogItemToBatch,
  onRenameBatch,
  onRequestCreateStream,
  onRequestCreateBatch,
  createRequest,
}: Props) {
  const { ordered, completed } = useMemo(() => {
    const active = batches.find((b) => b.id === activeBatchId && b.status !== "completed");
    const queued = batches
      .filter((b) => b.status === "queued" && b.id !== activeBatchId)
      .sort((a, b) => a.sort_index - b.sort_index);
    const other = batches
      .filter((b) => b.status === "active" && b.id !== activeBatchId)
      .sort((a, b) => a.sort_index - b.sort_index);
    const ordered: Batch[] = [];
    if (active) ordered.push(active);
    ordered.push(...other, ...queued);
    const completed = batches
      .filter((b) => b.status === "completed")
      .sort((a, b) => b.sort_index - a.sort_index);
    return { ordered, completed };
  }, [batches, activeBatchId]);

  const hasQueued = batches.some((b) => b.status === "queued");
  const [showOverflow, setShowOverflow] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; batch: Batch } | null>(null);

  useEffect(() => {
    if (createRequest === undefined || createRequest === 0) return;
    setShowCreate(true);
  }, [createRequest]);

  const contextMenuItems: MenuItem[] = contextMenu
    ? [
        {
          id: "batch.rename",
          label: "Rename…",
          enabled: !!onRenameBatch,
          run: () => onRenameBatch?.(contextMenu.batch.id, contextMenu.batch.title),
        },
        {
          id: "batch.add-batch",
          label: "Add batch",
          enabled: !!onRequestCreateBatch,
          run: () => (onRequestCreateBatch ? onRequestCreateBatch() : setShowCreate(true)),
        },
        {
          id: "batch.add-stream",
          label: "Add stream",
          enabled: !!onRequestCreateStream,
          run: () => onRequestCreateStream?.(),
        },
      ]
    : [];

  return (
    <div style={railStyle}>
      <div className="newde-rail-scroll" style={{ display: "flex", alignItems: "stretch", gap: 0, flex: 1, minWidth: 0, overflowX: "auto" }}>
        {ordered.length === 0 && completed.length === 0 ? (
          <span style={{ color: "var(--muted)", fontSize: 11, padding: "8px 12px", alignSelf: "center" }}>No batches yet.</span>
        ) : null}
        {ordered.map((batch) => (
          <BatchChip
            key={batch.id}
            batch={batch}
            isActive={batch.id === activeBatchId}
            isSelected={batch.id === selectedBatchId}
            agentStatus={agentStatuses[batch.id] ?? "idle"}
            workState={batchWorkStates[batch.id]}
            turns={agentTurns[batch.id]}
            fileChangeCount={fileChanges[batch.id]?.length}
            hasQueued={hasQueued}
            onSelect={() => void onSelectBatch(batch.id)}
            onPromote={() => void onPromoteBatch(batch.id)}
            onComplete={() => void onCompleteBatch(batch.id)}
            onContextMenu={(event) => {
              event.preventDefault();
              setContextMenu({ x: event.clientX, y: event.clientY, batch });
            }}
            onDropWorkItem={(onMoveWorkItem || onMoveBacklogItemToBatch) ? (payload) => {
              if (payload.fromBatchId === null) {
                if (onMoveBacklogItemToBatch) void onMoveBacklogItemToBatch(payload.itemId, batch.id);
              } else {
                if (onMoveWorkItem) void onMoveWorkItem(payload.itemId, payload.fromBatchId, batch.id);
              }
            } : undefined}
          />
        ))}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, flexShrink: 0, paddingLeft: 8 }}>
        <button style={smallBtn} onClick={() => setShowCreate((v) => !v)} title="Create batch">
          {showCreate ? "Cancel" : "+ New batch"}
        </button>
        {completed.length > 0 ? (
          <div style={{ position: "relative" }}>
            <button style={smallBtn} onClick={() => setShowOverflow((v) => !v)}>
              … {completed.length} done ▾
            </button>
            {showOverflow ? (
              <OverflowDropdown
                batches={completed}
                selectedBatchId={selectedBatchId}
                onSelect={(id) => {
                  setShowOverflow(false);
                  void onSelectBatch(id);
                }}
                onClose={() => setShowOverflow(false)}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      {showCreate ? (
        <CreateBatchInput
          nextIndex={batches.length + 1}
          onCancel={() => setShowCreate(false)}
          onSubmit={async (title) => {
            await onCreateBatch(title);
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
    </div>
  );
}

function BatchChip({
  batch,
  isActive,
  isSelected,
  agentStatus,
  workState,
  turns,
  fileChangeCount,
  hasQueued,
  onSelect,
  onPromote,
  onComplete,
  onContextMenu,
  onDropWorkItem,
}: {
  batch: Batch;
  isActive: boolean;
  isSelected: boolean;
  agentStatus: AgentStatus;
  workState: BatchWorkState | undefined;
  turns: AgentTurn[] | undefined;
  fileChangeCount: number | undefined;
  hasQueued: boolean;
  onSelect(): void;
  onPromote(): void;
  onComplete(): void;
  onContextMenu?(event: React.MouseEvent): void;
  onDropWorkItem?(payload: { itemId: string; fromBatchId: string | null }): void;
}) {
  const [hovered, setHovered] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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

  const total = workState?.items.length ?? 0;
  const done = workState?.done.length ?? 0;
  // Two orthogonal states on each tab:
  //  - `isSelected`: what the user is viewing (main bg, accent underline)
  //  - `isActive`:   the designated *writer* batch (the one allowed to write
  //                  to disk). All queued batches can have agents running;
  //                  only the writer can commit changes.
  // The writer is visualised with a pencil badge so it's obvious even when
  // another batch is selected.
  const background = isSelected ? "var(--bg)" : "transparent";
  const color = isSelected ? "var(--fg)" : "var(--muted)";

  const handleDragOver = (event: React.DragEvent) => {
    if (!onDropWorkItem) return;
    const types = event.dataTransfer.types;
    if (!types || !Array.from(types).includes(WORK_ITEM_DRAG_MIME)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    if (!dragOver) setDragOver(true);
  };
  const handleDragLeave = () => {
    if (dragOver) setDragOver(false);
  };
  const handleDrop = (event: React.DragEvent) => {
    if (!onDropWorkItem) return;
    const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
    if (!raw) return;
    event.preventDefault();
    setDragOver(false);
    try {
      const payload = JSON.parse(raw) as { itemId?: string; fromBatchId?: string | null };
      if (!payload.itemId) return;
      const fromBatchId = payload.fromBatchId ?? null;
      if (fromBatchId === batch.id) return;
      onDropWorkItem({ itemId: payload.itemId, fromBatchId });
    } catch {
      // ignore malformed payload
    }
  };

  return (
    <div
      style={{ position: "relative", flexShrink: 0 }}
      onMouseEnter={scheduleShow}
      onMouseLeave={cancelShow}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <button
        onClick={onSelect}
        onContextMenu={onContextMenu}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          padding: "6px 12px",
          border: "none",
          borderRight: "1px solid var(--border)",
          borderBottom: isSelected ? "2px solid var(--accent)" : "2px solid transparent",
          background,
          color,
          cursor: "pointer",
          fontFamily: "inherit",
          fontSize: 12,
          whiteSpace: "nowrap",
          marginBottom: -1, // overlap the rail's bottom border so the tab looks connected to the content below when selected
          boxShadow: dragOver ? "inset 0 0 0 2px var(--accent)" : undefined,
        }}
        title={isActive ? `${batch.title} · writer (can commit)` : `${batch.title} (read-only)`}
      >
        <AgentStatusDot status={agentStatus} />
        {isActive ? (
          <span
            aria-label="Writer batch"
            title="Writer batch — only this one can commit changes"
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
        <span style={{ fontWeight: isSelected ? 600 : isActive ? 500 : 400 }}>{batch.title}</span>
        {total > 0 ? (
          <span style={{ fontSize: 10, opacity: 0.75 }}>
            {done}/{total}
          </span>
        ) : null}
      </button>
      {hovered ? (
        <HoverCard
          batch={batch}
          isActive={isActive}
          agentStatus={agentStatus}
          workState={workState}
          turns={turns}
          fileChangeCount={fileChangeCount}
          hasQueued={hasQueued}
          onPromote={onPromote}
          onComplete={onComplete}
        />
      ) : null}
    </div>
  );
}

function HoverCard({
  batch,
  isActive,
  agentStatus,
  workState,
  turns,
  fileChangeCount,
  hasQueued,
  onPromote,
  onComplete,
}: {
  batch: Batch;
  isActive: boolean;
  agentStatus: AgentStatus;
  workState: BatchWorkState | undefined;
  turns: AgentTurn[] | undefined;
  fileChangeCount: number | undefined;
  hasQueued: boolean;
  onPromote(): void;
  onComplete(): void;
}) {
  const total = workState?.items.length ?? 0;
  const waiting = workState?.waiting.length ?? 0;
  const inProgress = workState?.inProgress ?? [];
  const done = workState?.done.length ?? 0;
  const turnCount = turns?.length ?? 0;
  const lastTurn = turns && turns.length > 0 ? turns[turns.length - 1] : null;
  // "writer" means this batch is the one allowed to commit changes; every
  // other live batch stays read-only. "completed" batches are archived.
  const statusLabel = isActive ? "writer" : batch.status === "completed" ? "completed" : "read-only";
  const statusColor =
    statusLabel === "writer" ? "#86efac" : statusLabel === "read-only" ? "#7dd3fc" : "#c4b5fd";

  return (
    <div style={hoverCardStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <strong style={{ fontSize: 13 }}>{batch.title}</strong>
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
        <span>turns: {turnCount}</span>
        {fileChangeCount != null ? <span>files: {fileChangeCount}</span> : null}
      </div>
      {lastTurn ? (
        <div style={{ fontSize: 11, color: "var(--muted)" }}>
          last turn: {relativeTime(lastTurn.started_at)}
        </div>
      ) : null}
      {batch.summary ? (
        <div style={{ fontSize: 11, color: "var(--fg)", lineHeight: 1.4, whiteSpace: "pre-wrap" }}>
          {batch.summary}
        </div>
      ) : null}
      <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
        {isActive ? (
          <button style={smallBtn} onClick={onComplete} disabled={!hasQueued} title="Mark this batch done and hand the writer role to the next queued batch">
            Complete batch
          </button>
        ) : batch.status !== "completed" ? (
          <button style={smallBtn} onClick={onPromote} title="Make this batch the writer — only one batch can write at a time">
            Make writer
          </button>
        ) : null}
      </div>
    </div>
  );
}

function OverflowDropdown({
  batches,
  selectedBatchId,
  onSelect,
  onClose,
}: {
  batches: Batch[];
  selectedBatchId: string | null;
  onSelect(id: string): void;
  onClose(): void;
}) {
  useEffect(() => {
    function handler(e: MouseEvent) {
      const target = e.target as HTMLElement | null;
      if (target && target.closest("[data-batch-overflow]")) return;
      onClose();
    }
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [onClose]);
  return (
    <div data-batch-overflow style={overflowStyle}>
      {batches.map((batch) => (
        <button
          key={batch.id}
          onClick={() => onSelect(batch.id)}
          style={{
            ...overflowItemStyle,
            background: batch.id === selectedBatchId ? "rgba(74, 158, 255, 0.18)" : "transparent",
          }}
        >
          <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {batch.title}
          </span>
          <span style={{ color: "var(--muted)", fontSize: 10 }}>completed</span>
        </button>
      ))}
    </div>
  );
}

function CreateBatchInput({
  nextIndex,
  onSubmit,
  onCancel,
}: {
  nextIndex: number;
  onSubmit(title: string): Promise<void>;
  onCancel(): void;
}) {
  const [title, setTitle] = useState(`Batch ${nextIndex}`);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <div style={createRowStyle}>
      <input
        value={title}
        autoFocus
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void submit();
          if (e.key === "Escape") onCancel();
        }}
        style={inputStyle}
        placeholder="Batch title"
      />
      <button style={smallBtn} onClick={() => void submit()} disabled={submitting}>
        {submitting ? "Creating…" : "Create"}
      </button>
      {error ? <span style={{ color: "#ff6b6b", fontSize: 11 }}>{error}</span> : null}
    </div>
  );
}

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return iso;
  const diff = Date.now() - then;
  const sec = Math.round(diff / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  return `${day}d ago`;
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
  background: "var(--bg-2)",
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

const createRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  flexBasis: "100%",
  marginTop: 4,
};

const inputStyle: CSSProperties = {
  flex: 1,
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 8px",
  fontFamily: "inherit",
  fontSize: 12,
};
