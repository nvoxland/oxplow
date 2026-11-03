/// The Context dock panel: what the person collected for the selected
/// thread's agents and not yet sent, each led by a × to take it out, and
/// Clear, a button at the foot as the Alerts panel's; the Context page is the panel header's arrow. The icon's badge
/// is how many wait.

import { FileText, Quote, X } from "lucide-react";

import { clearContext, removeFromContext, useBasket, useSelectedThreadId, waiting, type BasketItem } from "../../contextBasket.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { dockButtonStyle } from "../Dock/dockStyle.js";

export function useContextBadge(): DockBadge | null {
  const count = waiting(useBasket(useSelectedThreadId())).length;
  return count > 0 ? { count } : null;
}

/** What a row says: the label, else the source page, else the text's start. */
export function itemTitle(item: BasketItem): string {
  return item.label ?? item.sourceUri ?? (item.text ?? "").slice(0, 80);
}

export function ContextDockBody(_props: DockBodyProps) {
  const thread = useSelectedThreadId();
  const items = waiting(useBasket(thread));
  return (
    <div data-testid="context-panel" style={{ display: "flex", flexDirection: "column", gap: 6, padding: "8px 12px", fontSize: "var(--text-xs)" }}>
      {items.length === 0 ? (
        <div data-testid="context-panel-empty" style={{ color: "var(--text-muted)" }}>
          No context added yet
        </div>
      ) : (
        items.map((item) => (
          <div key={item.id} data-testid="context-panel-item" style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
            <button type="button" data-testid="context-panel-remove" aria-label="Remove from context" title="Remove" onClick={() => void removeFromContext(item.id)} style={iconButton}>
              <X size={14} aria-hidden />
            </button>
            {item.kind === "text" ? <Quote size={14} aria-hidden /> : <FileText size={14} aria-hidden />}
            <span style={{ flex: 1, minWidth: 0 }}>
              <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{itemTitle(item)}</div>
              {item.kind === "text" && item.text ? (
                <div style={{ color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{item.text.slice(0, 200)}</div>
              ) : null}
            </span>
          </div>
        ))
      )}
      {items.length > 0 && thread ? (
        <div style={{ display: "flex", marginTop: 2 }}>
          <button type="button" className="ox-button" style={dockButtonStyle} data-testid="context-panel-clear" title="Take everything out" onClick={() => void clearContext(thread)}>
            Clear
          </button>
        </div>
      ) : null}
    </div>
  );
}

const iconButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-muted)",
  cursor: "pointer",
  fontSize: "inherit",
  display: "inline-flex",
} as const;
