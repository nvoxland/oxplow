/// The Alerts dock panel: what needs the person, each with its Review
/// action, and a link to the Alerts page. The icon's badge is the same
/// count the bell showed (`alertsSummary`).

import { alertsRef, refFromTabId } from "../../tabs/pageRefs.js";
import type { DockBadge, DockBodyProps } from "../Dock/dockPanels.js";
import { alertsSummary, newAlerts } from "./alertsModel.js";
import { useAlerts } from "./useAlerts.js";

export function useAlertsBadge(): DockBadge | null {
  const { items } = useAlerts();
  const { count, tone } = alertsSummary(items);
  return count > 0 ? { count, ...(tone === "danger" ? { tone } : {}) } : null;
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
          <div key={r.key} data-testid="alerts-panel-item" style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ flex: 1, minWidth: 0 }}>{r.message}</span>
            <button
              type="button"
              data-testid="alerts-panel-review"
              onClick={() => onOpenPage((r.open ? refFromTabId(r.open) : null) ?? alertsRef())}
              style={linkButton}
            >
              Review
            </button>
          </div>
        ))
      )}
      <div>
        <button type="button" data-testid="alerts-panel-open-page" onClick={() => onOpenPage(alertsRef())} style={linkButton}>
          Open Alerts page
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
