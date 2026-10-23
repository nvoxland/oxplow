/// Above an agent session's terminal, what its thread's agents asked for
/// that waits for the person (P9.A3): the thread's pending proposals,
/// with Approve and Decline. Hidden while nothing waits. (What an agent
/// *shows* is an answer, the thread's own page: `lens/AnswerViz.tsx`.)
import type { CSSProperties } from "react";

import { useThreadProposals } from "../proposals.js";
import { ProposalList } from "../kit/Approvals.js";

export function ThreadWaiting({ threadId }: { threadId: string }) {
  const waiting = useThreadProposals(threadId).filter((p) => p.decision === "pending");
  if (waiting.length === 0) return null;
  return (
    <section data-testid="thread-proposals" style={stripStyle}>
      <h2 style={headingStyle}>Waiting for you ({waiting.length})</h2>
      <ProposalList proposals={waiting} />
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
  overflow: "auto",
  borderBottom: "1px solid var(--border-subtle)",
  background: "var(--surface-app)",
};
const headingStyle: CSSProperties = { margin: 0, fontSize: "var(--text-sm)" };
