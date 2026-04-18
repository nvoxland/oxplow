import type { CSSProperties } from "react";
import type { AgentStatus, Batch, BatchWorkState } from "../api.js";
import { AgentStatusDot } from "./AgentStatusDot.js";

interface Props {
  batch: Batch | null;
  activeBatchId: string | null;
  agentStatus: AgentStatus;
  batchWork: BatchWorkState | null;
  turnCount: number | null;
  onPromote(): void;
  onComplete(): void;
}

export function BatchStatusBar({ batch, activeBatchId, agentStatus, batchWork, turnCount, onPromote, onComplete }: Props) {
  if (!batch) {
    return (
      <div style={{ ...barStyle, color: "var(--muted)" }}>No batch selected.</div>
    );
  }
  const isActive = batch.id === activeBatchId;
  const total = batchWork?.items.length ?? 0;
  const done = batchWork?.done.length ?? 0;
  const percent = total === 0 ? 0 : Math.round((done / total) * 100);
  return (
    <div style={barStyle}>
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
        <AgentStatusDot status={agentStatus} />
        <strong style={{ color: "var(--fg)" }}>{batch.title}</strong>
      </span>
      <span>
        {isActive ? "active" : "queued"}
      </span>
      <span>
        {done}/{total} items · {percent}%
      </span>
      {turnCount != null ? <span>turns: {turnCount}</span> : null}
      <span style={{ color: "var(--muted)" }}>agent: {agentStatus}</span>
      <span style={{ flex: 1 }} />
      {isActive ? (
        <button style={buttonStyle} onClick={onComplete}>Complete batch</button>
      ) : (
        <button style={buttonStyle} onClick={onPromote}>Promote to active</button>
      )}
    </div>
  );
}

const barStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 10,
  padding: "4px 10px",
  borderBottom: "1px solid var(--border)",
  fontSize: 11,
  background: "var(--bg-2)",
};

const buttonStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "2px 8px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
};
