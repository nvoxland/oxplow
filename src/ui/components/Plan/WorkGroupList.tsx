import type { CSSProperties } from "react";
import { useMemo, useState } from "react";
import type { CommitPoint, WaitPoint, WorkItem, WorkItemPriority, WorkItemStatus } from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { CommitPointRow } from "./CommitPointRow.js";
import {
  classifyWorkItem,
  sectionDefaultStatus,
  sectionHeaderStyle,
  statusIcon,
  statusLabel,
  type WorkItemGroup,
  type WorkItemSectionKind,
} from "./plan-utils.js";
import { PriorityIcon } from "./plan-icons.js";
import {
  commitDividerBadgeStyle,
  commitDividerLineStyle,
  commitDividerStyle,
  waitDividerBadgeStyle,
} from "./queue-markers.js";
import { WaitPointRow } from "./WaitPointRow.js";
import type { WorkItemDetailChanges } from "./WorkItemDetail.js";

/**
 * Renders one work-item group (an epic + its children, or the root group
 * with no epic). Items are split by status into four sections —
 * In progress → To do → Human check → Done — with dividers between non-empty
 * sections. Commit / wait points are interleaved into the To do section
 * only (they represent work yet to run); clicking a divider expands its
 * CommitPointRow / WaitPointRow inline. The "+ Commit when done" and
 * "+ Wait here" buttons hang off the tail of the To do section via
 * `addPointsSlot` so the shape of the queue doesn't require scrolling past
 * done work.
 *
 * Drag-reorder rewrites `sort_index` globally. Dragging a work item across
 * section boundaries also changes its status to that section's default
 * (toDo → ready, humanCheck → human_check, done → done) so the user can
 * triage straight from the Work panel. InProgress rejects drop-in: the
 * agent owns that status and in-progress items are drag-locked. Empty
 * sections stay hidden until a drag is active, at which point they appear
 * as drop targets.
 */
export type QueueRow =
  | { kind: "work"; id: string; sortIndex: number; item: WorkItem }
  | { kind: "commit"; id: string; sortIndex: number; cp: CommitPoint }
  | { kind: "wait"; id: string; sortIndex: number; wp: WaitPoint };

interface SectionBucket {
  kind: WorkItemSectionKind;
  label: string;
  rows: QueueRow[];
}

const SECTION_ORDER: Array<{ kind: WorkItemSectionKind; label: string }> = [
  { kind: "inProgress", label: "In progress" },
  { kind: "toDo", label: "Ready" },
  { kind: "humanCheck", label: "Human check" },
  { kind: "blocked", label: "Blocked" },
  { kind: "done", label: "Done" },
];

