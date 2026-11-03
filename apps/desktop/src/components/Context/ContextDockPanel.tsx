/// The Context dock panel: what the person collected for the selected
/// thread's agents and not yet sent, each with a × to take it out, Clear,
/// and a link to the Context page. The icon's badge is how many wait.

import { FileText, Quote, X } from "lucide-react";

import { clearContext, removeFromContext, useBasket, useSelectedThreadId, waiting, type BasketItem } from "../../contextBasket.js";
import { contextRef } from "../../tabs/pageRefs.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";

export function useContextBadge(): DockBadge | null {
  const count = waiting(useBasket(useSelectedThreadId())).length;
  return count > 0 ? { count } : null;
}

/** What a row says: the label, else the source page, else the text's start. */
export function itemTitle(item: BasketItem): string {
  return item.label ?? item.sourceUri ?? (item.text ?? "").slice(0, 80);
}

export function ContextDockBody({ onOpenPage }: DockBodyProps) {
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
            {item.kind === "text" ? <Quote size={14} aria-hidden /> : <FileText size={14} aria-hidden />}
            <span style={{ flex: 1, minWidth: 0 }}>
              <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{itemTitle(item)}</div>
              {item.kind === "text" && item.text ? (
                <div style={{ color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{item.text.slice(0, 200)}</div>
              ) : null}
            </span>
            <button type="button" data-testid="context-panel-remove" aria-label="Remove from context" title="Remove" onClick={() => void removeFromContext(item.id)} style={iconButton}>
              <X size={14} aria-hidden />
            </button>
          </div>
        ))
      )}
      <div style={{ display: "flex", gap: 12 }}>
        {items.length > 0 && thread ? (
          <button type="button" data-testid="context-panel-clear" onClick={() => void clearContext(thread)} style={linkButton}>
            Clear
          </button>
        ) : null}
        <button type="button" data-testid="context-panel-open-page" onClick={() => onOpenPage(contextRef())} style={linkButton}>
          Open Context page
        </button>
      </div>
    </div>
  );
}

const linkButton = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  fontSize: "inherit",
} as const;

const iconButton = { ...linkButton, color: "var(--text-muted)", display: "inline-flex" } as const;
