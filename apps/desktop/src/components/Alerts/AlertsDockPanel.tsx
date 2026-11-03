/// The Alerts dock panel: what needs the person, each led by Review (the
/// open arrow: it opens the item's page, else the Alerts page), and Clear at the foot for what a person can put away (the hints
/// and the failed operations; `clearable`); the Alerts page is the panel
/// header's arrow. The icon's badge is the same count the bell showed
/// (`alertsSummary`).

import { SquareArrowRightEnter } from "lucide-react";

import { dismissHint } from "../../hints.js";
import { dismissAllOpErrors } from "../../opErrors.js";
import { alertsRef, refFromTabId } from "../../tabs/pageRefs.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { dockButtonStyle } from "../Dock/dockStyle.js";
import { alertsSummary, clearable, newAlerts, type AlertItems } from "./alertsModel.js";
import { useAlerts } from "./useAlerts.js";

export function useAlertsBadge(): DockBadge | null {
  const { items } = useAlerts();
  const { count, tone } = alertsSummary(items);
  return count > 0 ? { count, ...(tone === "danger" ? { tone } : {}) } : null;
}

/** Put away every hint and every failed operation at once. */
async function clearAlerts(items: AlertItems): Promise<void> {
  await Promise.all([...(items.opErrors.length > 0 ? [dismissAllOpErrors()] : []), ...items.hints.map((h) => dismissHint(h.id))]);
}

export function AlertsDockBody({ onOpenPage }: DockBodyProps) {
  const { items } = useAlerts();
  const rows = newAlerts(new Set(), items);
  return (
    <div data-testid="alerts-panel" style={{ display: "flex", flexDirection: "column", gap: 6, padding: "8px 12px", fontSize: "var(--text-xs)" }}>
      {rows.length === 0 ? (
        <div data-testid="alerts-panel-empty" style={{ color: "var(--text-muted)" }}>Nothing needs you.</div>
      ) : (
        rows.map((r) => (
          <div key={r.key} data-testid="alerts-panel-item" style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
            <button
              type="button"
              data-testid="alerts-panel-review"
              aria-label="Review"
              title="Review"
              onClick={() => onOpenPage((r.open ? refFromTabId(r.open) : null) ?? alertsRef())}
              style={iconButton}
            >
              <SquareArrowRightEnter size={14} aria-hidden />
            </button>
            <span style={{ flex: 1, minWidth: 0 }}>{r.message}</span>
          </div>
        ))
      )}
      {clearable(items) ? (
        <div style={{ display: "flex", marginTop: 2 }}>
          <button type="button" className="ox-button" style={dockButtonStyle} data-testid="alerts-panel-clear" title="Dismiss the hints and the failed operations" onClick={() => void clearAlerts(items)}>
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
  color: "var(--accent)",
  cursor: "pointer",
  display: "inline-flex",
} as const;
