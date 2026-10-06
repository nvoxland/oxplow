/// The status bar's bell: how many things need the person, red
/// while something failed, the accent while decisions or notices wait,
/// quiet when nothing does. Opens the Alerts page.

import { Bell } from "lucide-react";

import { alertsRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { alertsSummary } from "./alertsModel.js";
import { useAlerts } from "./useAlerts.js";

export function AlertsIndicator({ onOpenPage }: { onOpenPage(ref: TabRef): void }) {
  const { items } = useAlerts();
  const { count, tone } = alertsSummary(items);
  const color = tone === "danger" ? "var(--severity-critical)" : tone === "accent" ? "var(--accent)" : "var(--text-muted)";
  const title = count === 0 ? "Nothing needs you" : `${count} thing${count === 1 ? "" : "s"} need${count === 1 ? "s" : ""} you — open Alerts`;
  return (
    <button
      type="button"
      data-testid="alerts-indicator"
      data-tone={tone}
      title={title}
      aria-label={title}
      onClick={() => onOpenPage(alertsRef())}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        height: 22,
        padding: "2px 6px",
        background: "transparent",
        border: "none",
        borderRadius: 4,
        color,
        fontSize: 11,
        fontWeight: 600,
        cursor: "pointer",
      }}
    >
      <Bell size={13} aria-hidden />
      {count > 0 ? <span data-testid="alerts-count">{count}</span> : null}
    </button>
  );
}
