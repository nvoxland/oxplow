import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import type {
  BacklogState,
  Batch,
  BatchWorkState,
  CommitPoint,
  WaitPoint,
  WorkItem,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "../../api.js";
import {
  createCommitPoint,
  createWaitPoint,
  listCommitPoints,
  listWaitPoints,
  reorderBatchQueue,
  subscribeNewdeEvents,
} from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { reportUiError, runWithError } from "../../ui-error.js";
import { WorkGroupList } from "./WorkGroupList.js";
import type { WorkItemDetailChanges } from "./WorkItemDetail.js";
import {
  buildBacklogGroups,
  buildGroups,
  inputStyle,
  miniButtonStyle,
} from "./plan-utils.js";

interface CreateInput {
  kind: WorkItemKind;
  title: string;
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
  onUpdateWorkItem(itemId: string, changes: WorkItemDetailChanges): Promise<void>;
  onDeleteWorkItem(itemId: string): Promise<void>;
  onReorderWorkItems(orderedItemIds: string[]): Promise<void>;
  onCreateBacklogItem(input: CreateInput): Promise<void>;
  onUpdateBacklogItem(itemId: string, changes: WorkItemDetailChanges): Promise<void>;
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
      .catch((err) => reportUiError("Load commit points", err));
    const refreshWaits = () => void listWaitPoints(batchId)
      .then((points) => { if (!cancelled) setWaitPoints(points); })
      .catch((err) => reportUiError("Load wait points", err));
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
                ? (entries) => runWithError("Reorder queue", reorderBatchQueue(streamId, batchId, entries))
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
                runWithError("Add commit point", createCommitPoint(streamId, batchId, "approval"));
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
                runWithError("Add wait point", createWaitPoint(streamId, batchId, null));
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
            style={{ ...primaryButtonStyle, opacity: canSubmit ? 1 : 0.5 }}
          >Save</button>
        </div>
      </form>
    </div>
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

const primaryButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--accent)", color: "#fff", cursor: "pointer", font: "inherit", padding: "6px 10px",
};

const queueMarkerBarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 10px",
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
