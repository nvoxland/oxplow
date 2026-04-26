import type { ReactNode } from "react";
import { useState } from "react";
import type { AgentStatus } from "../../api.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";

export interface CenterTab {
  id: string;
  label: string;
  closable: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
  /** Right-click menu items for this tab. When present, right-clicking the
   *  tab chip opens a ContextMenu with these entries. */
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
  const [menu, setMenu] = useState<{ tabId: string; x: number; y: number } | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const menuTab = menu ? tabs.find((t) => t.id === menu.tabId) : null;
  const draggingTab = draggingId ? tabs.find((t) => t.id === draggingId) ?? null : null;
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-1)" }}>
        {tabs.map((tab) => {
          const isActive = tab.id === active?.id;
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
              onContextMenu={tab.contextMenu && tab.contextMenu.length > 0 ? (event) => {
                event.preventDefault();
                setMenu({ tabId: tab.id, x: event.clientX, y: event.clientY });
              } : undefined}
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
                padding: "8px 12px",
                background: isActive ? "var(--bg)" : "transparent",
                color: isActive ? "var(--fg)" : "var(--muted)",
                borderRight: "1px solid var(--border)",
                borderBottom: isActive
                  ? "2px solid var(--accent)"
                  : isDropTarget
                    ? "2px dashed var(--accent)"
                    : "2px solid transparent",
                outline: isDropTarget ? "1px dashed var(--accent)" : "none",
                outlineOffset: isDropTarget ? -2 : 0,
                opacity: draggingId === tab.id ? 0.5 : 1,
                cursor: canDrag ? "grab" : "pointer",
                fontSize: 12,
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
              <span>{tab.label}</span>
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
      {menu && menuTab?.contextMenu ? (
        <ContextMenu
          items={menuTab.contextMenu}
          position={{ x: menu.x, y: menu.y }}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </div>
  );
}
