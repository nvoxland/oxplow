import type { CommitPoint } from "../../api.js";
import { deleteCommitPoint } from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { miniButtonStyle } from "./plan-utils.js";
import { commitBadgeStyle, queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a commit-point divider — shows the currently-proposed
 * message (if the agent has drafted one) and a Delete button. Approval
 * now happens in chat: the agent drafts a message, outputs it in its
 * reply, and waits for the user to say "approve" (which triggers the
 * agent to run the actual commit). The row is read-only here; there are
 * no Approve / Reject / Edit controls to click.
 */
export function CommitPointRow({ cp }: { cp: CommitPoint }) {
  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={commitBadgeStyle(cp.status)}>{cp.status}</span>
        <span style={{ color: "var(--muted)", fontSize: 11, flex: 1 }}>
          {cp.status === "proposed"
            ? "Drafted in chat — reply to the agent to approve or revise."
            : cp.status === "pending"
              ? "Waiting for the agent to reach this point and draft a message."
              : "Committed."}
        </span>
        {cp.status !== "done" ? (
          <button
            type="button"
            style={miniButtonStyle}
            onClick={() => runWithError("Delete commit point", deleteCommitPoint(cp.id))}
          >Delete</button>
        ) : null}
      </div>
      {cp.commit_sha ? (
        <div style={{ fontFamily: "ui-monospace, monospace", fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
          committed {cp.commit_sha.slice(0, 8)}
        </div>
      ) : null}
      {cp.proposed_message ? (
        <div style={{ marginTop: 6 }}>
          <div style={{ fontSize: 11, color: "var(--muted)", marginBottom: 4 }}>Drafted message:</div>
          <pre style={{
            margin: 0,
            padding: 6,
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: 4,
            fontFamily: "ui-monospace, monospace",
            fontSize: 11,
            whiteSpace: "pre-wrap",
          }}>{cp.proposed_message}</pre>
        </div>
      ) : null}
    </div>
  );
}
