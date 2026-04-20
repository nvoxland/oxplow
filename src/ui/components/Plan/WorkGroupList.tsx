import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CommitPoint, WaitPoint, WorkItem, WorkItemPriority, WorkItemStatus } from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { CommitPointRow } from "./CommitPointRow.js";
import {
  classifyWorkItem,
  groupHeaderStyle,
  inputStyle,
  sectionDefaultStatus,
  sectionHeaderStyle,
  statusIcon,
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
import { WorkItemDetail, type WorkItemDetailChanges } from "./WorkItemDetail.js";

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
  { kind: "toDo", label: "To do" },
  { kind: "humanCheck", label: "Human check" },
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
  onRequestDelete,
  onContextMenu,
  addPointsSlot,
  selectedId,
  onSelect,
  renameRequest,
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
  onRequestDelete(item: WorkItem): void;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  addPointsSlot?: React.ReactNode;
  selectedId?: string | null;
  onSelect?(id: string): void;
  renameRequest?: { itemId: string; nonce: number } | null;
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [overSection, setOverSection] = useState<WorkItemSectionKind | null>(null);

  const { sections, allRows } = useMemo(() => {
    const work: QueueRow[] = group.items.map((item) => ({
      kind: "work" as const, id: item.id, sortIndex: item.sort_index, item,
    }));
    const buckets: Record<WorkItemSectionKind, QueueRow[]> = {
      inProgress: [], toDo: [], humanCheck: [], done: [],
    };
    for (const row of work) {
      if (row.kind !== "work") continue;
      buckets[classifyWorkItem(row.item.status)].push(row);
    }
    // Commit / wait points only belong to the To do section — they represent
    // a future action, not something that has run or is waiting for review.
    for (const cp of commitPoints ?? []) {
      buckets.toDo.push({ kind: "commit", id: cp.id, sortIndex: cp.sort_index, cp });
    }
    for (const wp of waitPoints ?? []) {
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
            <span style={commitDividerLineStyle} />
            <span style={commitDividerBadgeStyle(row.cp.status)}>
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
    return (
      <InlineItemRow
        key={key}
        rowKey={key}
        item={row.item}
        isExpanded={expandedId === row.item.id}
        isSelected={selectedId === row.item.id}
        renameRequest={renameRequest && renameRequest.itemId === row.item.id ? renameRequest : null}
        isOver={isOver}
        isDragging={isDragging}
        scopeBatchId={scopeBatchId}
        onToggleExpand={onToggleExpand}
        onSelect={onSelect}
        onUpdateWorkItem={onUpdateWorkItem}
        onRequestDelete={onRequestDelete}
        onContextMenu={onContextMenu}
        onDragStart={(event) => {
          setDraggingKey(key);
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", row.item.id);
          event.dataTransfer.setData(
            WORK_ITEM_DRAG_MIME,
            JSON.stringify({ itemId: row.item.id, fromBatchId: scopeBatchId }),
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
        // Only show empty sections while a work item is actively being
        // dragged — they act as drop zones for status changes. Otherwise
        // hide them so the panel stays compact.
        if (empty && (!draggedWorkItem || section.kind === "inProgress")) {
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
        return (
          <div key={section.kind} data-testid={`plan-section-${section.kind}`}>
            <div
              style={headerStyle}
              data-testid={`plan-section-header-${section.kind}`}
              {...headerDropHandlers}
            >
              {section.label}
            </div>
            {section.rows.map(renderRow)}
          </div>
        );
      })}
      {/* Queue-marker bar always renders after the sections when the parent
          provides one, so users can add a commit or wait point even when the
          To do section is empty (e.g. after the agent has pushed every item
          to human_check). Previously this slot lived inside the toDo section
          and disappeared whenever that section was empty. */}
      {addPointsSlot}
    </div>
  );
}

const firstSectionLabelStyle: CSSProperties = {
  ...sectionHeaderStyle,
  borderTop: "none",
};

const STATUS_OPTIONS: WorkItemStatus[] = [
  "waiting", "ready", "in_progress", "human_check", "blocked", "done", "canceled",
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
  isExpanded,
  isSelected,
  isOver,
  isDragging,
  scopeBatchId,
  onToggleExpand,
  onSelect,
  onUpdateWorkItem,
  onRequestDelete,
  onContextMenu,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
  renameRequest,
}: {
  rowKey: string;
  item: WorkItem;
  isExpanded: boolean;
  isSelected: boolean;
  isOver: boolean;
  isDragging: boolean;
  scopeBatchId: string | null;
  onToggleExpand(id: string): void;
  onSelect?(id: string): void;
  onUpdateWorkItem: (itemId: string, changes: WorkItemDetailChanges) => Promise<void>;
  onRequestDelete(item: WorkItem): void;
  onContextMenu(event: React.MouseEvent, item: WorkItem): void;
  onDragStart(event: React.DragEvent): void;
  onDragEnd(event: React.DragEvent): void;
  onDragOver(event: React.DragEvent): void;
  onDragLeave(event: React.DragEvent): void;
  onDrop(event: React.DragEvent): void;
  renameRequest?: { itemId: string; nonce: number } | null;
}) {
  // null = not editing; "" valid draft during edit
  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  // Ref (not state) so Escape's handler can set it synchronously and the
  // blur handler that fires on the same tick sees the update — matches
  // WorkItemDetail's EditableField, where useState raced the blur.
  const cancelRequested = useRef(false);

  useEffect(() => { setTitleDraft(null); }, [item.id]);

  useEffect(() => {
    if (!renameRequest || renameRequest.itemId !== item.id) return;
    if (item.status === "in_progress") return;
    setTitleDraft(item.title);
  }, [renameRequest?.nonce, renameRequest?.itemId, item.id, item.status, item.title]);

  const editing = titleDraft !== null;
  const dimmed = item.status === "done" || item.status === "canceled";
  const locked = item.status === "in_progress";

  const commitTitle = () => {
    if (titleDraft === null) return;
    if (cancelRequested.current) {
      cancelRequested.current = false;
      setTitleDraft(null);
      return;
    }
    const next = titleDraft.trim();
    setTitleDraft(null);
    if (next && next !== item.title) {
      void onUpdateWorkItem(item.id, { title: next });
    }
  };

  // scopeBatchId isn't used directly here, but the outer drag handler that
  // encoded it into dataTransfer was captured at onDragStart creation time —
  // suppress the unused-parameter lint without plumbing it away.
  void scopeBatchId;

  return (
    <div>
      <div
        draggable={!locked && !editing}
        onDragStart={locked || editing ? undefined : onDragStart}
        onDragEnd={onDragEnd}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
        onClick={() => {
          if (editing) return;
          onSelect?.(item.id);
          onToggleExpand(item.id);
        }}
        onContextMenu={(event) => onContextMenu(event, item)}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          padding: "4px 8px",
          cursor: isDragging ? "grabbing" : "pointer",
          borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
          borderLeft: isSelected ? "2px solid var(--accent)" : "2px solid transparent",
          background: isExpanded
            ? "var(--bg-detail)"
            : isSelected
              ? "rgba(74,158,255,0.12)"
              : isDragging
                ? "rgba(255,255,255,0.04)"
                : "transparent",
          fontSize: 12,
          userSelect: editing ? "text" : "none",
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
        {editing ? (
          <input
            autoFocus
            value={titleDraft ?? ""}
            onChange={(event) => setTitleDraft(event.target.value)}
            onClick={(event) => event.stopPropagation()}
            onBlur={commitTitle}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                (event.target as HTMLElement).blur();
              } else if (event.key === "Escape") {
                event.preventDefault();
                cancelRequested.current = true;
                (event.target as HTMLElement).blur();
              }
            }}
            style={{
              ...inputStyle,
              flex: 1,
              minWidth: 0,
              padding: "2px 4px",
            }}
          />
        ) : (
          <span
            onClick={(event) => {
              event.stopPropagation();
              setTitleDraft(item.title);
            }}
            style={{
              flex: 1,
              minWidth: 0,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontWeight: isExpanded ? 600 : 400,
            }}
            title="Click to rename"
          >
            {item.title}
          </span>
        )}
        <InlinePriorityPicker
          priority={item.priority}
          onChange={(priority) => { void onUpdateWorkItem(item.id, { priority }); }}
        />
      </div>
      {isExpanded ? (
        <WorkItemDetail item={item} onUpdateWorkItem={onUpdateWorkItem} onRequestDelete={() => onRequestDelete(item)} />
      ) : null}
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
      title={`Status: ${status} — click to change`}
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
          <option key={option} value={option}>{option}</option>
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
