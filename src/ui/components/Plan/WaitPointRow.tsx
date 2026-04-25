import { useState } from "react";
import type { WaitPoint } from "../../api.js";
import { deleteWaitPoint, setWaitPointNote } from "../../api.js";
import { runWithError } from "../../ui-error.js";
import { ConfirmDialog } from "../ConfirmDialog.js";
import { PromptDialog } from "../PromptDialog.js";
import { miniButtonStyle } from "./plan-utils.js";
import { queueRowExpandedStyle } from "./queue-markers.js";

/**
 * Expanded view of a wait-point divider — shows the note (if any),
 * Edit and Delete actions, and a hint when the marker has been
 * triggered. The compact divider itself is in WorkGroupList.
 */
export function WaitPointRow({ wp }: { wp: WaitPoint }) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editingNote, setEditingNote] = useState(false);
  return (
    <div style={queueRowExpandedStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {wp.note ? <span style={{ color: "var(--muted)", fontSize: 11 }}>{wp.note}</span> : null}
        <span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
          {wp.status === "pending" ? (
            <button type="button"
              style={miniButtonStyle}
              onClick={() => setEditingNote(true)}
            >
              Edit
            </button>
          ) : null}
          <button type="button"
            style={miniButtonStyle}
            onClick={() => setConfirmDelete(true)}
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
      {editingNote ? (
        <PromptDialog
          message="Wait point note"
          initialValue={wp.note ?? ""}
          allowEmpty
          confirmLabel="Save"
          onSubmit={(next) => {
            setEditingNote(false);
            runWithError("Update wait note", setWaitPointNote(wp.id, next || null));
          }}
          onCancel={() => setEditingNote(false)}
        />
      ) : null}
      {confirmDelete ? (
        <ConfirmDialog
          message="Delete this wait point?"
          confirmLabel="Delete"
          destructive
          onConfirm={() => {
            setConfirmDelete(false);
            runWithError("Delete wait point", deleteWaitPoint(wp.id));
          }}
          onCancel={() => setConfirmDelete(false)}
        />
      ) : null}
    </div>
  );
}
