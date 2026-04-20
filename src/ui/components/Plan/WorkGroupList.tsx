import type { CSSProperties } from "react";
import { useMemo, useState } from "react";
import type { CommitPoint, WaitPoint, WorkItem, WorkItemPriority, WorkItemStatus } from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { CommitPointRow } from "./CommitPointRow.js";
import {
  classifyWorkItem,
  groupHeaderStyle,
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
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [overSection, setOverSection] = useState<WorkItemSectionKind | null>(null);
  // Archived items fold into the Done section but are hidden by default — the
  // done-section header carries a "Show archived (N)" toggle and an
  // "Archive all" action.
  const [showArchived, setShowArchived] = useState(false);

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
    // Cross-section drop of a work item — change its status to match the
    // target section. The reorder that follows keeps its relative position
    // in the flattened list.
    if (dragged.kind === "work" && target.kind === "work") {
      const fromSection = classifyWorkItem(dragged.item.status);
      const toSection = classifyWorkItem(target.item.status);
      if (fromSection !== toSection) {
        const nextStatus = sectionDefaultStatus(toSection);
        if (nextStatus && nextStatus !== dragged.item.status) {
          void onUpdateWorkItem(dragged.item.id, { status: nextStatus });
        }
      }
    }
    const next = allRows.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
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
    void onUpdateWorkItem(draggedWorkItem.id, { status: nextStatus });
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
        onDragStart={(event) => {
          setDraggingKey(key);
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", row.item.id);
          // If the dragged row is part of the mark set, ship every marked id
          // with the payload so the drop target (BatchRail / backlog chip /
          // stream chip) can move them all at once. Otherwise it's a single-
          // item drag as before.
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
      />
    );
  };

  return (
    <div>
      {group.epic ? (
        <div style={groupHeaderStyle}>
          <span style={{ marginRight: 6 }}>{statusIcon(group.epic.status)}</span>
          <span style={{ fontWeight: 600, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{group.epic.title}</span>
          <PriorityIcon priority={group.epic.priority} />
        </div>
      ) : null}
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
          index === 0 && !group.epic ? firstSectionLabelStyle : sectionHeaderStyle;
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
          // Cmd/Ctrl+click toggles the mark set; Shift+click ranges from the
          // primary selection. Both skip the edit modal so multi-select
          // doesn't open the form for every clicked row.
          onSelect?.(item.id, { toggle, range });
          return;
        }
        // Plain click: select + open the edit modal. Title/description/
        // acceptance edits all happen there; the row itself only exposes
        // the inline status + priority pickers.
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
}: {
  status: WorkItemStatus;
  onChange(next: WorkItemStatus): void;
}) {
  return (
    <span
      onClick={(event) => event.stopPropagation()}
      style={{ position: "relative", display: "inline-block", flexShrink: 0, width: 14, textAlign: "center" }}
      title={`Status: ${statusLabel(status)} — click to change`}
    >
      <span>{statusIcon(status)}</span>
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
