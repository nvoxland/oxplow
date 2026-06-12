import type { CSSProperties } from "react";

/// View-state enum for the small per-agent status indicator. Mapped
/// from the bindings `AgentStatusState` ("running" → "working",
/// "awaiting_user" → "waiting", "stalled" → "stalled") at the call
/// site. Keeping the dot's alphabet narrow lets future state names
/// (idle/error/etc.) be distinguished or hidden without touching
/// every dot caller.
export type AgentStatusDotState = "working" | "waiting" | "stalled";

const COLORS: Record<AgentStatusDotState, string> = {
  working: "#fcd34d",
  waiting: "#fca5a5",
  stalled: "#ef4444",
};

const LABELS: Record<AgentStatusDotState, string> = {
  working: "Working",
  waiting: "Waiting for input",
  stalled: "Stalled — agent stopped responding mid-turn",
};

export function AgentStatusDot({
  status,
  size = 8,
}: {
  status: AgentStatusDotState;
  size?: number;
}) {
  const style: CSSProperties = {
    display: "inline-block",
    width: size,
    height: size,
    borderRadius: "50%",
    background: COLORS[status],
    flexShrink: 0,
    // Stalled pulses too — it's the "look at me, something broke"
    // state, not ordinary waiting.
    animation: status !== "waiting" ? "oxplow-pulse 1.4s ease-in-out infinite" : undefined,
    boxShadow:
      status === "waiting"
        ? `0 0 0 2px rgba(252, 165, 165, 0.25)`
        : status === "stalled"
          ? `0 0 0 2px rgba(239, 68, 68, 0.35)`
          : undefined,
  };
  return (
    <span
      style={style}
      title={LABELS[status]}
      aria-label={`Agent status: ${LABELS[status]}`}
      data-agent-status={status}
      data-agent-label={LABELS[status]}
    />
  );
}
