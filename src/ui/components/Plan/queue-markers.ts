import type { CSSProperties } from "react";
import type { CommitPoint, WaitPoint } from "../../api.js";

/**
 * Shared divider styles for commit and wait points in the work-queue
 * list. Both render as a horizontal line with a colored badge in the
 * middle; the badge color encodes the marker's current status.
 */

export const commitDividerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 8px",
  borderTop: "1px solid transparent",
  userSelect: "none",
};

export const commitDividerLineStyle: CSSProperties = {
  flex: 1,
  height: 1,
  background: "var(--border-strong)",
};

export function commitDividerBadgeStyle(status: CommitPoint["status"]): CSSProperties {
  const accent = status === "proposed" ? "#d97706"
    : status === "done" ? "#10b981"
    : "#8888aa";
  return badgeStyle(accent);
}

export function waitDividerBadgeStyle(status: WaitPoint["status"]): CSSProperties {
  const accent = status === "triggered" ? "#d97706" : "#8888aa";
  return badgeStyle(accent);
}

export function commitBadgeStyle(status: CommitPoint["status"]): CSSProperties {
  const colors: Record<CommitPoint["status"], string> = {
    pending: "#6b7280",
    proposed: "#d97706",
    done: "#10b981",
  };
  return {
    fontSize: 10,
    fontFamily: "ui-monospace, monospace",
    padding: "1px 6px",
    borderRadius: 8,
    background: colors[status] + "22",
    color: colors[status],
    border: `1px solid ${colors[status]}55`,
    textTransform: "uppercase",
    letterSpacing: 0.4,
  };
}

function badgeStyle(accent: string): CSSProperties {
  return {
    fontSize: 10,
    fontFamily: "ui-monospace, monospace",
    letterSpacing: 0.4,
    textTransform: "uppercase",
    padding: "2px 8px",
    borderRadius: 999,
    background: accent + "22",
    color: accent,
    border: `1px solid ${accent}55`,
    flexShrink: 0,
  };
}

export const queueRowExpandedStyle: CSSProperties = {
  padding: "6px 10px",
  borderBottom: "1px solid var(--border)",
  fontSize: 12,
};
