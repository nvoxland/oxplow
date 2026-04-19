import type { WaitPoint } from "../../api.js";
import { deleteWaitPoint, setWaitPointNote } from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { miniButtonStyle } from "./plan-utils.js";
import { queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a wait-point divider — shows the note (if any),
 * Edit and Delete actions, and a hint when the marker has been
 * triggered. The compact divider itself is in WorkGroupList.
 */
export function WaitPointRow({ wp }: { wp: WaitPoint }) {
  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {wp.note ? <span style={{ color: "var(--muted)", fontSize: 11 }}>{wp.note}</span> : null}
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          {wp.status === "pending" ? (
            <button type="button"
              style={miniButtonStyle}
              onClick={() => {
                const next = window.prompt("Wait point note:", wp.note ?? "");
                if (next != null) runWithError("Update wait note", setWaitPointNote(wp.id, next || null));
              }}
            >
              Edit
            </button>
          ) : null}
          <button type="button"
            style={miniButtonStyle}
            onClick={() => {
              if (window.confirm("Delete this wait point?")) runWithError("Delete wait point", deleteWaitPoint(wp.id));
            }}
          >
            Delete
          </button>
        </span>
      </div>
      {wp.status === "triggered" ? (
        <div style={{ marginTop: 4, fontSize: 11, color: "#d97706" }}>
          Agent stopped here. Prompt the agent directly to resume.
        </div>
      ) : null}
    </div>
  );
}