export function WorkGroupList({
  group,
  scopeBatchId,
  expandedId,
  onToggleExpand,
  onUpdateWorkItem,
  onReorderWorkItems,
  commitPoints,
  waitPoints,
  onReorderMixed,
  onContextMenu,
  addPointsSlot,
  selectedId,
  markedIds,
  onSelect,
  onRequestEdit,
  epicChildrenMap,
  onReparentWorkItem,
}: {
  group: WorkItemGroup;
  scopeBatchId: string | null;
  expandedId: string | null;
  onToggleExpand(id: string): void;
  onUpdateWorkItem: (itemId: string, changes: WorkItemDetailChanges) => Promise<void>;
  onReorderWorkItems: (orderedItemIds: string[]) => Promise<void>;
  commitPoints?: CommitPoint[];
  waitPoints?: WaitPoint[];
  onReorderMixed?(entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>): void;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  addPointsSlot?: React.ReactNode;
  selectedId?: string | null;
  markedIds?: ReadonlySet<string>;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  epicChildrenMap: Map<string, WorkItem[]>;
  onReparentWorkItem: (itemId: string, newParentId: string | null) => Promise<void>;
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [overSection, setOverSection] = useState<WorkItemSectionKind | null>(null);
  // Archived items fold into the Done section but are hidden by default — the
  // done-section header carries a "Show archived (N)" toggle and an
  // "Archive all" action.
  const [showArchived, setShowArchived] = useState(false);
  const [expandedEpicIds, setExpandedEpicIds] = useState<Set<string>>(() => new Set());

  const { sections, allRows } = useMemo(() => {
    const work: QueueRow[] = group.items.map((item) => ({
      kind: "work" as const, id: item.id, sortIndex: item.sort_index, item,
    }));
    const buckets: Record<WorkItemSectionKind, QueueRow[]> = {
      inProgress: [], toDo: [], humanCheck: [], blocked: [], done: [],
    };
    for (const row of work) {
      if (row.kind !== "work") continue;
      buckets[classifyWorkItem(row.item.status)].push(row);
    }
    // Commit / wait points only belong to the To do section — they represent
    // a future action. Done / triggered markers are historical; hide them so
    // the To do list is actually the to-do list (the commit sha is in git
    // log; the wait point already fired).
    for (const cp of commitPoints ?? []) {
      if (cp.status === "done") continue;
      buckets.toDo.push({ kind: "commit", id: cp.id, sortIndex: cp.sort_index, cp });
    }
    for (const wp of waitPoints ?? []) {
      if (wp.status === "triggered") continue;
      buckets.toDo.push({ kind: "wait", id: wp.id, sortIndex: wp.sort_index, wp });
    }
    const orderedSections: SectionBucket[] = [];
    const flat: QueueRow[] = [];
    for (const { kind, label } of SECTION_ORDER) {
      buckets[kind].sort((a, b) => a.sortIndex - b.sortIndex);
      // Keep empty sections in the list while a drag is active so the user
      // can drop into an empty "Done" / "Human check" to create the first
      // item there. When nothing is dragging, empty sections are suppressed
      // by the renderer below.
      if (buckets[kind].length === 0) {
        orderedSections.push({ kind, label, rows: [] });
      } else {
        orderedSections.push({ kind, label, rows: buckets[kind] });
        flat.push(...buckets[kind]);
      }
    }
    return { sections: orderedSections, allRows: flat };
  }, [group.items, commitPoints, waitPoints]);

  const keyFor = (row: { kind: string; id: string }) => `${row.kind}:${row.id}`;

  // Look up the dragged work item (if any) so cross-section drops can route
  // through onUpdateWorkItem. Commit/wait rows don't have a status to change.
  const draggedWorkItem = (() => {
    if (!draggingKey) return null;
    const row = allRows.find((r) => keyFor(r) === draggingKey);
    return row && row.kind === "work" ? row.item : null;
  })();

  const resetDrag = () => { setDraggingKey(null); setOverKey(null); setOverSection(null); };

  const handleDropOnKey = (targetKey: string) => {
    if (!draggingKey || draggingKey === targetKey) { resetDrag(); return; }
    const from = allRows.findIndex((row) => keyFor(row) === draggingKey);
    const to = allRows.findIndex((row) => keyFor(row) === targetKey);
    if (from < 0 || to < 0) { resetDrag(); return; }
    const dragged = allRows[from]!;
    const target = allRows[to]!;
    const isMultiDrag = dragged.kind === "work" && markedIds && markedIds.has(dragged.item.id) && markedIds.size > 1;
    // Cross-section drop — change status to match the target section.
    // When it's a multi-drag, apply the status change to every marked item.
    if (dragged.kind === "work" && target.kind === "work") {
      const fromSection = classifyWorkItem(dragged.item.status);
      const toSection = classifyWorkItem(target.item.status);
      if (fromSection !== toSection) {
        const nextStatus = sectionDefaultStatus(toSection);
        if (nextStatus) {
          if (isMultiDrag && markedIds) {
            for (const id of markedIds) {
              const row = allRows.find((r) => r.kind === "work" && r.id === id);
              if (row && row.kind === "work" && row.item.status !== nextStatus) {
                void onUpdateWorkItem(id, { status: nextStatus });
              }
            }
          } else if (nextStatus !== dragged.item.status) {
            void onUpdateWorkItem(dragged.item.id, { status: nextStatus });
          }
        }
      }
    }
    // Reorder: multi-drag moves all marked rows as a block to the drop position.
    let next: QueueRow[];
    if (isMultiDrag && markedIds) {
      const markedSet = new Set(markedIds);
      const markedRows = allRows.filter((r) => r.kind === "work" && markedSet.has(r.id));
      const unmarked = allRows.filter((r) => r.kind !== "work" || !markedSet.has(r.id));
      const insertIdx = unmarked.findIndex((r) => keyFor(r) === targetKey);
      const insertAt = insertIdx < 0 ? unmarked.length : insertIdx;
      next = [...unmarked.slice(0, insertAt), ...markedRows, ...unmarked.slice(insertAt)];
    } else {
      next = allRows.slice();
      const [moved] = next.splice(from, 1);
      next.splice(to, 0, moved!);
    }
    resetDrag();
    if (onReorderMixed && ((commitPoints?.length ?? 0) > 0 || (waitPoints?.length ?? 0) > 0)) {
      onReorderMixed(next.map((row) => ({ kind: row.kind, id: row.id })));
    } else {
      void onReorderWorkItems(next.filter((row) => row.kind === "work").map((row) => row.id));
    }
  };

  const handleDropOnSection = (section: WorkItemSectionKind) => {
    if (!draggedWorkItem) { resetDrag(); return; }
    const nextStatus = sectionDefaultStatus(section);
    resetDrag();
    if (!nextStatus) return;
    if (classifyWorkItem(draggedWorkItem.status) === section) return;
    const isMultiDrag = markedIds && markedIds.has(draggedWorkItem.id) && markedIds.size > 1;
    if (isMultiDrag && markedIds) {
      for (const id of markedIds) {
        const row = allRows.find((r) => r.kind === "work" && r.id === id);
        if (row && row.kind === "work" && classifyWorkItem(row.item.status) !== section) {
          void onUpdateWorkItem(id, { status: nextStatus });
        }
      }
    } else {
      void onUpdateWorkItem(draggedWorkItem.id, { status: nextStatus });
    }
  };

  const renderRow = (row: QueueRow) => {
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
            onDragEnd={resetDrag}
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
            <span style={commitDividerBadgeStyle(row.cp.status)}>
              commit
              {row.cp.status === "proposed" ? " · drafted" : ""}
              {row.cp.status === "done" ? " · done" : ""}
            </span>
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
            onDragEnd={resetDrag}
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
    const isMarked = markedIds?.has(row.item.id) ?? false;
    const sharedDragHandlers = {
      onDragStart: (event: React.DragEvent) => {
        setDraggingKey(key);
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", row.item.id);
        const ids = isMarked && markedIds && markedIds.size > 1
          ? [...markedIds]
          : [row.item.id];
        event.dataTransfer.setData(
          WORK_ITEM_DRAG_MIME,
          JSON.stringify({
            itemId: row.item.id,
            itemIds: ids,
            fromBatchId: scopeBatchId,
          }),
        );
      },
      onDragEnd: resetDrag,
      onDragOver: (event: React.DragEvent) => {
        if (!draggingKey || draggingKey === key) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        if (overKey !== key) setOverKey(key);
      },
      onDragLeave: () => { if (overKey === key) setOverKey(null); },
      onDrop: (event: React.DragEvent) => {
        event.preventDefault();
        if (draggingKey) { handleDropOnKey(key); return; }
        const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
        if (!raw) return;
        try {
          const payload = JSON.parse(raw) as { itemId?: string; itemIds?: string[]; parentEpicId?: string };
          if (payload.parentEpicId) {
            const ids = payload.itemIds?.length ? payload.itemIds : payload.itemId ? [payload.itemId] : [];
            for (const id of ids) void onReparentWorkItem(id, null);
          }
        } catch { /* ignore */ }
      },
    };
    if (row.item.kind === "epic") {
      const isExpanded = expandedEpicIds.has(row.item.id);
      const children = epicChildrenMap.get(row.item.id) ?? [];
      return (
        <div key={key}>
          <EpicInlineRow
            rowKey={key}
            item={row.item}
            isExpanded={isExpanded}
            onToggleExpand={() => {
              setExpandedEpicIds((prev) => {
                const next = new Set(prev);
                if (next.has(row.item.id)) next.delete(row.item.id);
                else next.add(row.item.id);
                return next;
              });
            }}
            isSelected={selectedId === row.item.id}
            isMarked={isMarked}
            isOver={isOver}
            isDragging={isDragging}
            scopeBatchId={scopeBatchId}
            onSelect={onSelect}
            onRequestEdit={onRequestEdit}
            onUpdateWorkItem={onUpdateWorkItem}
            onContextMenu={onContextMenu}
            {...sharedDragHandlers}
          />
          {isExpanded ? (
            <EpicChildrenPane
              epicId={row.item.id}
              children={children}
              onReorderWorkItems={onReorderWorkItems}
              onReparentWorkItem={onReparentWorkItem}
              onUpdateWorkItem={onUpdateWorkItem}
              onContextMenu={onContextMenu}
              scopeBatchId={scopeBatchId}
              onRequestEdit={onRequestEdit}
              selectedId={selectedId}
              markedIds={markedIds}
              onSelect={onSelect}
            />
          ) : null}
        </div>
      );
    }
    return (
      <InlineItemRow
        key={key}
        rowKey={key}
        item={row.item}
        isSelected={selectedId === row.item.id}
        isMarked={isMarked}
        isOver={isOver}
        isDragging={isDragging}
        scopeBatchId={scopeBatchId}
        onRequestEdit={onRequestEdit}
        onSelect={onSelect}
        onUpdateWorkItem={onUpdateWorkItem}
        onContextMenu={onContextMenu}
        {...sharedDragHandlers}
      />
    );
  };

  return (
    <div>
      {sections.map((section, index) => {
        const empty = section.rows.length === 0;
        // "To do" is the primary queue surface and always renders (even empty)
        // so the user has a visible anchor for the add-points slot and an
        // obvious drop target. Other empty sections only appear while a work
        // item is actively being dragged — they act as drop zones for status
        // changes. inProgress stays hidden when empty either way.
        if (empty && section.kind !== "toDo" && (!draggedWorkItem || section.kind === "inProgress")) {
          return null;
        }
        const canDrop = !!draggedWorkItem
          && section.kind !== "inProgress"
          && classifyWorkItem(draggedWorkItem.status) !== section.kind;
        const isOverSection = canDrop && overSection === section.kind;
        const headerDropHandlers = canDrop
          ? {
              onDragOver: (event: React.DragEvent) => {
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (overSection !== section.kind) setOverSection(section.kind);
              },
              onDragLeave: () => {
                if (overSection === section.kind) setOverSection(null);
              },
              onDrop: (event: React.DragEvent) => {
                event.preventDefault();
                handleDropOnSection(section.kind);
              },
            }
          : {};
        const headerBaseStyle: CSSProperties =
          index === 0 ? firstSectionLabelStyle : sectionHeaderStyle;
        const headerStyle: CSSProperties = {
          ...headerBaseStyle,
          outline: isOverSection ? "1px solid var(--accent)" : "none",
          background: isOverSection ? "rgba(74,158,255,0.08)" : headerBaseStyle.background,
          cursor: canDrop ? "copy" : headerBaseStyle.cursor,
        };
        const isDone = section.kind === "done";
        // Split the done bucket into "visible done" (done + canceled) and
        // "archived" so the header can surface both an archive-all action
        // (which only operates on visible done rows) and a toggle for the
        // archived ones. When the toggle is on, archived rows render
        // right after the regular done rows.
        const archivedRows = isDone
          ? section.rows.filter((r) => r.kind === "work" && r.item.status === "archived")
          : [];
        const visibleDoneRows = isDone
          ? section.rows.filter((r) => r.kind !== "work" || r.item.status !== "archived")
          : section.rows;
        const renderedRows = isDone && !showArchived ? visibleDoneRows : section.rows;
        const archivableCount = isDone
          ? visibleDoneRows.filter((r) => r.kind === "work" && r.item.status !== "archived").length
          : 0;
        return (
          <div key={section.kind} data-testid={`plan-section-${section.kind}`}>
            <div
              style={{ ...headerStyle, display: "flex", alignItems: "center", gap: 8 }}
              data-testid={`plan-section-header-${section.kind}`}
              {...headerDropHandlers}
            >
              <span style={{ flex: 1, minWidth: 0 }}>{section.label}</span>
              {isDone ? (
                <DoneHeaderActions
                  archivableCount={archivableCount}
                  archivedCount={archivedRows.length}
                  showArchived={showArchived}
                  onToggleArchived={() => setShowArchived((v) => !v)}
                  onArchiveAll={() => {
                    for (const row of visibleDoneRows) {
                      if (row.kind !== "work") continue;
                      if (row.item.status === "archived") continue;
                      void onUpdateWorkItem(row.item.id, { status: "archived" });
                    }
                  }}
                />
              ) : null}
            </div>
            {renderedRows.map(renderRow)}
            {/* The "+ Commit when done" / "+ Wait here" bar hangs off the tail
                of the "To do" section (not the very bottom of the list) so
                queueing a marker feels like appending to the active queue
                rather than the dead/done pile. Because "To do" always renders
                (even when empty), the slot is always reachable. */}
            {section.kind === "toDo" ? addPointsSlot : null}
          </div>
        );
      })}
    </div>
  );
}

function DoneHeaderActions({
  archivableCount,
  archivedCount,
  showArchived,
  onToggleArchived,
  onArchiveAll,
}: {
  archivableCount: number;
  archivedCount: number;
  showArchived: boolean;
  onToggleArchived(): void;
  onArchiveAll(): void;
}) {
  return (
    <span style={{ display: "flex", alignItems: "center", gap: 6, textTransform: "none", letterSpacing: 0 }}>
      {archivedCount > 0 ? (
        <button
          type="button"
          onClick={(event) => { event.stopPropagation(); onToggleArchived(); }}
          style={{ ...miniDoneHeaderButtonStyle }}
          data-testid="plan-done-toggle-archived"
          title={showArchived ? "Hide archived items" : "Show archived items inline with Done"}
        >
          {showArchived ? `▾ Hide archived (${archivedCount})` : `▸ Show archived (${archivedCount})`}
        </button>
      ) : null}
      {archivableCount > 0 ? (
        <button
          type="button"
          onClick={(event) => { event.stopPropagation(); onArchiveAll(); }}
          style={{ ...miniDoneHeaderButtonStyle }}
          data-testid="plan-done-archive-all"
          title={`Archive all ${archivableCount} visible done item${archivableCount === 1 ? "" : "s"}`}
        >
          Archive all
        </button>
      ) : null}
    </span>
  );
}

const miniDoneHeaderButtonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "var(--fg)",
  cursor: "pointer",
  font: "inherit",
  padding: "1px 6px",
  fontSize: 10,
};

