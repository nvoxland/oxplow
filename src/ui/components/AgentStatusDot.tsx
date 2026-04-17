import type { CSSProperties } from "react";
import type { AgentStatus } from "../api.js";

const COLORS: Record<AgentStatus, string> = {
  idle: "var(--muted)",
  working: "#fcd34d",
  waiting: "#fca5a5",
  done: "#86efac",
};

const LABELS: Record<AgentStatus, string> = {
  idle: "Idle",
  working: "Working",
  waiting: "Waiting for input",
  done: "Done",
};

export function AgentStatusDot({ status, size = 8 }: { status: AgentStatus; size?: number }) {
  const style: CSSProperties = {
    display: "inline-block",
    width: size,
    height: size,
    borderRadius: "50%",
    background: COLORS[status],
    flexShrink: 0,
    animation: status === "working" ? "newde-pulse 1.4s ease-in-out infinite" : undefined,
    boxShadow: status === "waiting" ? `0 0 0 2px rgba(252, 165, 165, 0.25)` : undefined,
  };
  return <span style={style} title={LABELS[status]} aria-label={`Agent status: ${LABELS[status]}`} />;
}
