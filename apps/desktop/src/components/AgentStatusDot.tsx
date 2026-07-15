import type { CSSProperties } from "react";

/// View-state enum for the small per-agent status indicator. Mapped
/// from the bindings `AgentStatusState` ("running" → "working",
/// "awaiting_user" → "awaiting", "stalled" → "stalled") at the call
/// site. Keeping the dot's alphabet narrow lets future state names
/// (idle/error/etc.) be distinguished or hidden without touching
/// every dot caller.
export type AgentStatusDotState = "working" | "waiting" | "stalled" | "awaiting";

const COLORS: Record<AgentStatusDotState, string> = {
  working: "#fcd34d",
  waiting: "#fca5a5",
  stalled: "#ef4444",
  // Sky blue: "you're on the clock" — distinct from working's yellow
  // and waiting's washed-out red, so a thread parked on YOUR answer
  // reads at a glance across streams.
  awaiting: "#38bdf8",
};

const LABELS: Record<AgentStatusDotState, string> = {
  working: "Working",
  waiting: "Waiting for input",
  stalled: "Agent exited or errored mid-turn — re-run (uncommitted work may be unsaved)",
  awaiting: "Waiting on your answer",
};

export function AgentStatusDot({
  status,
  size = 8,
  question,
}: {
  status: AgentStatusDotState;
  size?: number;
  /// The await_user question, shown as the tooltip when `status` is
  /// "awaiting". Falls back to the generic label when absent (e.g. the
  /// stream-level aggregate dot, which has no single question).
  question?: string;
}) {
  const style: CSSProperties = {
    display: "inline-block",
    width: size,
    height: size,
    borderRadius: "50%",
    background: COLORS[status],
    flexShrink: 0,
    // Everything but plain waiting pulses — awaiting/working/stalled all
    // want the eye. Waiting is the resting state.
    animation: status !== "waiting" ? "oxplow-pulse 1.4s ease-in-out infinite" : undefined,
    boxShadow:
      status === "waiting"
        ? `0 0 0 2px rgba(252, 165, 165, 0.25)`
        : status === "stalled"
          ? `0 0 0 2px rgba(239, 68, 68, 0.35)`
          : status === "awaiting"
            ? `0 0 0 2px rgba(56, 189, 248, 0.35)`
            : undefined,
  };
  const tooltip =
    status === "awaiting" && question && question.trim().length > 0
      ? `Waiting on your answer: ${question.trim()}`
      : LABELS[status];
  return (
    <span
      style={style}
      title={tooltip}
      aria-label={`Agent status: ${tooltip}`}
      data-agent-status={status}
      data-agent-label={tooltip}
    />
  );
}
