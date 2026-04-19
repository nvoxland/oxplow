import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
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
  classifyWorkItem,
  inputStyle,
  miniButtonStyle,
} from "./plan-utils.js";

const STATUS_RANK: Record<string, number> = { inProgress: 0, toDo: 1, humanCheck: 2, done: 3 };
function statusOrderRank(status: WorkItemStatus): number {
  return STATUS_RANK[classifyWorkItem(status)] ?? 0;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return type === "text" || type === "search" || type === "email" || type === "url" || type === "password" || type === "" || type === "tel";
  }
  return false;
}

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
  const [priority, setPriority] = useState<WorkItemPriority>("medium");
  const [parentId, setParentId] = useState<string>("");
  const [createOpen, setCreateOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [mode, setMode] = useState<"batch" | "backlog">("batch");
  const [backlogChipDragOver, setBacklogChipDragOver] = useState(false);
  const [commitPoints, setCommitPoints] = useState<CommitPoint[]>([]);
  const [waitPoints, setWaitPoints] = useState<WaitPoint[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [kbPicker, setKbPicker] = useState<{ kind: "status" | "priority"; itemId: string } | null>(null);
  const paneRef = useRef<HTMLDivElement | null>(null);

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
    if (mode === "backlog") return buildBacklogGroups(backlog);
    return buildGroups(batchWork);
  }, [mode, batchWork, backlog]);

  // Flat top-to-bottom list of work-item ids in the order they appear on
  // screen. Rebuilt whenever the groups change so ↑/↓ navigation stays in
  // sync with the section split in WorkGroupList (In progress → To do →
  // Human check → Done). Commit/wait-point rows are deliberately excluded:
  // they're not "selectable work" in the keyboard sense.
  const navigableIds = useMemo(() => {
    const ids: string[] = [];
    for (const group of groups) {
      const sorted = group.items.slice().sort((a, b) => {
        const byStatus = statusOrderRank(a.status) - statusOrderRank(b.status);
        if (byStatus !== 0) return byStatus;
        return a.sort_index - b.sort_index;
      });
      for (const item of sorted) ids.push(item.id);
    }
    return ids;
  }, [groups]);

  useEffect(() => {
    if (!selectedId) return;
    if (!navigableIds.includes(selectedId)) setSelectedId(null);
  }, [navigableIds, selectedId]);

  const selectedItem: WorkItem | null = useMemo(() => {
    if (!selectedId) return null;
    for (const group of groups) {
      const hit = group.items.find((item) => item.id === selectedId);
      if (hit) return hit;
    }
    return null;
  }, [groups, selectedId]);

  const activeCreate = mode === "backlog" ? onCreateBacklogItem : onCreateWorkItem;
  const activeUpdate = mode === "backlog" ? onUpdateBacklogItem : onUpdateWorkItem;
  const activeDelete = mode === "backlog" ? onDeleteBacklogItem : onDeleteWorkItem;
  const activeReorder = mode === "backlog" ? onReorderBacklog : onReorderWorkItems;
  const currentScopeBatchId = mode === "backlog" ? null : batch?.id ?? null;

  useEffect(() => {
    // Listen at the pane level (not window) so the Agent pane / editor don't
    // steal the shortcut when they're focused, AND so the Plan pane can
    // keep a visible "selected" row without grabbing focus away from the
    // rest of the app. We still honour editable-target suppression for
    // typing comfort.
    const el = paneRef.current;
    if (!el) return;
    const handler = (event: KeyboardEvent) => {
      if (kbPicker) return; // modal owns keyboard
      if (isEditableTarget(event.target)) return;
      const key = event.key;
      if ((key === "ArrowDown" || key === "ArrowUp") && event.shiftKey) {
        // Shift+↑/↓ reorders the selected item within its own status
        // section. Crossing a section boundary is a no-op — for that,
        // the user drags, which intentionally changes status as a side
        // effect. Reordering is section-local so the keyboard path
        // doesn't silently promote/demote.
        if (!selectedId) return;
        const selected = groups
          .flatMap((g) => g.items)
          .find((item) => item.id === selectedId);
        if (!selected) return;
        const selSection = classifyWorkItem(selected.status);
        const sectionIds = navigableIds.filter((id) => {
          const item = groups.flatMap((g) => g.items).find((i) => i.id === id);
          return item ? classifyWorkItem(item.status) === selSection : false;
        });
        const posInSection = sectionIds.indexOf(selectedId);
        const neighborPosInSection = key === "ArrowDown" ? posInSection + 1 : posInSection - 1;
        if (neighborPosInSection < 0 || neighborPosInSection >= sectionIds.length) return;
        event.preventDefault();
        const neighborId = sectionIds[neighborPosInSection]!;
        const nextOrder = navigableIds.slice();
        const i = nextOrder.indexOf(selectedId);
        const j = nextOrder.indexOf(neighborId);
        if (i < 0 || j < 0) return;
        [nextOrder[i], nextOrder[j]] = [nextOrder[j]!, nextOrder[i]!];
        void runWithError("Reorder work items", activeReorder(nextOrder));
        return;
      }
      if (key === "ArrowDown" || key === "ArrowUp") {
        if (navigableIds.length === 0) return;
        event.preventDefault();
        const idx = selectedId ? navigableIds.indexOf(selectedId) : -1;
        const next = key === "ArrowDown"
          ? Math.min(idx + 1, navigableIds.length - 1)
          : idx <= 0 ? 0 : idx - 1;
        setSelectedId(navigableIds[next] ?? null);
      } else if (key === "Enter" && selectedId) {
        event.preventDefault();
        setExpandedId((prev) => (prev === selectedId ? null : selectedId));
      } else if ((key === "s" || key === "S") && selectedId) {
        event.preventDefault();
        setKbPicker({ kind: "status", itemId: selectedId });
      } else if ((key === "p" || key === "P") && selectedId) {
        event.preventDefault();
        setKbPicker({ kind: "priority", itemId: selectedId });
      }
    };
    el.addEventListener("keydown", handler);
    return () => el.removeEventListener("keydown", handler);
  }, [navigableIds, selectedId, kbPicker, groups, activeReorder]);

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
    <div
      ref={paneRef}
      tabIndex={0}
      data-testid="plan-pane"
      onClick={() => paneRef.current?.focus()}
      style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden", outline: "none" }}
    >
      <div style={{ padding: 8, borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <button type="button" data-testid="plan-new-work-item" onClick={() => setCreateOpen(true)} style={{ ...miniButtonStyle, padding: "4px 10px" }}>
            + New work item
          </button>
          <span style={{ color: "var(--muted)", fontSize: 11 }}>
            {mode === "backlog" ? "Backlog" : ""}
          </span>
        </div>
      </div>
      {createOpen ? (
        <NewWorkItemModal
          title={title}
          setTitle={setTitle}
          description={description}
          setDescription={setDescription}
          acceptance={acceptance}
          setAcceptance={setAcceptance}
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
              kind: "task",
              title: nextTitle,
              description,
              acceptanceCriteria: acceptance || null,
              parentId: mode === "batch" ? (parentId || undefined) : undefined,
              priority,
              status: "waiting",
            });
            setTitle(""); setDescription(""); setAcceptance("");
            if (!andAnother) {
              setParentId(""); setPriority("medium");
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
          groups.map((group) => {
            const isRootBatch = group.epic === null && mode === "batch";
            const canAddPoints = isRootBatch && !!batchWork && batchWork.items.length > 0;
            const addPointsSlot = isRootBatch && batch ? (
              <div style={queueMarkerBarStyle} data-testid="plan-add-points-bar">
                <button type="button"
                  data-testid="plan-add-commit-point"
                  onClick={() => {
                    if (!streamId || !batchId) return;
                    runWithError("Add commit point", createCommitPoint(streamId, batchId, "approval"));
                  }}
                  disabled={!canAddPoints}
                  style={{
                    ...miniButtonStyle,
                    opacity: canAddPoints ? 1 : 0.5,
                    cursor: canAddPoints ? "pointer" : "not-allowed",
                  }}
                  title={
                    canAddPoints
                      ? "Append a commit point to the To-do queue (drag to reposition; click to switch to auto-commit)"
                      : "Add a work item first — a commit point can't be the very first queue entry"
                  }
                >
                  + Commit when done
                </button>
                <button type="button"
                  data-testid="plan-add-wait-point"
                  onClick={() => {
                    if (!streamId || !batchId) return;
                    runWithError("Add wait point", createWaitPoint(streamId, batchId, null));
                  }}
                  disabled={!canAddPoints}
                  style={{
                    ...miniButtonStyle,
                    opacity: canAddPoints ? 1 : 0.5,
                    cursor: canAddPoints ? "pointer" : "not-allowed",
                  }}
                  title={
                    canAddPoints
                      ? "Append a wait point to the To-do queue (drag to reposition; click the divider to add a note)"
                      : "Add a work item first — a wait point can't be the very first queue entry"
                  }
                >
                  + Wait here
                </button>
              </div>
            ) : null;
            return (
              <WorkGroupList
                key={group.epic?.id ?? "__root__"}
                group={group}
                scopeBatchId={currentScopeBatchId}
                expandedId={expandedId}
                onToggleExpand={(id) => setExpandedId((prev) => (prev === id ? null : id))}
                onUpdateWorkItem={activeUpdate}
                onReorderWorkItems={activeReorder}
                // Commit/wait points only belong to the root (non-epic) group —
                // they divide the top-level To-do queue, not items scoped inside
                // an epic.
                commitPoints={isRootBatch ? commitPoints : []}
                waitPoints={isRootBatch ? waitPoints : []}
                onReorderMixed={isRootBatch && streamId && batchId
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
                addPointsSlot={addPointsSlot}
                selectedId={selectedId}
                onSelect={setSelectedId}
              />
            );
          })
        )}
      </div>
      <div style={bottomBarStyle}>
        <button type="button"
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
      {kbPicker && selectedItem ? (
        <KeyboardValuePicker
          kind={kbPicker.kind}
          item={selectedItem}
          onPick={(value) => {
            if (kbPicker.kind === "status") {
              void activeUpdate(selectedItem.id, { status: value as WorkItemStatus });
            } else {
              void activeUpdate(selectedItem.id, { priority: value as WorkItemPriority });
            }
            setKbPicker(null);
            paneRef.current?.focus();
          }}
          onClose={() => { setKbPicker(null); paneRef.current?.focus(); }}
        />
      ) : null}
    </div>
  );
}

