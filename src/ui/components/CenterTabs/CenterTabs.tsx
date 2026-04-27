import { useState, type ReactNode } from "react";
import type { AgentStatus } from "../../api.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { Kebab } from "../Kebab.js";
import type { MenuItem } from "../../menu.js";

export interface CenterTab {
  id: string;
  label: string;
  closable: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
  /** Per-tab kebab menu. When present, a `⋯` button appears on the
   *  tab chip; clicking it opens a popover with these entries.
   *  (The legacy right-click affordance was retired in phase 5 of the
   *  IA redesign — visible kebab buttons are the new primary path.)
   */
  contextMenu?: MenuItem[];
  /** Tabs that share a `reorderGroup` can be drag-reordered relative to
   *  each other. Tabs without a group are pinned (e.g. the agent tab). */
  reorderGroup?: string;
}

interface CenterTabsProps {
  tabs: CenterTab[];
  activeId: string;
  onActivate(id: string): void;
  onClose?(id: string): void;
  /** Rendered above the active tab's content. */
  header?: ReactNode;
  /** Called when the user drag-reorders tabs. Receives the new full id list. */
  onReorder?(orderedIds: string[]): void;
}

const TAB_DRAG_MIME = "application/x-oxplow-center-tab";

export function CenterTabs({ tabs, activeId, onActivate, onClose, header, onReorder }: CenterTabsProps) {
  const active = tabs.find((t) => t.id === activeId) ?? tabs[0] ?? null;
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const draggingTab = draggingId ? tabs.find((t) => t.id === draggingId) ?? null : null;
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border-strong)", background: "var(--surface-tab-inactive)", minHeight: 36 }}>
        {tabs.map((tab) => {
          const isActive = tab.id === active?.id;
          const isHover = !isActive && hoverId === tab.id;
          const canDrag = !!onReorder && !!tab.reorderGroup;
          const isDropTarget =
            !!draggingTab &&
            !!tab.reorderGroup &&
            tab.reorderGroup === draggingTab.reorderGroup &&
            overId === tab.id &&
            draggingId !== tab.id;
          return (
            <div
              key={tab.id}
              data-testid={`center-tab-${tab.id}`}
              draggable={canDrag}
              onClick={() => onActivate(tab.id)}
              onMouseEnter={() => setHoverId(tab.id)}
              onMouseLeave={() => setHoverId((prev) => (prev === tab.id ? null : prev))}
              onDragStart={canDrag ? (event) => {
                event.dataTransfer.setData(TAB_DRAG_MIME, tab.id);
                event.dataTransfer.effectAllowed = "move";
                setDraggingId(tab.id);
              } : undefined}
              onDragEnd={canDrag ? () => {
                setDraggingId(null);
                setOverId(null);
              } : undefined}
              onDragOver={onReorder ? (event) => {
                if (!draggingTab) return;
                if (!tab.reorderGroup || tab.reorderGroup !== draggingTab.reorderGroup) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (overId !== tab.id) setOverId(tab.id);
              } : undefined}
              onDragLeave={onReorder ? () => {
                if (overId === tab.id) setOverId(null);
              } : undefined}
              onDrop={onReorder ? (event) => {
                if (!draggingTab) return;
                if (!tab.reorderGroup || tab.reorderGroup !== draggingTab.reorderGroup) return;
                event.preventDefault();
                const sourceId = draggingTab.id;
                const targetId = tab.id;
                setDraggingId(null);
                setOverId(null);
                if (sourceId === targetId) return;
                const ids = tabs.map((t) => t.id);
                const fromIdx = ids.indexOf(sourceId);
                const toIdx = ids.indexOf(targetId);
                if (fromIdx < 0 || toIdx < 0) return;
                const next = ids.slice();
                const [moved] = next.splice(fromIdx, 1);
                next.splice(toIdx, 0, moved);
                onReorder(next);
              } : undefined}
              style={{
                padding: "10px 14px",
                background: isActive
                  ? "var(--surface-tab-active)"
                  : isHover
                    ? "var(--surface-card)"
                    : "transparent",
                color: isActive ? "var(--accent)" : "var(--text-secondary)",
                borderRight: "1px solid var(--border-strong)",
                borderTop: isActive ? "1px solid var(--border-strong)" : "1px solid transparent",
                borderLeft: isActive ? "1px solid var(--border-strong)" : "1px solid transparent",
                borderBottom: isActive
                  ? "3px solid var(--accent)"
                  : isDropTarget
                    ? "3px dashed var(--accent)"
                    : "3px solid transparent",
                outline: isDropTarget ? "1px dashed var(--accent)" : "none",
                outlineOffset: isDropTarget ? -2 : 0,
                opacity: draggingId === tab.id ? 0.5 : 1,
                cursor: canDrag ? "grab" : "pointer",
                fontSize: 13,
                fontWeight: isActive ? 600 : 400,
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
              <span>{tab.label}</span>
              {tab.contextMenu && tab.contextMenu.length > 0 ? (
                <span onClick={(e) => e.stopPropagation()}>
                  <Kebab items={tab.contextMenu} testId={`center-tab-kebab-${tab.id}`} size={14} />
                </span>
              ) : null}
              {tab.closable && onClose ? (
                <button type="button"
                  data-testid={`center-tab-close-${tab.id}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.id);
                  }}
                  title={`Close ${tab.label}`}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--muted)",
                    cursor: "pointer",
                    padding: "0 2px",
                    fontSize: 14,
                    lineHeight: 1,
                  }}
                >
                  ×
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
      {header}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
        {active ? active.render() : null}
      </div>
    </div>
  );
}