const firstSectionLabelStyle: CSSProperties = {
  ...sectionHeaderStyle,
  borderTop: "none",
};

const STATUS_OPTIONS: WorkItemStatus[] = [
  "blocked", "ready", "in_progress", "human_check", "done", "canceled", "archived",
];
const PRIORITY_OPTIONS: WorkItemPriority[] = ["urgent", "high", "medium", "low"];

function EpicInlineRow({
  rowKey, item, isExpanded, onToggleExpand,
  isSelected, isMarked, isOver, isDragging,
  scopeBatchId, onSelect, onRequestEdit, onUpdateWorkItem, onContextMenu,
  onDragStart, onDragEnd, onDragOver, onDragLeave, onDrop,
}: {
  rowKey: string;
  item: WorkItem;
  isExpanded: boolean;
  onToggleExpand(): void;
  isSelected: boolean;
  isMarked: boolean;
  isOver: boolean;
  isDragging: boolean;
  scopeBatchId: string | null;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  onUpdateWorkItem: (itemId: string, changes: WorkItemDetailChanges) => Promise<void>;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  onDragStart(event: React.DragEvent): void;
  onDragEnd(event: React.DragEvent): void;
  onDragOver(event: React.DragEvent): void;
  onDragLeave(event: React.DragEvent): void;
  onDrop(event: React.DragEvent): void;
}) {
  const dimmed = item.status === "done" || item.status === "canceled" || item.status === "archived";
  const locked = item.status === "in_progress";
  void scopeBatchId;
  return (
    <div
      draggable={!locked}
      onDragStart={locked ? undefined : onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      onClick={(event) => {
        const toggle = event.metaKey || event.ctrlKey;
        const range = event.shiftKey && !toggle;
        if (toggle || range) { onSelect?.(item.id, { toggle, range }); return; }
        onSelect?.(item.id);
      }}
      onDoubleClick={(event) => {
        if (event.metaKey || event.ctrlKey || event.shiftKey) return;
        onSelect?.(item.id);
        onRequestEdit?.(item);
      }}
      onContextMenu={(event) => onContextMenu(event, item)}
      style={{
        display: "flex", alignItems: "center", gap: 6, padding: "4px 8px",
        cursor: isDragging ? "grabbing" : "pointer",
        borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
        borderLeft: isMarked ? "2px solid var(--priority-urgent)" : isSelected ? "2px solid var(--accent)" : "2px solid transparent",
        background: isMarked ? "rgba(234,179,8,0.14)" : isSelected ? "rgba(74,158,255,0.12)" : isDragging ? "rgba(255,255,255,0.04)" : "transparent",
        fontSize: 12, userSelect: "none", opacity: dimmed ? 0.6 : 1,
      }}
      title={locked ? `${item.title} (in progress — pinned in place)` : item.title}
      data-key={rowKey}
      data-testid={`work-item-row-${item.id}`}
    >
      <span
        onClick={(event) => { event.stopPropagation(); onToggleExpand(); }}
        style={{ flexShrink: 0, width: 12, textAlign: "center", color: "var(--muted)", fontSize: 10, cursor: "pointer" }}
        title={isExpanded ? "Collapse epic children" : "Expand epic children"}
      >
        {isExpanded ? "\u25BC" : "\u25B6"}
      </span>
      <InlineStatusPicker status={item.status} onChange={(status) => { void onUpdateWorkItem(item.id, { status }); }} locked={item.status === "in_progress"} />
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: 500 }}>
        {item.title}
      </span>
      <InlinePriorityPicker priority={item.priority} onChange={(priority) => { void onUpdateWorkItem(item.id, { priority }); }} />
    </div>
  );
}

