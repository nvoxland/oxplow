import type { WaitPoint } from "../../api.js";
import { deleteWaitPoint, setWaitPointNote } from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { InlineConfirm } from "../InlineConfirm.js";
import { InlineEdit } from "../InlineEdit.js";
import { miniButtonStyle } from "./plan-utils.js";
import { queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a wait-point divider — shows the note (inline-
 * editable when pending), and inline-confirmed Delete. The compact
 * divider itself is in WorkGroupList.
 */
export function WaitPointRow({ wp }: { wp: WaitPoint }) {
  const isPending = wp.status === "pending";
  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ flex: 1, color: "var(--muted)", fontSize: 11 }}>
          {isPending ? (
            <InlineEdit
              value={wp.note ?? ""}
              placeholder="Add a note (click to edit)"
              allowEmpty
              onCommit={(next) => {
                runWithError("Update wait note", setWaitPointNote(wp.id, next || null));
              }}
            />
          ) : (
            wp.note
          )}
        </span>
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          <InlineConfirm
            triggerLabel="Delete"
            confirmLabel="Delete"
            triggerStyle={miniButtonStyle}
            onConfirm={() => {
              runWithError("Delete wait point", deleteWaitPoint(wp.id));
            }}
          />
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
