import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import type {
  BacklogState,
  Batch,
  BatchWorkState,
  CommitPoint,
  CommitPointMode,
  WaitPoint,
  WorkItem,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "../../api.js";
import {
  approveCommitPoint,
  createCommitPoint,
  createWaitPoint,
  deleteCommitPoint,
  deleteWaitPoint,
  listCommitPoints,
  listWaitPoints,
  rejectCommitPoint,
  reorderBatchQueue,
  resetCommitPoint,
  setCommitPointMode,
  setWaitPointNote,
  subscribeNewdeEvents,
} from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";

interface CreateInput {
  kind: WorkItemKind;
  title: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

interface UpdateChanges {
  title?: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

interface Props {
  batch: Batch | null;
  batchWork: BatchWorkState | null;
  backlog: BacklogState | null;
  onCreateWorkItem(input: CreateInput): Promise<void>;
  onUpdateWorkItem(itemId: string, changes: UpdateChanges): Promise<void>;
  onDeleteWorkItem(itemId: string): Promise<void>;
  onReorderWorkItems(orderedItemIds: string[]): Promise<void>;
  onCreateBacklogItem(input: CreateInput): Promise<void>;
  onUpdateBacklogItem(itemId: string, changes: UpdateChanges): Promise<void>;
  onDeleteBacklogItem(itemId: string): Promise<void>;
  onReorderBacklog(orderedItemIds: string[]): Promise<void>;
  onMoveItemToBacklog(itemId: string, fromBatchId: string): Promise<void>;
  openNewRequest?: number;
}

interface ContextMenuState {
  x: number;
  y: number;
  item: WorkItem;
}

export function PlanPane({
  batch,
  batchWork,
  backlog,
  onCreateWorkItem,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
  onCreateBacklogItem,
  onUpdateBacklogItem,
  onDeleteBacklogItem,
  onReorderBacklog,
  onMoveItemToBacklog,
  openNewRequest,
}: Props) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [acceptance, setAcceptance] = useState("");
  const [kind, setKind] = useState<WorkItemKind>("task");
  const [priority, setPriority] = useState<WorkItemPriority>("medium");
  const [parentId, setParentId] = useState<string>("");
  const [showCompleted, setShowCompleted] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [mode, setMode] = useState<"batch" | "backlog">("batch");
  const [backlogChipDragOver, setBacklogChipDragOver] = useState(false);
  const [commitPoints, setCommitPoints] = useState<CommitPoint[]>([]);
  const [waitPoints, setWaitPoints] = useState<WaitPoint[]>([]);

  const batchId = batch?.id ?? null;
  const streamId = batch?.stream_id ?? null;

  useEffect(() => {
    if (!batchId) { setCommitPoints([]); setWaitPoints([]); return; }
    let cancelled = false;
    const refreshCommits = () => void listCommitPoints(batchId)
      .then((points) => { if (!cancelled) setCommitPoints(points); })
      .catch(() => {});
    const refreshWaits = () => void listWaitPoints(batchId)
      .then((points) => { if (!cancelled) setWaitPoints(points); })
      .catch(() => {});
    refreshCommits();
    refreshWaits();
    const off = subscribeNewdeEvents((event) => {
      if (event.type === "commit-point.changed" && event.batchId === batchId) refreshCommits();
      if (event.type === "wait-point.changed" && event.batchId === batchId) refreshWaits();
    });
    return () => { cancelled = true; off(); };
  }, [batchId]);

  const epics = batchWork?.epics ?? [];

  const groups = useMemo(() => {
    if (mode === "backlog") return buildBacklogGroups(backlog, showCompleted);
    return buildGroups(batchWork, showCompleted);
  }, [mode, batchWork, backlog, showCompleted]);

  const activeCreate = mode === "backlog" ? onCreateBacklogItem : onCreateWorkItem;
  const activeUpdate = mode === "backlog" ? onUpdateBacklogItem : onUpdateWorkItem;
  const activeDelete = mode === "backlog" ? onDeleteBacklogItem : onDeleteWorkItem;
  const activeReorder = mode === "backlog" ? onReorderBacklog : onReorderWorkItems;
  const currentScopeBatchId = mode === "backlog" ? null : batch?.id ?? null;

  useEffect(() => {
    if (openNewRequest === undefined || openNewRequest === 0) return;
    setCreateOpen(true);
  }, [openNewRequest]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [contextMenu]);

  if (mode === "batch" && !batch) {
    return <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>;
  }

  const handleBacklogChipDragOver = (event: React.DragEvent) => {
    const types = event.dataTransfer.types;
    if (!types || !Array.from(types).includes(WORK_ITEM_DRAG_MIME)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    if (!backlogChipDragOver) setBacklogChipDragOver(true);
  };

  const handleBacklogChipDrop = (event: React.DragEvent) => {
    const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
    setBacklogChipDragOver(false);
    if (!raw) return;
    event.preventDefault();
    try {
      const payload = JSON.parse(raw) as { itemId?: string; fromBatchId?: string | null };
      if (payload.itemId && payload.fromBatchId) {
        void onMoveItemToBacklog(payload.itemId, payload.fromBatchId);
      }
    } catch {
      // ignore malformed payload
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <div style={{ padding: 8, borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <button onClick={() => setCreateOpen(true)} style={{ ...miniButtonStyle, padding: "4px 10px" }}>
            + New work item
          </button>
          <span style={{ color: "var(--muted)", fontSize: 11 }}>
            {mode === "backlog" ? "Backlog" : ""}
          </span>
        </div>
        <label style={{ display: "inline-flex", alignItems: "center", gap: 6, color: "var(--muted)", fontSize: 11, cursor: "pointer" }}>
          <input type="checkbox" checked={showCompleted} onChange={(event) => setShowCompleted(event.target.checked)} />
          Show completed
        </label>
      </div>
      {createOpen ? (
        <NewWorkItemModal
          title={title}
          setTitle={setTitle}
          description={description}
          setDescription={setDescription}
          acceptance={acceptance}
          setAcceptance={setAcceptance}
          kind={kind}
          setKind={setKind}
          priority={priority}
          setPriority={setPriority}
          parentId={parentId}
          setParentId={setParentId}
          epics={mode === "batch" ? epics : []}
          showParent={mode === "batch"}
          modalTitle={mode === "backlog" ? "New backlog item" : "New work item"}
          onClose={() => setCreateOpen(false)}
          onSubmit={async (andAnother) => {
            const nextTitle = title.trim();
            if (!nextTitle) return;
            await activeCreate({
              kind,
              title: nextTitle,
              description,
              acceptanceCriteria: acceptance || null,
              parentId: mode === "batch" ? (parentId || undefined) : undefined,
              priority,
              status: kind === "epic" ? "in_progress" : "waiting",
            });
            setTitle(""); setDescription(""); setAcceptance("");
            if (!andAnother) {
              setParentId(""); setKind("task"); setPriority("medium");
              setCreateOpen(false);
            }
          }}
        />
      ) : null}
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {groups.length === 0 ? (
          <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>
            {mode === "backlog" ? "Backlog is empty." : "No work items."}
          </div>
        ) : (
          groups.map((group) => (
            <WorkGroupList
              key={group.epic?.id ?? "__root__"}
              group={group}
              scopeBatchId={currentScopeBatchId}
              expandedId={expandedId}
              onToggleExpand={(id) => setExpandedId((prev) => (prev === id ? null : id))}
              onUpdateWorkItem={activeUpdate}
              onReorderWorkItems={activeReorder}
              // Commit/wait points only belong to the root (non-epic) group —
              // they divide the top-level queue, not items scoped inside an
              // epic.
              commitPoints={group.epic === null && mode === "batch" ? commitPoints : []}
              waitPoints={group.epic === null && mode === "batch" ? waitPoints : []}
              onReorderMixed={group.epic === null && mode === "batch" && streamId && batchId
                ? (entries) => { void reorderBatchQueue(streamId, batchId, entries).catch(() => {}); }
                : undefined}
              onRequestDelete={(item) => {
                if (!window.confirm(`Delete "${item.title}"?`)) return;
                if (expandedId === item.id) setExpandedId(null);
                void activeDelete(item.id);
              }}
              onContextMenu={(event, item) => {
                event.preventDefault();
                setContextMenu({ x: event.clientX, y: event.clientY, item });
              }}
            />
          ))
        )}
        {mode === "batch" && batch ? (
          <div style={queueMarkerBarStyle}>
            <button
              onClick={() => {
                if (!streamId || !batchId) return;
                void createCommitPoint(streamId, batchId, "approval").catch(() => {});
              }}
              disabled={!batchWork || batchWork.items.length === 0}
              style={{
                ...miniButtonStyle,
                opacity: !batchWork || batchWork.items.length === 0 ? 0.5 : 1,
                cursor: !batchWork || batchWork.items.length === 0 ? "not-allowed" : "pointer",
              }}
              title={
                !batchWork || batchWork.items.length === 0
                  ? "Add a work item first — a commit point can't be the very first queue entry"
                  : "Append a commit point to the work queue (drag to reposition; click to switch to auto-commit)"
              }
            >
              + Commit when done
            </button>
            <button
              onClick={() => {
                if (!streamId || !batchId) return;
                void createWaitPoint(streamId, batchId, null).catch(() => {});
              }}
              disabled={!batchWork || batchWork.items.length === 0}
              style={{
                ...miniButtonStyle,
                opacity: !batchWork || batchWork.items.length === 0 ? 0.5 : 1,
                cursor: !batchWork || batchWork.items.length === 0 ? "not-allowed" : "pointer",
              }}
              title={
                !batchWork || batchWork.items.length === 0
                  ? "Add a work item first — a wait point can't be the very first queue entry"
                  : "Append a wait point to the work queue (drag to reposition; click the divider to add a note)"
              }
            >
              + Wait here
            </button>
          </div>
        ) : null}
      </div>
      <div style={bottomBarStyle}>
        <button
          onClick={() => setMode((prev) => (prev === "backlog" ? "batch" : "backlog"))}
          onDragOver={handleBacklogChipDragOver}
          onDragLeave={() => setBacklogChipDragOver(false)}
          onDrop={handleBacklogChipDrop}
          style={{
            ...bottomChipStyle,
            background: mode === "backlog" ? "var(--accent)" : "var(--bg-2)",
            color: mode === "backlog" ? "#fff" : "inherit",
            borderColor: backlogChipDragOver ? "var(--accent)" : "var(--border)",
            boxShadow: backlogChipDragOver ? "0 0 0 2px var(--accent)" : undefined,
          }}
          title="Backlog (global across streams)"
        >
          Backlog{backlog ? ` · ${backlog.items.length}` : ""}
        </button>
      </div>
      {contextMenu ? (
        <ContextMenu
          menu={contextMenu}
          onDelete={() => {
            const item = contextMenu.item;
            setContextMenu(null);
            if (!window.confirm(`Delete "${item.title}"?`)) return;
            if (expandedId === item.id) setExpandedId(null);
            void activeDelete(item.id);
          }}
        />
      ) : null}
    </div>
  );
}

function NewWorkItemModal({
  title,
  setTitle,
  description,
  setDescription,
  acceptance,
  setAcceptance,
  kind,
  setKind,
  priority,
  setPriority,
  parentId,
  setParentId,
  epics,
  showParent = true,
  modalTitle = "New work item",
  onClose,
  onSubmit,
}: {
  title: string;
  setTitle(value: string): void;
  description: string;
  setDescription(value: string): void;
  acceptance: string;
  setAcceptance(value: string): void;
  kind: WorkItemKind;
  setKind(value: WorkItemKind): void;
  priority: WorkItemPriority;
  setPriority(value: WorkItemPriority): void;
  parentId: string;
  setParentId(value: string): void;
  epics: WorkItem[];
  showParent?: boolean;
  modalTitle?: string;
  onClose(): void;
  onSubmit(andAnother: boolean): Promise<void>;
}) {
  const canSubmit = title.trim().length > 0;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 2000,
      }}
      onClick={onClose}
    >
      <form
        onClick={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (!canSubmit) return;
          void onSubmit(false);
        }}
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 16,
          width: "min(520px, 90vw)",
          maxHeight: "90vh",
          overflow: "auto",
          display: "flex",
          flexDirection: "column",
          gap: 8,
          boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div style={{ fontWeight: 600 }}>{modalTitle}</div>
          <button type="button" onClick={onClose} style={{ ...miniButtonStyle, border: "none", background: "transparent" }} aria-label="Close">✕</button>
        </div>
        <input
          autoFocus
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Title (required)"
          style={inputStyle}
        />
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
          <select value={kind} onChange={(e) => setKind(e.target.value as WorkItemKind)} style={inputStyle}>
            <option value="epic">Epic</option>
            <option value="task">Task</option>
            <option value="subtask">Subtask</option>
            <option value="bug">Bug</option>
            <option value="note">Note</option>
          </select>
          <select value={priority} onChange={(e) => setPriority(e.target.value as WorkItemPriority)} style={inputStyle}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
            <option value="urgent">Urgent</option>
          </select>
        </div>
        {showParent ? (
        <select value={parentId} onChange={(e) => setParentId(e.target.value)} style={inputStyle}>
          <option value="">No parent epic</option>
          {epics.map((epic) => (<option key={epic.id} value={epic.id}>{epic.title}</option>))}
        </select>
        ) : null}
        <textarea
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Description"
          style={{ ...inputStyle, minHeight: 64, resize: "vertical" }}
        />
        <textarea
          value={acceptance}
          onChange={(e) => setAcceptance(e.target.value)}
          placeholder="Acceptance criteria, one per line"
          style={{ ...inputStyle, minHeight: 64, resize: "vertical" }}
        />
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginTop: 4 }}>
          <button type="button" onClick={onClose} style={miniButtonStyle}>Cancel</button>
          <button
            type="button"
            disabled={!canSubmit}
            onClick={() => { if (canSubmit) void onSubmit(true); }}
            style={{ ...miniButtonStyle, padding: "6px 10px", opacity: canSubmit ? 1 : 0.5 }}
          >Save and Another</button>
          <button
            type="submit"
            disabled={!canSubmit}
            style={{ ...buttonStyle, opacity: canSubmit ? 1 : 0.5 }}
          >Save</button>
        </div>
      </form>
    </div>
  );
}

