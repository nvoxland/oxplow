import { useMemo, useState } from "react";
import type { CommitPoint, WaitPoint, WorkItem } from "../../api.js";
import { WORK_ITEM_DRAG_MIME } from "../BatchRail.js";
import { CommitPointRow } from "./CommitPointRow.js";
import {
  groupHeaderStyle,
  priorityIcon,
  priorityStyle,
  statusIcon,
  type WorkItemGroup,
} from "./plan-utils.js";
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
 * with no epic). For the root group, optional `commitPoints` and
 * `waitPoints` are interleaved into the queue by `sort_index` and rendered
 * as horizontal dividers; clicking a divider expands its
 * CommitPointRow / WaitPointRow inline. The whole list is drag-reorderable
 * via the same DnD machinery for both items and markers — work items use
 * the existing scope-aware reorder callback, mixed lists go through
 * `onReorderMixed`.
 */
export type QueueRow =
  | { kind: "work"; id: string; sortIndex: number; item: WorkItem }
  | { kind: "commit"; id: string; sortIndex: number; cp: CommitPoint }
  | { kind: "wait"; id: string; sortIndex: number; wp: WaitPoint };

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
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);

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
      onReorderMixed(next.map((row) => ({ kind: row.kind, id: row.id })));
    } else {
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
                background: isExpanded ? "var(--bg-detail)" : isDragging ? "rgba(255,255,255,0.04)" : "transparent",
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