function EpicChildrenPane({
  epicId, children, onReorderWorkItems, onReparentWorkItem,
  onUpdateWorkItem, onContextMenu, scopeBatchId, onRequestEdit,
  selectedId, markedIds, onSelect,
}: {
  epicId: string;
  children: WorkItem[];
  onReorderWorkItems(ids: string[]): Promise<void>;
  onReparentWorkItem(itemId: string, newParentId: string | null): Promise<void>;
  onUpdateWorkItem(itemId: string, changes: WorkItemDetailChanges): Promise<void>;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  scopeBatchId: string | null;
  onRequestEdit?(item: WorkItem): void;
  selectedId?: string | null;
  markedIds?: ReadonlySet<string>;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [dropTargetOver, setDropTargetOver] = useState(false);
  const resetDrag = () => { setDraggingKey(null); setOverKey(null); };

  const handleDropOnChild = (targetId: string) => {
    if (!draggingKey || draggingKey === targetId) { resetDrag(); return; }
    const from = children.findIndex((c) => c.id === draggingKey);
    const to = children.findIndex((c) => c.id === targetId);
    if (from < 0 || to < 0) { resetDrag(); return; }
    const next = children.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
    resetDrag();
    void onReorderWorkItems(next.map((c) => c.id));
  };

  const handleExternalDrop = (event: React.DragEvent) => {
    event.preventDefault();
    setDropTargetOver(false);
    const raw = event.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
    if (!raw) return;
    try {
      const payload = JSON.parse(raw) as { itemId?: string; itemIds?: string[]; fromBatchId?: string | null };
      const ids = payload.itemIds && payload.itemIds.length > 0 ? payload.itemIds : payload.itemId ? [payload.itemId] : [];
      for (const id of ids) {
        if (!children.some((c) => c.id === id)) {
          void onReparentWorkItem(id, epicId);
        }
      }
    } catch { /* ignore */ }
  };

  return (
    <div style={{ marginLeft: 20, borderLeft: "2px solid var(--border)", paddingLeft: 4 }}>
      {children.map((child) => {
        const key = child.id;
        const isOver = overKey === key && draggingKey !== key;
        const isDragging = draggingKey === key;
        const isMarked = markedIds?.has(child.id) ?? false;
        return (
          <InlineItemRow
            key={key}
            rowKey={key}
            item={child}
            isSelected={selectedId === child.id}
            isMarked={isMarked}
            isOver={isOver}
            isDragging={isDragging}
            scopeBatchId={scopeBatchId}
            onSelect={onSelect}
            onRequestEdit={onRequestEdit}
            onUpdateWorkItem={onUpdateWorkItem}
            onContextMenu={onContextMenu}
            onDragStart={(event) => {
              setDraggingKey(key);
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", child.id);
              const ids = isMarked && markedIds && markedIds.size > 1 ? [...markedIds] : [child.id];
              event.dataTransfer.setData(
                WORK_ITEM_DRAG_MIME,
                JSON.stringify({ itemId: child.id, itemIds: ids, fromBatchId: scopeBatchId, parentEpicId: epicId }),
              );
            }}
            onDragEnd={resetDrag}
            onDragOver={(event) => {
              if (!draggingKey || draggingKey === key) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (overKey !== key) setOverKey(key);
            }}
            onDragLeave={() => { if (overKey === key) setOverKey(null); }}
            onDrop={(event) => { event.preventDefault(); handleDropOnChild(key); }}
          />
        );
      })}
      <div
        onDragOver={(event) => {
          if (!event.dataTransfer.types.includes(WORK_ITEM_DRAG_MIME)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          setDropTargetOver(true);
        }}
        onDragLeave={() => setDropTargetOver(false)}
        onDrop={handleExternalDrop}
        style={{
          height: 24, display: "flex", alignItems: "center", paddingLeft: 8,
          fontSize: 10, color: dropTargetOver ? "var(--accent)" : "var(--muted)",
          borderTop: dropTargetOver ? "1px solid var(--accent)" : "1px solid transparent",
          opacity: dropTargetOver ? 1 : 0.5,
        }}
      >
        {children.length === 0 ? "Drop items here to add to epic" : ""}
      </div>
    </div>
  );
}

/**
 * One collapsed work-item row with inline editing. Title click swaps to an
 * input; status icon and priority marker each open a transparent <select>
 * overlay that commits on change. Clicking anywhere else on the row toggles
 * the expanded WorkItemDetail (for description + acceptance + delete).
 *
 * Drag-reorder and right-click-to-delete still hang off the outer row div;
 * the inline controls stopPropagation so they don't bubble to the row's
 * expand click.
 */
function InlineItemRow({
  rowKey,
  item,
  isSelected,
  isMarked,
  isOver,
  isDragging,
  scopeBatchId,
  onSelect,
  onRequestEdit,
  onUpdateWorkItem,
  onContextMenu,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  rowKey: string;
  item: WorkItem;
  isSelected: boolean;
  isMarked: boolean;
  isOver: boolean;
  isDragging: boolean;
  scopeBatchId: string | null;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  onUpdateWorkItem: (itemId: string, changes: WorkItemDetailChanges) => Promise<void>;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  onDragStart(event: React.DragEvent): void;
  onDragEnd(event: React.DragEvent): void;
  onDragOver(event: React.DragEvent): void;
  onDragLeave(event: React.DragEvent): void;
  onDrop(event: React.DragEvent): void;
}) {
  const dimmed = item.status === "done" || item.status === "canceled" || item.status === "archived";
  const locked = item.status === "in_progress";

  // scopeBatchId isn't used directly here, but the outer drag handler that
  // encoded it into dataTransfer was captured at onDragStart creation time —
  // suppress the unused-parameter lint without plumbing it away.
  void scopeBatchId;

  return (
    <div
      draggable={!locked}
      onDragStart={locked ? undefined : onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      onClick={(event) => {
        const toggle = event.metaKey || event.ctrlKey;
        const range = event.shiftKey && !toggle;
        if (toggle || range) {
          onSelect?.(item.id, { toggle, range });
          return;
        }
        // Plain click: select only. Double-click (or Enter) opens the edit modal.
        onSelect?.(item.id);
      }}
      onDoubleClick={(event) => {
        if (event.metaKey || event.ctrlKey || event.shiftKey) return;
        onSelect?.(item.id);
        onRequestEdit?.(item);
      }}
      onContextMenu={(event) => onContextMenu(event, item)}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "4px 8px",
        cursor: isDragging ? "grabbing" : "pointer",
        borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
        borderLeft: isMarked
          ? "2px solid var(--priority-urgent)"
          : isSelected ? "2px solid var(--accent)" : "2px solid transparent",
        background: isMarked
          ? "rgba(234,179,8,0.14)"
          : isSelected
            ? "rgba(74,158,255,0.12)"
            : isDragging
              ? "rgba(255,255,255,0.04)"
              : "transparent",
        fontSize: 12,
        userSelect: "none",
        opacity: dimmed ? 0.6 : 1,
      }}
      title={locked ? `${item.title} (in progress — pinned in place)` : item.title}
      data-key={rowKey}
      data-testid={`work-item-row-${item.id}`}
    >
      <InlineStatusPicker
        status={item.status}
        onChange={(status) => { void onUpdateWorkItem(item.id, { status }); }}
        locked={item.status === "in_progress"}
      />
      <span
        style={{
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {item.title}
      </span>
      <InlinePriorityPicker
        priority={item.priority}
        onChange={(priority) => { void onUpdateWorkItem(item.id, { priority }); }}
      />
    </div>
  );
}

function InlineStatusPicker({
  status,
  onChange,
  locked,
}: {
  status: WorkItemStatus;
  onChange(next: WorkItemStatus): void;
  locked?: boolean;
}) {
  return (
    <span
      onClick={(event) => event.stopPropagation()}
      style={{ position: "relative", display: "inline-block", flexShrink: 0, width: 14, textAlign: "center" }}
      title={locked ? `Status: ${statusLabel(status)} — locked while in progress` : `Status: ${statusLabel(status)} — click to change`}
    >
      <span>{statusIcon(status)}</span>
      {!locked ? (
        <select
          value={status}
          onChange={(event) => onChange(event.target.value as WorkItemStatus)}
          onClick={(event) => event.stopPropagation()}
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
          {STATUS_OPTIONS.map((option) => (
            <option key={option} value={option}>{statusLabel(option)}</option>
          ))}
        </select>
      ) : null}
    </span>
  );
}

function InlinePriorityPicker({
  priority,
  onChange,
}: {
  priority: WorkItemPriority;
  onChange(next: WorkItemPriority): void;
}) {
  return (
    <span
      onClick={(event) => event.stopPropagation()}
      style={{ position: "relative", display: "inline-block", flexShrink: 0 }}
      title={`Priority: ${priority} — click to change`}
    >
      <PriorityIcon priority={priority} />
      <select
        value={priority}
        onChange={(event) => onChange(event.target.value as WorkItemPriority)}
        onClick={(event) => event.stopPropagation()}
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
        {PRIORITY_OPTIONS.map((option) => (
          <option key={option} value={option}>{option}</option>
        ))}
      </select>
    </span>
  );
}
