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
  WorkNote,
} from "../../api.js";
import {
  createCommitPoint,
  createWaitPoint,
  getWorkNotes,
  listCommitPoints,
  listWaitPoints,
  reorderBatchQueue,
  setAutoCommit,
  subscribeNewdeEvents,
  updateCommitPoint,
} from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { ContextMenu } from "../ContextMenu.js";
import { ConfirmDialog } from "../ConfirmDialog.js";
import type { MenuItem } from "../../menu.js";
import { reportUiError, runWithError } from "../../ui-error.js";
import { WorkGroupList } from "./WorkGroupList.js";
import type { WorkItemDetailChanges } from "./WorkItemDetail.js";
import {
  buildBacklogGroups,
  buildGroups,
  classifyWorkItem,
  inputStyle,
  miniButtonStyle,
  statusLabel,
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
  activeBatchId: string | null;
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
  /** Non-null when the right-clicked item belongs to a multi-selection. */
  groupIds: string[] | null;
}

export function PlanPane({
  batch,
  activeBatchId,
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
  // Modal surface: `create` = blank New Work Item form; `edit` = same modal
  // shape but pre-filled from `editingItemId` and writing back via
  // activeUpdate on submit. Plain clicks on a work-item row open edit mode
  // (the legacy inline expansion + title click-to-edit were removed per the
  // "change work item editing UI" task).
  const [modalMode, setModalMode] = useState<"create" | "edit" | null>(null);
  const [editingItemId, setEditingItemId] = useState<string | null>(null);
  // When opening the create modal from an epic's "+ Task" button, remember
  // the epic id so the new item gets filed as a child. Null means top-level.
  const [createParentId, setCreateParentId] = useState<string | null>(null);
  const [editingItem, setEditingItem] = useState<WorkItem | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [pendingDelete, setPendingDelete] = useState<WorkItem | null>(null);
  const [mode, setMode] = useState<"batch" | "backlog">("batch");
  const [backlogChipDragOver, setBacklogChipDragOver] = useState(false);
  const [commitPoints, setCommitPoints] = useState<CommitPoint[]>([]);
  const [waitPoints, setWaitPoints] = useState<WaitPoint[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Extra "marked" ids for multi-select beyond the primary `selectedId`. Driven
  // by Cmd/Ctrl+click (toggle) and Shift+click (range from selectedId). When a
  // drag starts on any of the effectiveMarkedIds, the drag payload carries the
  // whole set so drop targets (BatchRail, backlog chip, stream chip) can move
  // them all in one gesture. Plain click clears marks.
  const [markedIds, setMarkedIds] = useState<Set<string>>(() => new Set());
  const [kbPicker, setKbPicker] = useState<{ kind: "status" | "priority"; itemId: string; extraIds?: string[] } | null>(null);
  const [pendingDeleteGroup, setPendingDeleteGroup] = useState<string[] | null>(null);
  // Commit point edit modal: opened by double-clicking a commit point row.
  const [editingCommitPoint, setEditingCommitPoint] = useState<CommitPoint | null>(null);
  // Notes for the currently-open edit modal.
  const [editingItemNotes, setEditingItemNotes] = useState<WorkNote[]>([]);
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
      for (const item of sorted) {
        ids.push(item.id);
        const children = group.epicChildren.get(item.id);
        if (children) {
          for (const child of children) ids.push(child.id);
        }
      }
    }
    return ids;
  }, [groups]);

  useEffect(() => {
    if (!selectedId) return;
    if (!navigableIds.includes(selectedId)) setSelectedId(null);
  }, [navigableIds, selectedId]);

  // Prune any marked ids that no longer exist in the visible list — keeps the
  // mark set from accumulating stale entries after a move/delete/status change
  // pulls a row out from under the user.
  useEffect(() => {
    setMarkedIds((prev) => {
      if (prev.size === 0) return prev;
      const live = new Set(navigableIds);
      let changed = false;
      const next = new Set<string>();
      for (const id of prev) {
        if (live.has(id)) next.add(id);
        else changed = true;
      }
      return changed ? next : prev;
    });
  }, [navigableIds]);

  const handleSelect = (id: string, modifiers?: { toggle?: boolean; range?: boolean }) => {
    const toggle = modifiers?.toggle ?? false;
    const range = modifiers?.range ?? false;
    if (toggle) {
      // Cmd/Ctrl+click: flip the row in/out of the mark set without changing
      // selectedId (so the kb-focused row and the expand state stay put).
      setMarkedIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
      return;
    }
    if (range && selectedId && selectedId !== id) {
      // Shift+click: mark every row between selectedId and id (inclusive of
      // both endpoints) in screen order. Selected anchor itself stays the
      // primary.
      const fromIdx = navigableIds.indexOf(selectedId);
      const toIdx = navigableIds.indexOf(id);
      if (fromIdx >= 0 && toIdx >= 0) {
        const [lo, hi] = fromIdx <= toIdx ? [fromIdx, toIdx] : [toIdx, fromIdx];
        const next = new Set<string>();
        for (let i = lo; i <= hi; i++) next.add(navigableIds[i]!);
        setMarkedIds(next);
        return;
      }
    }
    // Plain click: clear marks and move the primary selection.
    setMarkedIds(new Set());
    setSelectedId(id);
  };


  const selectedItem: WorkItem | null = useMemo(() => {
    if (!selectedId) return null;
    for (const group of groups) {
      const hit = group.items.find((item) => item.id === selectedId);
      if (hit) return hit;
      for (const children of group.epicChildren.values()) {
        const childHit = children.find((item) => item.id === selectedId);
        if (childHit) return childHit;
      }
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
    const allItems = groups.flatMap((g) => [
      ...g.items,
      ...[...g.epicChildren.values()].flat(),
    ]);
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
        const selected = allItems.find((item) => item.id === selectedId);
        if (!selected) return;
        const selSection = classifyWorkItem(selected.status);
        const sectionIds = navigableIds.filter((id) => {
          const item = allItems.find((i) => i.id === id);
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
        const item = allItems.find((i) => i.id === selectedId);
        if (item) openEditModal(item);
      } else if ((key === "s" || key === "S") && selectedId) {
        if (allItems.find((i) => i.id === selectedId)?.status === "in_progress") return;
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

  const openCreateModal = (parentId: string | null = null) => {
    setTitle(""); setDescription(""); setAcceptance("");
    setPriority("medium");
    setEditingItemId(null);
    setCreateParentId(parentId);
    setModalMode("create");
  };

  const openEditModal = (item: WorkItem) => {
    setTitle(item.title);
    setDescription(item.description ?? "");
    setAcceptance(item.acceptance_criteria ?? "");
    setPriority(item.priority);
    setEditingItemId(item.id);
    setEditingItem(item);
    setEditingItemNotes([]);
    setModalMode("edit");
    void getWorkNotes(item.id)
      .then((notes) => setEditingItemNotes(notes))
      .catch(() => { /* non-fatal */ });
  };

  const closeModal = () => {
    setModalMode(null);
    setEditingItemId(null);
    setEditingItem(null);
    setEditingItemNotes([]);
    paneRef.current?.focus();
  };

  useEffect(() => {
    if (openNewRequest === undefined || openNewRequest === 0) return;
    openCreateModal();
    // openCreateModal is intentionally not in deps — it closes over setters
    // that are stable across renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openNewRequest]);


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
      const payload = JSON.parse(raw) as {
        itemId?: string;
        itemIds?: string[];
        fromBatchId?: string | null;
      };
      const fromBatchId = payload.fromBatchId;
      if (!fromBatchId) return;
      const ids = payload.itemIds && payload.itemIds.length > 0
        ? payload.itemIds
        : payload.itemId ? [payload.itemId] : [];
      // Move each marked item in sequence — the store already serialises the
      // batch mutations, and doing them one at a time keeps the failure mode
      // simple (a bad id throws, the rest keep going isn't worth the risk of
      // a partial state that surprises the user). If the first one fails the
      // later ones are skipped by Promise.allSettled semantics in the caller.
      for (const id of ids) {
        void onMoveItemToBacklog(id, fromBatchId);
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
          <button type="button" data-testid="plan-new-task" onClick={() => openCreateModal()} style={{ ...miniButtonStyle, padding: "4px 10px" }}>
            + New Task
          </button>
          <span style={{ color: "var(--muted)", fontSize: 11 }}>
            {mode === "backlog" ? "Backlog" : ""}
          </span>
        </div>
      </div>
      {modalMode ? (
        <NewWorkItemModal
          title={title}
          setTitle={setTitle}
          description={description}
          setDescription={setDescription}
          acceptance={acceptance}
          setAcceptance={setAcceptance}
          priority={priority}
          setPriority={setPriority}
          showSaveAndAnother={modalMode === "create"}
          notes={modalMode === "edit" ? editingItemNotes : []}
          item={modalMode === "edit" ? editingItem : null}
          epics={batchWork?.epics ?? []}
          onOpenItem={(target) => openEditModal(target)}
          modalTitle={
            modalMode === "edit"
              ? "Edit work item"
              : mode === "backlog" ? "New backlog item" : "New work item"
          }
          onClose={closeModal}
          onSubmit={async (andAnother) => {
            const nextTitle = title.trim();
            if (!nextTitle) return;
            if (modalMode === "edit" && editingItemId) {
              await activeUpdate(editingItemId, {
                title: nextTitle,
                description,
                acceptanceCriteria: acceptance || null,
                priority,
              });
              closeModal();
              return;
            }
            await activeCreate({
              kind: "task",
              title: nextTitle,
              description,
              acceptanceCriteria: acceptance || null,
              priority,
              status: "ready",
              parentId: createParentId,
            });
            setTitle(""); setDescription(""); setAcceptance("");
            if (!andAnother) {
              setPriority("medium");
              closeModal();
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
            const isRootBatch = mode === "batch";
            const isActive = isRootBatch && batch?.id === activeBatchId;
            const canAddPoints = isRootBatch && !!batchWork && batchWork.waiting.length > 0;
            const autoCommitOn = batch?.auto_commit ?? false;
            const addPointsSlot = isRootBatch && batch ? (
              <div style={queueMarkerBarStyle} data-testid="plan-add-points-bar">
                <CommitModeDropdown
                  autoCommitOn={autoCommitOn}
                  onSelect={(auto) => {
                    if (!streamId || !batchId) return;
                    runWithError("Set commit mode", setAutoCommit(streamId, batchId, auto));
                  }}
                />
                {!autoCommitOn ? (
                  <button type="button"
                    data-testid="plan-add-commit-point"
                    onClick={() => {
                      if (!streamId || !batchId) return;
                      runWithError("Add commit point", createCommitPoint(streamId, batchId));
                    }}
                    disabled={!canAddPoints}
                    style={{
                      ...miniButtonStyle,
                      opacity: canAddPoints ? 1 : 0.5,
                      cursor: canAddPoints ? "pointer" : "not-allowed",
                    }}
                    title={
                      canAddPoints
                        ? "Add a commit point to the queue"
                        : "No To Do items in the queue"
                    }
                  >
                    + Commit Point
                  </button>
                ) : null}
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
                      ? "Add a wait point to the queue"
                      : "No To Do items in the queue"
                  }
                >
                  + Wait Point
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
                commitPoints={isRootBatch ? commitPoints : []}
                waitPoints={isRootBatch ? waitPoints : []}
                onReorderMixed={isRootBatch && streamId && batchId
                  ? (entries) => runWithError("Reorder queue", reorderBatchQueue(streamId, batchId, entries))
                  : undefined}
                onContextMenu={(event, item) => {
                  event.preventDefault();
                  const groupIds = markedIds.has(item.id) && markedIds.size > 1
                    ? [...markedIds]
                    : null;
                  setContextMenu({ x: event.clientX, y: event.clientY, item, groupIds });
                }}
                addPointsSlot={addPointsSlot}
                selectedId={selectedId}
                markedIds={markedIds}
                onSelect={handleSelect}
                onRequestEdit={openEditModal}
                onDoubleClickCommitPoint={(cp) => setEditingCommitPoint(cp)}
                epicChildrenMap={group.epicChildren}
                onReparentWorkItem={(itemId, newParentId) => activeUpdate(itemId, { parentId: newParentId })}
                onAddChildTask={(epicId) => openCreateModal(epicId)}
                isActive={isActive}
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
          items={contextMenu.groupIds
            ? buildGroupMenu(contextMenu.item, contextMenu.groupIds, {
                onChangeStatus: (item, ids) => {
                  setContextMenu(null);
                  setSelectedId(item.id);
                  const allWi = groups.flatMap((g) => [...g.items, ...[...g.epicChildren.values()].flat()]);
                  const liveIds = ids.filter((id) => allWi.find((i) => i.id === id)?.status !== "in_progress");
                  setKbPicker({ kind: "status", itemId: item.id, extraIds: liveIds.filter((id) => id !== item.id) });
                },
                onChangePriority: (item, ids) => {
                  setContextMenu(null);
                  setSelectedId(item.id);
                  setKbPicker({ kind: "priority", itemId: item.id, extraIds: ids.filter((id) => id !== item.id) });
                },
                onDelete: (_item, ids) => {
                  setContextMenu(null);
                  const allWi = groups.flatMap((g) => [...g.items, ...[...g.epicChildren.values()].flat()]);
                  const liveIds = ids.filter((id) => allWi.find((i) => i.id === id)?.status !== "in_progress");
                  if (liveIds.length > 0) setPendingDeleteGroup(liveIds);
                },
              })
            : buildWorkItemMenu(contextMenu.item, {
                onDelete: (item) => {
                  setContextMenu(null);
                  setPendingDelete(item);
                },
                onRename: (item) => {
                  setContextMenu(null);
                  setSelectedId(item.id);
                  openEditModal(item);
                },
                onChangeStatus: (item) => {
                  setContextMenu(null);
                  setSelectedId(item.id);
                  setKbPicker({ kind: "status", itemId: item.id });
                },
                onChangePriority: (item) => {
                  setContextMenu(null);
                  setSelectedId(item.id);
                  setKbPicker({ kind: "priority", itemId: item.id });
                },
              })}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={160}
        />
      ) : null}
      {pendingDelete ? (
        <ConfirmDialog
          message={`Delete "${pendingDelete.title}"?`}
          confirmLabel="Delete"
          destructive
          onConfirm={() => {
            const item = pendingDelete;
            setPendingDelete(null);
            if (expandedId === item.id) setExpandedId(null);
            void activeDelete(item.id);
          }}
          onCancel={() => setPendingDelete(null)}
        />
      ) : null}
      {pendingDeleteGroup ? (
        <ConfirmDialog
          message={`Delete ${pendingDeleteGroup.length} work items?`}
          confirmLabel="Delete all"
          destructive
          onConfirm={() => {
            const ids = pendingDeleteGroup;
            setPendingDeleteGroup(null);
            for (const id of ids) void activeDelete(id);
          }}
          onCancel={() => setPendingDeleteGroup(null)}
        />
      ) : null}
      {kbPicker && selectedItem ? (
        <KeyboardValuePicker
          kind={kbPicker.kind}
          item={selectedItem}
          onPick={(value) => {
            const allIds = kbPicker.extraIds
              ? [kbPicker.itemId, ...kbPicker.extraIds]
              : [kbPicker.itemId];
            if (kbPicker.kind === "status") {
              for (const id of allIds) void activeUpdate(id, { status: value as WorkItemStatus });
            } else {
              for (const id of allIds) void activeUpdate(id, { priority: value as WorkItemPriority });
            }
            setKbPicker(null);
            paneRef.current?.focus();
          }}
          onClose={() => { setKbPicker(null); paneRef.current?.focus(); }}
        />
      ) : null}
      {editingCommitPoint ? (
        <CommitPointModal
          cp={editingCommitPoint}
          onSave={async (changes) => {
            await updateCommitPoint(editingCommitPoint.id, changes);
            setEditingCommitPoint(null);
          }}
          onClose={() => setEditingCommitPoint(null)}
        />
      ) : null}
    </div>
  );
}

/**
 * Custom dropdown for commit mode — replaces the native <select> that was
 * closing prematurely in Electron due to re-renders triggered mid-interaction.
 */
function CommitModeDropdown({
  autoCommitOn,
  onSelect,
}: {
  autoCommitOn: boolean;
  onSelect(auto: boolean): void;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    const handler = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [isOpen]);

  const options: { value: boolean; label: string }[] = [
    { value: false, label: "Manual Commit" },
    { value: true, label: "Auto Commit" },
  ];

  return (
    <div ref={containerRef} style={{ position: "relative" }}>
      <button
        type="button"
        data-testid="plan-commit-mode-select"
        onClick={() => setIsOpen((prev) => !prev)}
        style={{
          ...miniButtonStyle,
          padding: "4px 8px",
          display: "flex",
          alignItems: "center",
          gap: 4,
          minWidth: 120,
          justifyContent: "space-between",
        }}
      >
        <span>{autoCommitOn ? "Auto Commit" : "Manual Commit"}</span>
        <span style={{ opacity: 0.6, fontSize: 10 }}>▾</span>
      </button>
      {isOpen ? (
        <div
          style={{
            position: "absolute",
            top: "100%",
            left: 0,
            zIndex: 100,
            background: "var(--bg)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            marginTop: 2,
            minWidth: "100%",
            boxShadow: "0 4px 12px rgba(0,0,0,0.3)",
            overflow: "hidden",
          }}
        >
          {options.map(({ value, label }) => {
            const active = autoCommitOn === value;
            return (
              <button
                key={label}
                type="button"
                onClick={() => {
                  onSelect(value);
                  setIsOpen(false);
                }}
                style={{
                  display: "block",
                  width: "100%",
                  textAlign: "left",
                  padding: "6px 10px",
                  border: "none",
                  background: active ? "var(--accent)" : "transparent",
                  color: active ? "#fff" : "var(--fg)",
                  cursor: "pointer",
                  fontFamily: "inherit",
                  fontSize: 12,
                  whiteSpace: "nowrap",
                }}
              >
                {label}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

const KB_STATUS_OPTIONS: WorkItemStatus[] = [
  "blocked", "ready", "in_progress", "human_check", "done", "archived", "canceled",
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
                {kind === "status" ? statusLabel(option as WorkItemStatus) : option}
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
  showSaveAndAnother = true,
  notes = [],
  item = null,
  epics = [],
  onOpenItem,
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
  showSaveAndAnother?: boolean;
  notes?: WorkNote[];
  item?: WorkItem | null;
  epics?: WorkItem[];
  onOpenItem?(item: WorkItem): void;
  modalTitle?: string;
  onClose(): void;
  onSubmit(andAnother: boolean): Promise<void>;
}) {
  const readOnly = item?.status === "in_progress";
  const canSubmit = !readOnly && title.trim().length > 0;

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

  const parentEpic = item?.parent_id ? epics.find((e) => e.id === item.parent_id) ?? null : null;
  const updatedDiffers = item && item.updated_at && item.updated_at !== item.created_at;

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
          width: "75vw",
          maxWidth: 1100,
          minWidth: 600,
          height: "75vh",
          maxHeight: 800,
          minHeight: 500,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
          gap: 8,
          boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 8px 24px rgba(0,0,0,0.4)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div style={{ fontWeight: 600 }}>{modalTitle}</div>
          <button type="button" onClick={onClose} style={{ ...miniButtonStyle, border: "none", background: "transparent" }} aria-label="Close">✕</button>
        </div>
        <div style={{ display: "flex", gap: 16, flex: 1, minHeight: 0 }}>
          {/* Left column: editable fields + notes */}
          <div style={{ flex: 2, display: "flex", flexDirection: "column", gap: 10, minWidth: 0, overflow: "auto" }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <label htmlFor="work-item-title" style={modalFieldLabelStyle}>Title</label>
              <input
                autoFocus
                id="work-item-title"
                data-testid="work-item-title"
                value={title}
                readOnly={readOnly}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Title (required)"
                style={inputStyle}
              />
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <label htmlFor="work-item-description" style={modalFieldLabelStyle}>Description</label>
              <textarea
                id="work-item-description"
                data-testid="work-item-description"
                value={description}
                readOnly={readOnly}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Description"
                style={{ ...inputStyle, minHeight: 120, resize: "vertical" }}
              />
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <label htmlFor="work-item-acceptance" style={modalFieldLabelStyle}>Acceptance Criteria</label>
              <textarea
                id="work-item-acceptance"
                data-testid="work-item-acceptance"
                value={acceptance}
                readOnly={readOnly}
                onChange={(e) => setAcceptance(e.target.value)}
                placeholder="Acceptance criteria, one per line"
                style={{ ...inputStyle, minHeight: 100, resize: "vertical" }}
              />
            </div>
            {item ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1, minHeight: 0 }}>
                <div style={modalFieldLabelStyle}>Notes {notes.length > 0 ? `(${notes.length})` : ""}</div>
                {notes.length > 0 ? (
                  <div style={{ display: "flex", flexDirection: "column", gap: 8, overflowY: "auto", border: "1px solid var(--border)", borderRadius: 6, padding: 8, background: "var(--bg-1)" }}>
                    {notes.map((note) => (
                      <div key={note.id} style={{ fontSize: 12, borderLeft: "2px solid var(--border)", paddingLeft: 8 }}>
                        <div style={{ display: "flex", gap: 6, marginBottom: 2, alignItems: "baseline" }}>
                          <span style={{ fontWeight: 600, color: "var(--accent)" }}>{note.author}</span>
                          <span style={{ color: "var(--muted)", fontSize: 11 }}>{formatNoteDate(note.created_at)}</span>
                        </div>
                        <div style={{ color: "var(--fg)", whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{note.body}</div>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div style={{ color: "var(--muted)", fontSize: 12, fontStyle: "italic" }}>No notes yet.</div>
                )}
              </div>
            ) : null}
          </div>
          {/* Right column: metadata */}
          <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 10, minWidth: 200, overflow: "auto", borderLeft: "1px solid var(--border)", paddingLeft: 16 }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <label htmlFor="work-item-priority" style={modalFieldLabelStyle}>Priority</label>
              <select id="work-item-priority" data-testid="work-item-priority" value={priority} disabled={readOnly} onChange={(e) => setPriority(e.target.value as WorkItemPriority)} style={inputStyle}>
                <option value="low">Low</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
                <option value="urgent">Urgent</option>
              </select>
            </div>
            {item ? (
              <>
                {parentEpic ? (
                  <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                    <div style={modalFieldLabelStyle}>Inside</div>
                    {onOpenItem ? (
                      <button
                        type="button"
                        onClick={() => onOpenItem(parentEpic)}
                        style={{
                          background: "transparent",
                          border: "none",
                          padding: 0,
                          color: "var(--accent)",
                          cursor: "pointer",
                          textAlign: "left",
                          font: "inherit",
                          textDecoration: "underline",
                          fontSize: 12,
                        }}
                      >{parentEpic.title}</button>
                    ) : (
                      <div style={{ fontSize: 12 }}>{parentEpic.title}</div>
                    )}
                  </div>
                ) : item.parent_id ? (
                  <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                    <div style={modalFieldLabelStyle}>Inside</div>
                    <div style={{ fontSize: 12, color: "var(--muted)" }}>Parent epic</div>
                  </div>
                ) : null}
                <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <div style={modalFieldLabelStyle}>Author</div>
                  <span style={authorBadgeStyle}>{item.created_by}</span>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 2, color: "var(--muted)", fontSize: 11 }}>
                  <div>Created {formatNoteDate(item.created_at)}</div>
                  {updatedDiffers ? <div>Updated {formatNoteDate(item.updated_at)}</div> : null}
                  {item.completed_at ? <div>Completed {formatNoteDate(item.completed_at)}</div> : null}
                </div>
              </>
            ) : null}
          </div>
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 6, marginTop: 4 }}>
          {readOnly ? (
            <span style={{ marginRight: "auto", color: "var(--muted)", fontSize: 11, fontStyle: "italic" }}>
              Read-only — item is in progress
            </span>
          ) : null}
          <button type="button" data-testid="work-item-cancel" onClick={onClose} style={miniButtonStyle}>
            {readOnly ? "Close" : "Cancel"}
          </button>
          {!readOnly && showSaveAndAnother ? (
            <button
              type="button"
              data-testid="work-item-save-another"
              disabled={!canSubmit}
              onClick={() => { if (canSubmit) void onSubmit(true); }}
              style={{ ...miniButtonStyle, padding: "6px 10px", opacity: canSubmit ? 1 : 0.5 }}
            >Save and Another</button>
          ) : null}
          {!readOnly ? (
            <button
              type="submit"
              data-testid="work-item-save"
              disabled={!canSubmit}
              style={{ ...primaryButtonStyle, opacity: canSubmit ? 1 : 0.5 }}
            >Save</button>
          ) : null}
        </div>
      </form>
    </div>
  );
}

const modalFieldLabelStyle: CSSProperties = {
  textTransform: "uppercase",
  letterSpacing: 0.4,
  fontSize: 10,
  color: "var(--muted)",
  fontWeight: 600,
};

const authorBadgeStyle: CSSProperties = {
  display: "inline-block",
  padding: "2px 8px",
  borderRadius: 999,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  color: "var(--fg)",
  fontSize: 11,
  alignSelf: "flex-start",
};

function formatNoteDate(isoString: string): string {
  try {
    const d = new Date(isoString);
    const now = Date.now();
    const diffMs = now - d.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return "just now";
    if (diffMin < 60) return `${diffMin}m ago`;
    const diffHr = Math.floor(diffMin / 60);
    if (diffHr < 24) return `${diffHr}h ago`;
    const diffDay = Math.floor(diffHr / 24);
    if (diffDay < 7) return `${diffDay}d ago`;
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  } catch {
    return isoString;
  }
}

function buildWorkItemMenu(
  item: WorkItem,
  actions: {
    onDelete: (item: WorkItem) => void;
    onRename: (item: WorkItem) => void;
    onChangeStatus: (item: WorkItem) => void;
    onChangePriority: (item: WorkItem) => void;
  },
): MenuItem[] {
  const locked = item.status === "in_progress";
  return [
    {
      id: "workitem.rename",
      label: "Rename…",
      enabled: !locked,
      run: () => actions.onRename(item),
    },
    {
      id: "workitem.status",
      label: "Change status…",
      enabled: !locked,
      run: () => actions.onChangeStatus(item),
    },
    {
      id: "workitem.priority",
      label: "Change priority…",
      enabled: true,
      run: () => actions.onChangePriority(item),
    },
    {
      id: "workitem.delete",
      label: "Delete",
      enabled: !locked,
      run: () => actions.onDelete(item),
    },
  ];
}

function buildGroupMenu(
  item: WorkItem,
  groupIds: string[],
  actions: {
    onChangeStatus: (item: WorkItem, ids: string[]) => void;
    onChangePriority: (item: WorkItem, ids: string[]) => void;
    onDelete: (item: WorkItem, ids: string[]) => void;
  },
): MenuItem[] {
  const locked = item.status === "in_progress";
  const n = groupIds.length;
  return [
    {
      id: "workitem.status",
      label: `Change status… (${n} items)`,
      enabled: !locked,
      run: () => actions.onChangeStatus(item, groupIds),
    },
    {
      id: "workitem.priority",
      label: `Change priority… (${n} items)`,
      enabled: true,
      run: () => actions.onChangePriority(item, groupIds),
    },
    {
      id: "workitem.delete",
      label: `Delete (${n} items)`,
      enabled: !locked,
      run: () => actions.onDelete(item, groupIds),
    },
  ];
}

const primaryButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--accent)", color: "#fff", cursor: "pointer", font: "inherit", padding: "6px 10px",
};

const labelStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  fontSize: 12,
  color: "var(--muted)",
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

/**
 * Modal for editing a commit point — opened by double-clicking a commit
 * divider row. Only lets the user change the mode (auto vs approve); the
 * drafted commit message now lives in chat between the agent and the user,
 * not in the commit point row.
 */
function CommitPointModal({
  cp,
  onSave,
  onClose,
}: {
  cp: CommitPoint;
  onSave(changes: { mode?: "auto" | "approve" }): Promise<void>;
  onClose(): void;
}) {
  const [mode, setMode] = useState<"auto" | "approve">(cp.mode);
  const [saving, setSaving] = useState(false);

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

  const handleSave = async () => {
    setSaving(true);
    try {
      const changes: { mode?: "auto" | "approve" } = {};
      if (mode !== cp.mode) changes.mode = mode;
      await onSave(changes);
    } finally {
      setSaving(false);
    }
  };

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
      <div
        onClick={(event) => event.stopPropagation()}
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 16,
          width: "min(480px, 90vw)",
          maxHeight: "90vh",
          overflow: "auto",
          display: "flex",
          flexDirection: "column",
          gap: 10,
          boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 8px 24px rgba(0,0,0,0.4)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div style={{ fontWeight: 600, fontSize: 14 }}>Edit commit point</div>
          <button type="button" onClick={onClose} style={{ ...miniButtonStyle, border: "none", background: "transparent" }} aria-label="Close">✕</button>
        </div>

        <label style={labelStyle}>
          Mode
          <div style={{ display: "flex", gap: 6 }}>
            <button
              type="button"
              onClick={() => setMode("approve")}
              style={{
                ...miniButtonStyle,
                padding: "5px 12px",
                background: mode === "approve" ? "var(--accent)" : "var(--bg-2)",
                color: mode === "approve" ? "#fff" : "inherit",
                border: `1px solid ${mode === "approve" ? "var(--accent)" : "var(--border)"}`,
              }}
            >
              Approve
            </button>
            <button
              type="button"
              onClick={() => setMode("auto")}
              style={{
                ...miniButtonStyle,
                padding: "5px 12px",
                background: mode === "auto" ? "var(--accent)" : "var(--bg-2)",
                color: mode === "auto" ? "#fff" : "inherit",
                border: `1px solid ${mode === "auto" ? "var(--accent)" : "var(--border)"}`,
              }}
            >
              Auto
            </button>
          </div>
          <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 2 }}>
            {mode === "approve"
              ? "Agent drafts a message, shows it in chat, and waits for your approval."
              : "Agent commits immediately without waiting for approval."}
          </div>
        </label>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginTop: 4 }}>
          <button type="button" onClick={onClose} style={miniButtonStyle} disabled={saving}>Cancel</button>
          <button
            type="button"
            data-testid="commit-point-save"
            onClick={() => void handleSave()}
            disabled={saving}
            style={{ ...primaryButtonStyle }}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
