/// The Board (P6.E1a): work items as cards in one column per canonical
/// state (`workItems.ts`), live through `useRerunOnChange`. Drag a card to
/// a column, or right-click it → Move To, to transition it through its
/// provider (`transitionWorkItem`). Every card opens its item's page.
/// Drop targets highlight while a card is over them.
import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { RouteLink } from "../../tabs/RouteLink.js";
import { workItemTabRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import type { Reads } from "../../tauri-bridge/generated/bindings.js";
import {
  CANONICAL_STATES,
  boardColumns,
  readWorkItems,
  STATE_LABEL,
  transitionWorkItem,
  type CanonicalState,
  type WorkItem,
  type WorkItemScope,
} from "../../workItems.js";
import { personCommands } from "../../personCommands.js";
import { recordOpError } from "../opErrorsStore.js";
import { uiCommandMenuItems, uiCommandsAbout } from "../uiCommands.js";
import { useUiCommands } from "../useUiCommands.js";
import { useContextMenu } from "../useRowContextMenu.js";


export function WorkBoard({ scope, onOpenPage }: { scope: WorkItemScope; onOpenPage?(ref: TabRef): void }) {
  const [items, setItems] = useState<WorkItem[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [over, setOver] = useState<CanonicalState | null>(null);
  const ctxMenu = useContextMenu();
  const uiCommands = useUiCommands(null);
  const scopeKey = JSON.stringify(scope);
  const refresh = useCallback(async () => {
    try {
      const out = await readWorkItems({ scope: JSON.parse(scopeKey) as WorkItemScope, hideArchived: true });
      setItems(out.items);
      setReads(out.reads);
    } catch (e) {
      recordOpError({ label: "Load the board", message: e instanceof Error ? e.message : String(e) });
    }
  }, [scopeKey]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());

  const move = (ref: string, state: CanonicalState) => void transitionWorkItem(ref, state);

  return (
    <div data-testid="work-board" style={boardStyle}>
      {boardColumns(items).map((col) => (
        <div
          key={col.state}
          data-testid={`board-column-${col.state}`}
          style={{ ...columnStyle, ...(over === col.state ? dropStyle : null) }}
          onDragOver={(e) => {
            if (!Array.from(e.dataTransfer.types ?? []).includes(WORK_ITEM_DRAG_MIME)) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
            setOver(col.state);
          }}
          onDragLeave={() => setOver((s) => (s === col.state ? null : s))}
          onDrop={(e) => {
            setOver(null);
            const ref = e.dataTransfer.getData(WORK_ITEM_DRAG_MIME);
            if (!ref) return;
            e.preventDefault();
            const item = items.find((i) => i.ref === ref);
            if (item && item.state !== col.state) void move(ref, col.state);
          }}
        >
          <h3 style={headStyle}>
            {STATE_LABEL[col.state]} <span style={countStyle}>{col.items.length}</span>
          </h3>
          {col.items.map((item) => {
            return (
              <div
                key={item.ref}
                data-testid="board-card"
                tabIndex={0}
                draggable
                style={cardStyle}
                onDragStart={(e) => {
                  e.dataTransfer.setData(WORK_ITEM_DRAG_MIME, item.ref);
                  e.dataTransfer.effectAllowed = "move";
                }}
                onContextMenu={(e) =>
                  ctxMenu.open(e, [
                    ...CANONICAL_STATES.filter((s) => s !== item.state).map((s) => ({
                      id: `board-move-${s}`,
                      label: `Move to ${STATE_LABEL[s]}`,
                      enabled: true,
                      run: () => void move(item.ref, s),
                    })),
                    // Extensions' commands for the item (P6b.C4).
                    ...uiCommandMenuItems(uiCommandsAbout(uiCommands, item.ref, "context"), item.ref, (c, input) =>
                      void personCommands.run(c.label, c.command, input),
                    ),
                  ])
                }
              >
                <RouteLink
                  to={workItemTabRef(item.ref)}
                  onNavigate={onOpenPage ? () => onOpenPage(workItemTabRef(item.ref)) : undefined}
                  style={titleLinkStyle}
                >
                  {item.title}
                </RouteLink>
                <div style={metaStyle}>
                  {item.provider === "oxplow" ? item.task?.priority : `${item.provider} · ${item.nativeState}`}
                  {item.task && item.task.noteCount > 0 ? ` · ${item.task.noteCount} notes` : ""}
                </div>
              </div>
            );
          })}
        </div>
      ))}
      {ctxMenu.menu}
    </div>
  );
}

const boardStyle: CSSProperties = { display: "flex", gap: 10, alignItems: "flex-start", overflowX: "auto", padding: 12 };
const columnStyle: CSSProperties = {
  flex: "1 0 200px",
  minWidth: 200,
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: 8,
  borderRadius: 6,
  border: "1px solid var(--border-subtle)",
  background: "var(--surface-app)",
  minHeight: 120,
};
const dropStyle: CSSProperties = { border: "1px dashed var(--accent)", boxShadow: "0 0 0 2px var(--accent-soft-bg)" };
const headStyle: CSSProperties = { margin: 0, fontSize: "var(--text-sm)" };
const countStyle: CSSProperties = { color: "var(--text-secondary)", fontWeight: 400 };
const cardStyle: CSSProperties = {
  padding: 8,
  borderRadius: 4,
  border: "1px solid var(--border-subtle)",
  background: "var(--surface-card)",
  fontSize: "var(--text-sm)",
  cursor: "grab",
};
const titleLinkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-primary)",
  cursor: "pointer",
  textAlign: "left",
  font: "inherit",
};
const metaStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginTop: 4 };
