import type { CSSProperties } from "react";
import { useState } from "react";

import { summarizeProposal, type Proposal } from "../../proposals.js";

/** One agent proposal with Approve and Decline (P6b.A4). Approving *is*
 *  the confirmation: it runs the command as the person, no further ask.
 *  A destructive one is tinted. A failed decision shows next to the
 *  buttons; the list re-reads itself when the proposal is decided. */
export function ProposalCard({
  proposal,
  compact,
  onDecide,
}: {
  proposal: Proposal;
  /** The one-line form for a Settings row (no who/children lines). */
  compact?: boolean;
  onDecide(p: Proposal, approve: boolean): Promise<void>;
}) {
  const s = summarizeProposal(proposal);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (approve: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await onDecide(proposal, approve);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const id = proposal.id;
  return (
    <div
      data-testid={`proposal-${id}`}
      style={{ ...cardStyle, borderLeftColor: s.destructive ? "var(--diff-del-fg, #f85149)" : "var(--accent)" }}
    >
      {compact ? null : (
        <div style={{ color: "var(--text-primary)", fontWeight: 600 }}>
          {s.title}
          {s.destructive ? <span style={destructiveStyle}> destructive</span> : null}
        </div>
      )}
      {compact ? null : <div style={mutedStyle}>{s.who} proposed this.</div>}
      {s.change ? (
        <div data-testid={`proposal-change-${id}`} style={changeStyle}>
          {compact ? <span style={mutedStyle}>{s.who} proposes: </span> : null}
          <span>{s.change.before}</span>
          <span style={mutedStyle}> → </span>
          <span style={{ color: "var(--text-primary)" }}>{s.change.after}</span>
        </div>
      ) : null}
      {!compact && s.children.length > 0 ? (
        <ol data-testid={`proposal-children-${id}`} style={{ margin: "2px 0 0 18px", padding: 0 }}>
          {s.children.map((c, i) => (
            <li key={i}>
              <code>{c}</code>
            </li>
          ))}
        </ol>
      ) : null}
      <div style={{ display: "flex", gap: 6, marginTop: 4, alignItems: "center" }}>
        <button
          type="button"
          data-testid={`proposal-approve-${id}`}
          disabled={busy}
          onClick={() => void run(true)}
          style={primaryButtonStyle}
        >
          Approve
        </button>
        <button
          type="button"
          data-testid={`proposal-decline-${id}`}
          disabled={busy}
          onClick={() => void run(false)}
          style={buttonStyle}
        >
          Decline
        </button>
        {error ? (
          <span data-testid={`proposal-error-${id}`} style={{ color: "var(--diff-del-fg, #f85149)" }}>
            {error}
          </span>
        ) : null}
      </div>
    </div>
  );
}

const cardStyle: CSSProperties = {
  borderLeft: "3px solid",
  padding: "4px 8px",
  fontSize: "var(--text-xs)",
  display: "flex",
  flexDirection: "column",
  gap: 2,
};
const mutedStyle: CSSProperties = { color: "var(--text-secondary)" };
const destructiveStyle: CSSProperties = { color: "var(--diff-del-fg, #f85149)", fontWeight: 500 };
const changeStyle: CSSProperties = { fontFamily: "var(--font-mono)", wordBreak: "break-word" };
const buttonStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  padding: "1px 8px",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  background: "transparent",
  color: "var(--text-primary)",
  cursor: "pointer",
};
const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  borderColor: "var(--accent)",
  color: "var(--accent-fg, #fff)",
};
