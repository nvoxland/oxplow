import type { CommitPoint } from "../../api.js";
import { deleteCommitPoint } from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { miniButtonStyle } from "./plan-utils.js";
import { commitBadgeStyle, queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a commit-point divider. Drafts live in chat now, so the
 * row is just a status line plus a Delete button; once committed, the sha
 * is shown.
 */
export function CommitPointRow({ cp }: { cp: CommitPoint }) {
  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={commitBadgeStyle(cp.status)}>{cp.status}</span>
        <span style={{ color: "var(--muted)", fontSize: 11, flex: 1 }}>
          {cp.status === "pending"
            ? "Waiting for the agent to reach this point."
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
    </div>
  );
}
