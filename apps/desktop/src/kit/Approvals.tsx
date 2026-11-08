import type { CSSProperties } from "react";
import { useState } from "react";

import { useActionAnchor, useRunner } from "../commands/RunnerContext.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import { configUri, idOf } from "../uri/uri.js";
import { decide, sourceOf, summarizeProposal, useProposals, type Proposal } from "../proposals.js";
import { SourceChip } from "./SourceChip.js";
import { InlineError } from "./InlineError.js";

/** The pending proposals an agent raised, read here and decided through
 *  the person-only path (`decideProposal`): the one place they are drawn.
 *  `thread` (a `thread:thr3` ref or `thr3`) and `setting` (a config key)
 *  narrow the list; it draws nothing while none waits. The `approvals`
 *  a viz renders this; the built-in pages that read their proposals
 *  already draw them with [`ProposalList`]. */
export function Approvals({
  thread,
  setting,
  compact,
}: {
  thread?: string | null;
  setting?: string | null;
  compact?: boolean;
}) {
  const proposals = useProposals();
  return <ProposalList proposals={narrow(proposals, thread, setting)} compact={compact} />;
}

/** The proposals of `all` a `thread` and/or `setting` filter leaves. */
export function narrow(all: Proposal[], thread?: string | null, setting?: string | null): Proposal[] {
  const row = thread ? Number((idOf(thread, "oxplow/thread") ?? thread).replace(/^thr/, "")) : null;
  return all.filter(
    (p) => (row === null || p.threadId === row) && (!setting || p.key === configUri(setting)),
  );
}

/** `proposals` as cards, each with Approve and Decline. */
export function ProposalList({
  proposals,
  compact,
  onDecide = decide,
}: {
  proposals: Proposal[];
  compact?: boolean;
  onDecide?(p: Proposal, approve: boolean): Promise<void>;
}) {
  if (proposals.length === 0) return null;
  return (
    <div data-testid="approvals" style={{ display: "flex", flexDirection: "column", gap: compact ? 0 : 8 }}>
      {proposals.map((p) => (
        <ProposalCard key={p.id} proposal={p} compact={compact} onDecide={onDecide} />
      ))}
    </div>
  );
}

/** One agent proposal with Approve and Decline (P6b.A4). Approving *is*
 *  the confirmation: it runs the command as the person — except that a
 *  destructive one is tinted and its Approve asks inline first
 *  (`InlineConfirm`). A decision runs through the person's runner: a
 *  failure is recorded and shows next to the buttons, kept by the proposal
 *  so the card drawn again by a re-read still shows it; the list re-reads
 *  itself when the proposal is decided. */
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
  const id = proposal.id;
  const anchor = `proposal/${id}`;
  const failure = useActionAnchor(anchor);
  const runner = useRunner();
  const run = async (approve: boolean) => {
    setBusy(true);
    try {
      await runner.run(`${approve ? "Approve" : "Decline"}: ${s.title}`, () => onDecide(proposal, approve), {
        anchor,
        command: proposal.command,
        proposal: id,
      });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div
      data-testid={`proposal-${id}`}
      style={{ ...cardStyle, borderLeftColor: s.destructive ? "var(--diff-del-fg, #f85149)" : "var(--accent)" }}
    >
      {compact ? null : (
        <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <span style={{ color: "var(--text-primary)", fontWeight: 600, flex: 1 }}>
            {s.title}
            {s.destructive ? <span style={destructiveStyle}> destructive</span> : null}
          </span>
          <SourceChip source={sourceOf(proposal)} />
        </div>
      )}
      {s.change ? (
        <div data-testid={`proposal-change-${id}`} style={changeStyle}>
          {compact ? (
            <span style={{ marginRight: 6 }}>
              <SourceChip source={sourceOf(proposal)} />
            </span>
          ) : null}
          <span>{s.change.before}</span>
          <span style={mutedStyle}> → </span>
          <span style={{ color: "var(--text-primary)" }}>{s.change.after}</span>
        </div>
      ) : null}
      {s.details.length > 0 ? (
        <ul data-testid={`proposal-details-${id}`} style={{ margin: "2px 0 0 18px", padding: 0 }}>
          {s.details.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
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
        {s.destructive ? (
          <InlineConfirm
            triggerLabel="Approve"
            confirmLabel="Approve — it's destructive"
            testIdPrefix={`proposal-approve-${id}`}
            disabled={busy}
            triggerStyle={primaryButtonStyle}
            onConfirm={() => void run(true)}
          />
        ) : (
          <button
            type="button"
            data-testid={`proposal-approve-${id}`}
            disabled={busy}
            onClick={() => void run(true)}
            style={primaryButtonStyle}
          >
            Approve
          </button>
        )}
        <button
          type="button"
          data-testid={`proposal-decline-${id}`}
          disabled={busy}
          onClick={() => void run(false)}
          style={buttonStyle}
        >
          Decline
        </button>
        {failure ? <InlineError failure={failure} testId={`proposal-error-${id}`} /> : null}
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
