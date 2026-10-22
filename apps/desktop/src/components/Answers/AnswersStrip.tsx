/// The Answers strip (P6.C2, target §13.3.1): beside a terminal thread's
/// agent, the lenses it showed with `oxplow_show_lens`, newest first, each live
/// with Keep This. Collapsible — Escape inside it collapses it — and
/// remembered in the session tab's history state. Above them, what the agent asked for that waits
/// for the person (P9.A3): its pending proposals, never collapsed. Hidden
/// while the thread has neither.
import type { CSSProperties } from "react";

import { useHistoryState } from "../../tabs/useHistoryState.js";
import { useThreadProposals } from "../../proposals.js";
import type { TabRef } from "../../tabs/tabState.js";
import { ProposalList } from "../../kit/Approvals.js";
import { ThreadAnswer } from "./ThreadAnswer.js";
import { useThreadAnswers } from "./useThreadAnswers.js";

export function AnswersStrip({ threadId, onOpenPage }: { threadId: string; onOpenPage?(ref: TabRef): void }) {
  const answers = useThreadAnswers(threadId);
  const waiting = useThreadProposals(threadId).filter((p) => p.decision === "pending");
  const [collapsed, setCollapsed] = useHistoryState<boolean>("answersCollapsed", false);

  if (answers.length === 0 && waiting.length === 0) return null;
  return (
    <section
      data-testid="answers-strip"
      style={stripStyle}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !collapsed) setCollapsed(true);
      }}
    >
      {waiting.length === 0 ? null : (
        <div data-testid="thread-proposals" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <h2 style={headingStyle}>Waiting for you ({waiting.length})</h2>
          <ProposalList proposals={waiting} />
        </div>
      )}
      {answers.length === 0 ? null : (
        <h2 style={headingStyle}>
          <button
            type="button"
            data-testid="answers-strip-toggle"
            aria-expanded={!collapsed}
            onClick={() => setCollapsed(!collapsed)}
            style={toggleStyle}
          >
            {collapsed ? "▸" : "▾"} Answers ({answers.length})
          </button>
        </h2>
      )}
      {collapsed || answers.length === 0 ? null : (
        <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", minHeight: 0 }}>
          {answers.map((a) => (
            <ThreadAnswer key={a.uri} answer={a} onOpenPage={onOpenPage} />
          ))}
        </div>
      )}
    </section>
  );
}

const stripStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: 8,
  // The terminal keeps most of the column (tsk1042).
  maxHeight: "35%",
  minHeight: 0,
  borderBottom: "1px solid var(--border-subtle)",
  background: "var(--surface-app)",
};
const headingStyle: CSSProperties = { margin: 0, fontSize: "var(--text-sm)" };
const toggleStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  font: "inherit",
  fontWeight: 600,
  color: "var(--text-primary)",
  cursor: "pointer",
};