const KB_STATUS_OPTIONS: WorkItemStatus[] = [
  "waiting", "ready", "in_progress", "human_check", "blocked", "done", "canceled",
];
const KB_PRIORITY_OPTIONS: WorkItemPriority[] = ["urgent", "high", "medium", "low"];

/**
 * Small centered picker opened by the keyboard shortcuts `S` / `P` when a
 * work-item row is selected. Autofocuses, ↑/↓ navigate options, Enter
 * commits, Escape cancels. Mouse click on a row also commits. Kept in-line
 * in this file rather than extracted because nothing else uses it.
 */
function KeyboardValuePicker({
  kind,
  item,
  onPick,
  onClose,
}: {
  kind: "status" | "priority";
  item: WorkItem;
  onPick(value: string): void;
  onClose(): void;
}) {
  const options: readonly string[] = kind === "status" ? KB_STATUS_OPTIONS : KB_PRIORITY_OPTIONS;
  const current = kind === "status" ? item.status : item.priority;
  const initialIdx = Math.max(0, options.indexOf(current as string));
  const [idx, setIdx] = useState(initialIdx);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setIdx((prev) => Math.min(prev + 1, options.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setIdx((prev) => Math.max(prev - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        onPick(options[idx]!);
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [idx, options, onPick, onClose]);

  return (
    <div style={kbPickerOverlayStyle} onClick={onClose}>
      <div style={kbPickerStyle} onClick={(event) => event.stopPropagation()}>
        <div style={{ padding: "8px 12px", borderBottom: "1px solid var(--border)", fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.6 }}>
          {kind === "status" ? "Set status" : "Set priority"}
          <span style={{ float: "right", fontFamily: "ui-monospace, monospace" }}>↑↓ · Enter · Esc</span>
        </div>
        <div style={{ padding: 4 }}>
          {options.map((option, i) => {
            const active = i === idx;
            return (
              <div
                key={option}
                onMouseEnter={() => setIdx(i)}
                onClick={() => onPick(option)}
                style={{
                  padding: "5px 10px",
                  borderRadius: 4,
                  fontSize: 13,
                  cursor: "pointer",
                  background: active ? "var(--accent)" : "transparent",
                  color: active ? "#fff" : "var(--fg)",
                }}
              >
                {option}
                {option === current ? <span style={{ marginLeft: 8, opacity: 0.7 }}>· current</span> : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

const kbPickerOverlayStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.4)",
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "center",
  paddingTop: "20vh",
  zIndex: 3000,
};

const kbPickerStyle: CSSProperties = {
  background: "var(--bg-1)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  width: "min(280px, 90vw)",
  boxShadow: "0 12px 32px rgba(0,0,0,0.6)",
};

function NewWorkItemModal({
  title,
  setTitle,
  description,
  setDescription,
  acceptance,
  setAcceptance,
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
        <label htmlFor="work-item-title" style={srOnlyStyle}>Title</label>
        <input
          autoFocus
          id="work-item-title"
          data-testid="work-item-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Title (required)"
          style={inputStyle}
        />
        <label htmlFor="work-item-priority" style={srOnlyStyle}>Priority</label>
        <select id="work-item-priority" data-testid="work-item-priority" value={priority} onChange={(e) => setPriority(e.target.value as WorkItemPriority)} style={inputStyle}>
          <option value="low">Low</option>
          <option value="medium">Medium</option>
          <option value="high">High</option>
          <option value="urgent">Urgent</option>
        </select>
        {showParent ? (
        <>
        <label htmlFor="work-item-parent" style={srOnlyStyle}>Parent epic</label>
        <select id="work-item-parent" data-testid="work-item-parent" value={parentId} onChange={(e) => setParentId(e.target.value)} style={inputStyle}>
          <option value="">No parent epic</option>
          {epics.map((epic) => (<option key={epic.id} value={epic.id}>{epic.title}</option>))}
        </select>
        </>
        ) : null}
        <label htmlFor="work-item-description" style={srOnlyStyle}>Description</label>
        <textarea
          id="work-item-description"
          data-testid="work-item-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Description"
          style={{ ...inputStyle, minHeight: 64, resize: "vertical" }}
        />
        <label htmlFor="work-item-acceptance" style={srOnlyStyle}>Acceptance criteria</label>
        <textarea
          id="work-item-acceptance"
          data-testid="work-item-acceptance"
          value={acceptance}
          onChange={(e) => setAcceptance(e.target.value)}
          placeholder="Acceptance criteria, one per line"
          style={{ ...inputStyle, minHeight: 64, resize: "vertical" }}
        />
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginTop: 4 }}>
          <button type="button" data-testid="work-item-cancel" onClick={onClose} style={miniButtonStyle}>Cancel</button>
          <button
            type="button"
            data-testid="work-item-save-another"
            disabled={!canSubmit}
            onClick={() => { if (canSubmit) void onSubmit(true); }}
            style={{ ...miniButtonStyle, padding: "6px 10px", opacity: canSubmit ? 1 : 0.5 }}
          >Save and Another</button>
          <button
            type="submit"
            data-testid="work-item-save"
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
      data-testid="plan-context-menu"
    >
      <button type="button"
        onClick={onDelete}
        data-testid="plan-context-menu-delete"
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

const srOnlyStyle: CSSProperties = {
  position: "absolute",
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: "hidden",
  clip: "rect(0,0,0,0)",
  whiteSpace: "nowrap",
  border: 0,
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
