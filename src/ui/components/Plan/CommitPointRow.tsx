import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import type { CommitPoint, CommitPointMode } from "../../api.js";
import {
  approveCommitPoint,
  deleteCommitPoint,
  rejectCommitPoint,
  resetCommitPoint,
  setCommitPointMode,
} from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { miniButtonStyle } from "./plan-utils.js";
import { commitBadgeStyle, queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a commit-point divider — shows the proposed message,
 * mode selector, and Approve / Edit / Reject / Retry / Delete actions
 * depending on status. Rendered when the user clicks a divider in the
 * queue. The compact divider itself is in WorkGroupList.
 */
export function CommitPointRow({ cp }: { cp: CommitPoint }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(cp.proposed_message ?? "");
  useEffect(() => { setDraft(cp.proposed_message ?? ""); }, [cp.proposed_message]);

  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={commitBadgeStyle(cp.status)}>{cp.status}</span>
        <select
          value={cp.mode}
          disabled={cp.status !== "pending"}
          onChange={(e) => runWithError("Change commit mode", setCommitPointMode(cp.id, e.target.value as CommitPointMode))}
          style={{ fontSize: 11, padding: "2px 4px" }}
        >
          <option value="approval">Approval</option>
          <option value="auto">Auto-commit</option>
        </select>
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          {cp.status === "rejected" ? (
            <button style={miniButtonStyle} onClick={() => runWithError("Reset commit point", resetCommitPoint(cp.id))}>Retry</button>
          ) : null}
          {cp.status !== "done" ? (
            <button style={miniButtonStyle} onClick={() => runWithError("Delete commit point", deleteCommitPoint(cp.id))}>Delete</button>
          ) : null}
        </span>
      </div>
      {cp.commit_sha ? (
        <div style={{ fontFamily: "ui-monospace, monospace", fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
          committed {cp.commit_sha.slice(0, 8)}
        </div>
      ) : null}
      {cp.status === "proposed" ? (
        <div style={{ marginTop: 6 }}>
          <div style={{ fontSize: 11, color: "var(--muted)", marginBottom: 4 }}>Proposed message (awaiting approval):</div>
          {editing ? (
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              rows={Math.min(6, Math.max(2, draft.split("\n").length))}
              style={messageEditStyle}
            />
          ) : (
            <pre style={messagePreStyle}>{cp.proposed_message}</pre>
          )}
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            {editing ? (
              <>
                <button style={miniButtonStyle} onClick={() => runWithError("Approve commit point", approveCommitPoint(cp.id, draft))}>Save & approve</button>
                <button style={miniButtonStyle} onClick={() => { setEditing(false); setDraft(cp.proposed_message ?? ""); }}>Cancel</button>
              </>
            ) : (
              <>
                <button style={miniButtonStyle} onClick={() => runWithError("Approve commit point", approveCommitPoint(cp.id))}>Approve</button>
                <button style={miniButtonStyle} onClick={() => setEditing(true)}>Edit</button>
                <button style={miniButtonStyle} onClick={() => {
                  const note = window.prompt("Rejection note (sent to agent on retry):", "");
                  if (note != null) runWithError("Reject commit point", rejectCommitPoint(cp.id, note));
                }}>Reject</button>
              </>
            )}
          </div>
        </div>
      ) : null}
      {cp.status === "rejected" && cp.rejection_note ? (
        <div style={{ marginTop: 6, fontSize: 11, color: "#e06b6b" }}>Rejected: {cp.rejection_note}</div>
      ) : null}
    </div>
  );
}

const messagePreStyle: CSSProperties = {
  margin: 0,
  padding: 6,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  whiteSpace: "pre-wrap",
};

const messageEditStyle: CSSProperties = {
  width: "100%",
  padding: 6,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  color: "var(--fg)",
  resize: "vertical",
};