type QueueRow =
  | { kind: "work"; id: string; sortIndex: number; item: WorkItem }
  | { kind: "commit"; id: string; sortIndex: number; cp: CommitPoint }
  | { kind: "wait"; id: string; sortIndex: number; wp: WaitPoint };

function WorkGroupList({
  group,
  scopeBatchId,
  expandedId,
  onToggleExpand,
  onUpdateWorkItem,
  onReorderWorkItems,
  commitPoints,
  waitPoints,
  onReorderMixed,
  onRequestDelete,
  onContextMenu,
}: {
  group: { epic: WorkItem | null; items: WorkItem[] };
  scopeBatchId: string | null;
  expandedId: string | null;
  onToggleExpand(id: string): void;
  onUpdateWorkItem: (itemId: string, changes: UpdateChanges) => Promise<void>;
  onReorderWorkItems: (orderedItemIds: string[]) => Promise<void>;
  commitPoints?: CommitPoint[];
  waitPoints?: WaitPoint[];
  onReorderMixed?(entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>): void;
  onRequestDelete(item: WorkItem): void;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);

  // Merge work items + commit points into one ordered list by sort_index.
  // Commit points only apply at the root group; epic groups get just their
  // own child work items.
  const rows: QueueRow[] = useMemo(() => {
    const work: QueueRow[] = group.items.map((item) => ({
      kind: "work" as const, id: item.id, sortIndex: item.sort_index, item,
    }));
    const commits: QueueRow[] = (commitPoints ?? []).map((cp) => ({
      kind: "commit" as const, id: cp.id, sortIndex: cp.sort_index, cp,
    }));
    const waits: QueueRow[] = (waitPoints ?? []).map((wp) => ({
      kind: "wait" as const, id: wp.id, sortIndex: wp.sort_index, wp,
    }));
    return [...work, ...commits, ...waits].sort((a, b) => a.sortIndex - b.sortIndex);
  }, [group.items, commitPoints, waitPoints]);

  const keyFor = (row: { kind: string; id: string }) => `${row.kind}:${row.id}`;

  const handleDropOnKey = (targetKey: string) => {
    if (!draggingKey || draggingKey === targetKey) {
      setDraggingKey(null); setOverKey(null); return;
    }
    const from = rows.findIndex((row) => keyFor(row) === draggingKey);
    const to = rows.findIndex((row) => keyFor(row) === targetKey);
    if (from < 0 || to < 0) {
      setDraggingKey(null); setOverKey(null); return;
    }
    const next = rows.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
    setDraggingKey(null); setOverKey(null);
    if (onReorderMixed && ((commitPoints?.length ?? 0) > 0 || (waitPoints?.length ?? 0) > 0)) {
      // Mixed list — use the cross-store reorder so commit points get their
      // sort_index updated alongside work items.
      onReorderMixed(next.map((row) => ({ kind: row.kind, id: row.id })));
    } else {
      // Pure work-item list — keep using the scope-aware reorder handler
      // (it also handles the backlog mode).
      void onReorderWorkItems(next.filter((row) => row.kind === "work").map((row) => row.id));
    }
  };

  return (
    <div>
      {group.epic ? (
        <div style={groupHeaderStyle}>
          <span style={{ marginRight: 6 }}>{statusIcon(group.epic.status)}</span>
          <span style={{ fontWeight: 600, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{group.epic.title}</span>
          <span style={{ color: "var(--muted)", fontSize: 10 }}>{priorityIcon(group.epic.priority)}</span>
        </div>
      ) : null}
      {rows.map((row) => {
        const key = keyFor(row);
        const isOver = overKey === key && draggingKey !== key;
        const isDragging = draggingKey === key;
        if (row.kind === "commit") {
          const isExpanded = expandedId === key;
          return (
            <div key={key}>
              <div
                draggable
                onDragStart={(event) => {
                  setDraggingKey(key);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", row.id);
                }}
                onDragEnd={() => { setDraggingKey(null); setOverKey(null); }}
                onDragOver={(event) => {
                  if (!draggingKey || draggingKey === key) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  if (overKey !== key) setOverKey(key);
                }}
                onDragLeave={() => { if (overKey === key) setOverKey(null); }}
                onDrop={(event) => { event.preventDefault(); handleDropOnKey(key); }}
                onClick={() => onToggleExpand(key)}
                style={{
                  ...commitDividerStyle,
                  cursor: isDragging ? "grabbing" : "pointer",
                  borderTopColor: isOver ? "var(--accent)" : commitDividerStyle.borderTopColor,
                  background: isDragging ? "rgba(74,158,255,0.08)" : "transparent",
                }}
                title="Commit point — drag to reposition"
              >
                <span style={commitDividerLineStyle} />
                <span style={commitDividerBadgeStyle(row.cp.mode, row.cp.status)}>
                  commit · {row.cp.mode}
                  {row.cp.status !== "pending" && row.cp.status !== "done" ? ` · ${row.cp.status}` : ""}
                  {row.cp.status === "done" ? " · done" : ""}
                </span>
                <span style={commitDividerLineStyle} />
              </div>
              {isExpanded ? <CommitPointRow cp={row.cp} /> : null}
            </div>
          );
        }
        if (row.kind === "wait") {
          const isExpanded = expandedId === key;
          return (
            <div key={key}>
              <div
                draggable
                onDragStart={(event) => {
                  setDraggingKey(key);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", row.id);
                }}
                onDragEnd={() => { setDraggingKey(null); setOverKey(null); }}
                onDragOver={(event) => {
                  if (!draggingKey || draggingKey === key) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  if (overKey !== key) setOverKey(key);
                }}
                onDragLeave={() => { if (overKey === key) setOverKey(null); }}
                onDrop={(event) => { event.preventDefault(); handleDropOnKey(key); }}
                onClick={() => onToggleExpand(key)}
                style={{
                  ...commitDividerStyle,
                  cursor: isDragging ? "grabbing" : "pointer",
                  borderTopColor: isOver ? "var(--accent)" : commitDividerStyle.borderTopColor,
                  background: isDragging ? "rgba(217,119,6,0.08)" : "transparent",
                }}
                title="Wait point — drag to reposition"
              >
                <span style={commitDividerLineStyle} />
                <span style={waitDividerBadgeStyle(row.wp.status)}>
                  wait{row.wp.note ? ` · ${row.wp.note}` : ""}
                  {row.wp.status === "triggered" ? " · stopped" : ""}
                </span>
                <span style={commitDividerLineStyle} />
              </div>
              {isExpanded ? <WaitPointRow wp={row.wp} /> : null}
            </div>
          );
        }
        const item = row.item;
        const isExpanded = expandedId === item.id;
        return (
          <div key={key}>
            <div
              draggable
              onDragStart={(event) => {
                setDraggingKey(key);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", item.id);
                event.dataTransfer.setData(
                  WORK_ITEM_DRAG_MIME,
                  JSON.stringify({ itemId: item.id, fromBatchId: scopeBatchId }),
                );
              }}
              onDragEnd={() => { setDraggingKey(null); setOverKey(null); }}
              onDragOver={(event) => {
                if (!draggingKey || draggingKey === key) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (overKey !== key) setOverKey(key);
              }}
              onDragLeave={() => { if (overKey === key) setOverKey(null); }}
              onDrop={(event) => { event.preventDefault(); handleDropOnKey(key); }}
              onClick={() => onToggleExpand(item.id)}
              onContextMenu={(event) => onContextMenu(event, item)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                padding: "4px 8px",
                cursor: isDragging ? "grabbing" : "pointer",
                borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
                background: isExpanded ? "var(--bg-2)" : isDragging ? "rgba(255,255,255,0.04)" : "transparent",
                fontSize: 12,
                userSelect: "none",
              }}
              title={item.title}
            >
              <span style={{ flexShrink: 0, width: 14, textAlign: "center" }}>{statusIcon(item.status)}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: isExpanded ? 600 : 400 }}>{item.title}</span>
              <span style={{ flexShrink: 0, fontSize: 10, ...priorityStyle(item.priority) }}>{priorityIcon(item.priority)}</span>
            </div>
            {isExpanded ? (
              <WorkItemDetail item={item} onUpdateWorkItem={onUpdateWorkItem} onRequestDelete={() => onRequestDelete(item)} />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

function WorkItemDetail({
  item,
  onUpdateWorkItem,
  onRequestDelete,
}: {
  item: WorkItem;
  onUpdateWorkItem: (itemId: string, changes: UpdateChanges) => Promise<void>;
  onRequestDelete(): void;
}) {
  const statusOptions: WorkItemStatus[] = ["waiting", "ready", "in_progress", "to_check", "blocked", "done", "canceled"];
  const priorityOptions: WorkItemPriority[] = ["low", "medium", "high", "urgent"];
  return (
    <div
      style={{ padding: "6px 10px 10px 10px", background: "var(--bg-2)", borderBottom: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: 6, fontSize: 12 }}
      onClick={(event) => event.stopPropagation()}
    >
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center", fontSize: 11 }}>
        <span style={{ color: "var(--muted)" }}>{item.kind}</span>
        <span style={{ color: "var(--muted)" }}>·</span>
        <InlineSelect
          value={item.status}
          options={statusOptions}
          onChange={(value) => void onUpdateWorkItem(item.id, { status: value as WorkItemStatus })}
        />
        <span style={{ color: "var(--muted)" }}>·</span>
        <InlineSelect
          value={item.priority}
          options={priorityOptions}
          onChange={(value) => void onUpdateWorkItem(item.id, { priority: value as WorkItemPriority })}
          suffix=" priority"
        />
        <span style={{ color: "var(--muted)" }}>·</span>
        <span style={{ color: "var(--muted)" }}>by {item.created_by}</span>
        <span style={{ flex: 1 }} />
        <button onClick={onRequestDelete} style={deleteButtonStyle} title="Delete work item">Delete</button>
      </div>
      <EditableField
        key={`title-${item.id}-${item.updated_at}`}
        label="Title"
        value={item.title}
        placeholder="Title"
        multiline={false}
        onCommit={(value) => {
          const trimmed = value.trim();
          if (!trimmed || trimmed === item.title) return;
          void onUpdateWorkItem(item.id, { title: trimmed });
        }}
      />
      <EditableField
        key={`desc-${item.id}-${item.updated_at}`}
        label="Description"
        value={item.description}
        placeholder="Add a description…"
        multiline
        onCommit={(value) => {
          if (value === item.description) return;
          void onUpdateWorkItem(item.id, { description: value });
        }}
      />
      <EditableField
        key={`accept-${item.id}-${item.updated_at}`}
        label="Acceptance"
        value={item.acceptance_criteria ?? ""}
        placeholder="Acceptance criteria, one per line"
        multiline
        onCommit={(value) => {
          const next = value.length === 0 ? null : value;
          if (next === item.acceptance_criteria) return;
          void onUpdateWorkItem(item.id, { acceptanceCriteria: next });
        }}
      />
    </div>
  );
}

function EditableField({
  label,
  value,
  placeholder,
  multiline,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  multiline: boolean;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    if (draft === value) return;
    onCommit(draft);
  };

  const cancel = () => {
    setDraft(value);
    setEditing(false);
  };

  const commonProps = {
    value: draft,
    placeholder,
    onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDraft(event.target.value),
    onFocus: () => setEditing(true),
    onBlur: commit,
    onKeyDown: (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && !multiline) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      }
    },
    style: {
      ...inputStyle,
      width: "100%",
      minHeight: multiline ? 48 : undefined,
      resize: multiline ? ("vertical" as const) : undefined,
      fontFamily: "inherit",
    },
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)" }}>{label}</div>
      {multiline ? <textarea {...commonProps} /> : <input {...commonProps} />}
    </div>
  );
}

function InlineSelect({
  value,
  options,
  onChange,
  suffix,
}: {
  value: string;
  options: readonly string[];
  onChange(value: string): void;
  suffix?: string;
}) {
  return (
    <span style={{ position: "relative", display: "inline-block" }}>
      <span style={{ color: "inherit" }}>{value}{suffix ?? ""}</span>
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0,
          cursor: "pointer",
          width: "100%",
          height: "100%",
          font: "inherit",
        }}
      >
        {options.map((option) => (
          <option key={option} value={option}>{option}</option>
        ))}
      </select>
    </span>
  );
}

function ContextMenu({ menu, onDelete }: { menu: ContextMenuState; onDelete(): void }) {
  return (
    <div
      style={{
        position: "fixed",
        top: menu.y,
        left: menu.x,
        background: "var(--bg-2)",
        border: "1px solid var(--border)",
        borderRadius: 6,
        padding: 4,
        boxShadow: "0 4px 12px rgba(0,0,0,0.3)",
        zIndex: 1000,
        minWidth: 140,
      }}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <button
        onClick={onDelete}
        style={{
          display: "block",
          width: "100%",
          textAlign: "left",
          padding: "6px 10px",
          background: "transparent",
          border: "none",
          color: "inherit",
          font: "inherit",
          fontSize: 12,
          cursor: "pointer",
          borderRadius: 4,
        }}
        onMouseEnter={(event) => { event.currentTarget.style.background = "var(--bg)"; }}
        onMouseLeave={(event) => { event.currentTarget.style.background = "transparent"; }}
      >Delete</button>
    </div>
  );
}

function buildBacklogGroups(state: BacklogState | null, showCompleted: boolean): Array<{ epic: WorkItem | null; items: WorkItem[] }> {
  if (!state) return [];
  const items = showCompleted
    ? [...state.waiting, ...state.inProgress, ...state.done]
    : [...state.waiting, ...state.inProgress];
  if (items.length === 0) return [];
  items.sort((a, b) => a.sort_index - b.sort_index);
  return [{ epic: null, items }];
}

function buildGroups(batchWork: BatchWorkState | null, showCompleted: boolean): Array<{ epic: WorkItem | null; items: WorkItem[] }> {
  if (!batchWork) return [];
  const active = [...batchWork.waiting, ...batchWork.inProgress];
  const all = showCompleted ? [...active, ...batchWork.done] : active;
  const nonEpics = all.filter((item) => item.kind !== "epic");
  const epics = batchWork.epics.filter((epic) => showCompleted || epic.status !== "done");

  const epicMap = new Map<string, { epic: WorkItem | null; items: WorkItem[] }>();
  for (const epic of epics) {
    epicMap.set(epic.id, { epic, items: [] });
  }
  const rootGroup: { epic: WorkItem | null; items: WorkItem[] } = { epic: null, items: [] };

  for (const item of nonEpics) {
    const parentGroup = item.parent_id ? epicMap.get(item.parent_id) : undefined;
    if (parentGroup) {
      parentGroup.items.push(item);
    } else {
      rootGroup.items.push(item);
    }
  }

  const groups: Array<{ epic: WorkItem | null; items: WorkItem[] }> = [];
  if (rootGroup.items.length > 0) groups.push(rootGroup);
  const epicGroups = [...epicMap.values()].sort(
    (a, b) => (a.epic?.sort_index ?? 0) - (b.epic?.sort_index ?? 0),
  );
  for (const group of epicGroups) {
    group.items.sort((a, b) => a.sort_index - b.sort_index);
    groups.push(group);
  }
  rootGroup.items.sort((a, b) => a.sort_index - b.sort_index);
  return groups;
}

function statusIcon(status: WorkItemStatus): string {
  switch (status) {
    case "waiting": return "○";
    case "ready": return "◔";
    case "in_progress": return "◐";
    case "to_check": return "?";
    case "blocked": return "⊘";
    case "done": return "✓";
    case "canceled": return "✕";
  }
}

function priorityIcon(priority: WorkItemPriority): string {
  switch (priority) {
    case "urgent": return "!!";
    case "high": return "▲";
    case "medium": return "●";
    case "low": return "▽";
  }
}

function priorityStyle(priority: WorkItemPriority): CSSProperties {
  switch (priority) {
    case "urgent": return { color: "#e06c75", fontWeight: 700 };
    case "high": return { color: "#e5a06a" };
    case "medium": return { color: "var(--muted)" };
    case "low": return { color: "var(--muted)", opacity: 0.6 };
  }
}

const inputStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", font: "inherit", padding: "4px 6px", fontSize: 12,
};

const buttonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--accent)", color: "#fff", cursor: "pointer", font: "inherit", padding: "6px 10px",
};

const miniButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", cursor: "pointer", font: "inherit", padding: "3px 6px", fontSize: 11,
};

const deleteButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "#e06c75", cursor: "pointer", font: "inherit", padding: "2px 8px", fontSize: 11,
};

const bottomBarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 8px",
  borderTop: "1px solid var(--border)",
  background: "var(--bg)",
};

const bottomChipStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "3px 10px",
  border: "1px solid var(--border)",
  borderRadius: 999,
  background: "var(--bg-2)",
  color: "inherit",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
  whiteSpace: "nowrap",
};

const groupHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "6px 8px",
  background: "var(--bg-2)",
  borderTop: "1px solid var(--border)",
  borderBottom: "1px solid var(--border)",
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: 0.4,
  color: "var(--muted)",
};

function CommitPointRow({ cp }: { cp: CommitPoint }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(cp.proposed_message ?? "");
  useEffect(() => { setDraft(cp.proposed_message ?? ""); }, [cp.proposed_message]);

  return (
    <div style={commitRowStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={commitBadgeStyle(cp.status)}>{cp.status}</span>
        <select
          value={cp.mode}
          disabled={cp.status !== "pending"}
          onChange={(e) => void setCommitPointMode(cp.id, e.target.value as CommitPointMode).catch(() => {})}
          style={{ fontSize: 11, padding: "2px 4px" }}
        >
          <option value="approval">Approval</option>
          <option value="auto">Auto-commit</option>
        </select>
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          {cp.status === "rejected" ? (
            <button style={miniButtonStyle} onClick={() => void resetCommitPoint(cp.id).catch(() => {})}>Retry</button>
          ) : null}
          {cp.status !== "done" ? (
            <button style={miniButtonStyle} onClick={() => {
              void deleteCommitPoint(cp.id).catch(() => {});
            }}>Delete</button>
          ) : null}
        </span>
      </div>
      {cp.commit_sha ? (
        <div style={{ fontFamily: "ui-monospace, monospace", fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
          committed {cp.commit_sha.slice(0, 8)}
        </div>
      ) : null}
      {cp.status === "proposed" ? (
        <div style={{ marginTop: 6 }}>
          <div style={{ fontSize: 11, color: "var(--muted)", marginBottom: 4 }}>Proposed message (awaiting approval):</div>
          {editing ? (
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              rows={Math.min(6, Math.max(2, draft.split("\n").length))}
              style={commitMessageEditStyle}
            />
          ) : (
            <pre style={commitMessagePreStyle}>{cp.proposed_message}</pre>
          )}
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            {editing ? (
              <>
                <button style={miniButtonStyle} onClick={() => { void approveCommitPoint(cp.id, draft).catch(() => {}); }}>Save & approve</button>
                <button style={miniButtonStyle} onClick={() => { setEditing(false); setDraft(cp.proposed_message ?? ""); }}>Cancel</button>
              </>
            ) : (
              <>
                <button style={miniButtonStyle} onClick={() => void approveCommitPoint(cp.id).catch(() => {})}>Approve</button>
                <button style={miniButtonStyle} onClick={() => setEditing(true)}>Edit</button>
                <button style={miniButtonStyle} onClick={() => {
                  const note = window.prompt("Rejection note (sent to agent on retry):", "");
                  if (note != null) void rejectCommitPoint(cp.id, note).catch(() => {});
                }}>Reject</button>
              </>
            )}
          </div>
        </div>
      ) : null}
      {cp.status === "rejected" && cp.rejection_note ? (
        <div style={{ marginTop: 6, fontSize: 11, color: "#e06b6b" }}>Rejected: {cp.rejection_note}</div>
      ) : null}
    </div>
  );
}

const queueMarkerBarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 10px",
};

const commitDividerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 8px",
  borderTop: "1px solid transparent",
  userSelect: "none",
};

const commitDividerLineStyle: CSSProperties = {
  flex: 1,
  height: 1,
  background: "var(--border-strong)",
};

function commitDividerBadgeStyle(mode: CommitPointMode, status: CommitPoint["status"]): CSSProperties {
  const accent = status === "proposed" ? "#d97706"
    : status === "rejected" ? "#e06b6b"
    : status === "done" ? "#10b981"
    : status === "approved" ? "#0ea5e9"
    : "#8888aa";
  return {
    fontSize: 10,
    fontFamily: "ui-monospace, monospace",
    letterSpacing: 0.4,
    textTransform: "uppercase",
    padding: "2px 8px",
    borderRadius: 999,
    background: accent + "22",
    color: accent,
    border: `1px solid ${accent}55`,
    flexShrink: 0,
  };
}

const commitRowStyle: CSSProperties = {
  padding: "6px 10px",
  borderBottom: "1px solid var(--border)",
  fontSize: 12,
};

const commitMessagePreStyle: CSSProperties = {
  margin: 0,
  padding: 6,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  whiteSpace: "pre-wrap",
};

const commitMessageEditStyle: CSSProperties = {
  width: "100%",
  padding: 6,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  color: "var(--fg)",
  resize: "vertical",
};

function WaitPointRow({ wp }: { wp: WaitPoint }) {
  return (
    <div style={commitRowStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {wp.note ? <span style={{ color: "var(--muted)", fontSize: 11 }}>{wp.note}</span> : null}
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          {wp.status === "pending" ? (
            <button
              style={miniButtonStyle}
              onClick={() => {
                const next = window.prompt("Wait point note:", wp.note ?? "");
                if (next != null) void setWaitPointNote(wp.id, next || null).catch(() => {});
              }}
            >
              Edit
            </button>
          ) : null}
          <button
            style={miniButtonStyle}
            onClick={() => {
              if (window.confirm("Delete this wait point?")) void deleteWaitPoint(wp.id).catch(() => {});
            }}
          >
            Delete
          </button>
        </span>
      </div>
      {wp.status === "triggered" ? (
        <div style={{ marginTop: 4, fontSize: 11, color: "#d97706" }}>
          Agent stopped here. Prompt the agent directly to resume.
        </div>
      ) : null}
    </div>
  );
}

function waitDividerBadgeStyle(status: WaitPoint["status"]): CSSProperties {
  const accent = status === "triggered" ? "#d97706" : "#8888aa";
  return {
    fontSize: 10,
    fontFamily: "ui-monospace, monospace",
    letterSpacing: 0.4,
    textTransform: "uppercase",
    padding: "2px 8px",
    borderRadius: 999,
    background: accent + "22",
    color: accent,
    border: `1px solid ${accent}55`,
    flexShrink: 0,
  };
}

function commitBadgeStyle(status: CommitPoint["status"]): CSSProperties {
  const colors: Record<CommitPoint["status"], string> = {
    pending: "#6b7280",
    proposed: "#d97706",
    approved: "#0ea5e9",
    done: "#10b981",
    rejected: "#e06b6b",
  };
  return {
    fontSize: 10,
    fontFamily: "ui-monospace, monospace",
    padding: "1px 6px",
    borderRadius: 8,
    background: colors[status] + "22",
    color: colors[status],
    border: `1px solid ${colors[status]}55`,
    textTransform: "uppercase",
    letterSpacing: 0.4,
  };
}
