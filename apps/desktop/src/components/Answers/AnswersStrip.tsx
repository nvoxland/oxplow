/// The Answers strip (P6.C2, target §13.3.1): beside a terminal thread's
/// agent, the lenses it showed with `show_lens`, newest first, each live
/// with Keep This. Collapsible — Escape inside it collapses it — and
/// remembered per thread. Above them, what the agent asked for that waits
/// for the person (P9.A3): its pending proposals, never collapsed. Hidden
/// while the thread has neither.
import type { CSSProperties } from "react";
import { useEffect, useState } from "react";

import { decide, useThreadProposals } from "../../proposals.js";
import type { TabRef } from "../../tabs/tabState.js";
import { ProposalCard } from "../Proposals/ProposalCard.js";
import { ThreadAnswer } from "./ThreadAnswer.js";
import { useThreadAnswers } from "./useThreadAnswers.js";

const COLLAPSED_KEY = "oxplow.answers.collapsed";

function readCollapsed(threadId: string): boolean {
  try {
    const all = JSON.parse(window.localStorage.getItem(COLLAPSED_KEY) ?? "{}") as Record<string, boolean>;
    return all[threadId] === true;
  } catch {
    return false;
  }
}

function writeCollapsed(threadId: string, collapsed: boolean): void {
  try {
    const all = JSON.parse(window.localStorage.getItem(COLLAPSED_KEY) ?? "{}") as Record<string, boolean>;
    if (collapsed) all[threadId] = true;
    else delete all[threadId];
    window.localStorage.setItem(COLLAPSED_KEY, JSON.stringify(all));
  } catch {
    // Storage full or disabled: the toggle still works for this session.
  }
}

export function AnswersStrip({ threadId, onOpenPage }: { threadId: string; onOpenPage?(ref: TabRef): void }) {
  const answers = useThreadAnswers(threadId);
  const waiting = useThreadProposals(threadId).filter((p) => p.decision === "pending");
  const [collapsed, setCollapsedState] = useState(() => readCollapsed(threadId));
  useEffect(() => setCollapsedState(readCollapsed(threadId)), [threadId]);
  const setCollapsed = (next: boolean) => {
    setCollapsedState(next);
    writeCollapsed(threadId, next);
  };

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
          {waiting.map((p) => (
            <ProposalCard key={p.id} proposal={p} onDecide={decide} />
          ))}
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
            <ThreadAnswer key={a.ref} answer={a} onOpenPage={onOpenPage} />
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
