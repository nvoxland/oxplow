import { Slideover } from "./Slideover.js";
import type { StreamRemoval } from "../window/useStreamThreadMenus.js";

/** Remove… on a stream asks here first (with the option to delete its
 *  worktree): every shell draws it for the window's `removal`. */
export function RemoveStreamDialog({ removal }: { removal: StreamRemoval }) {
  return (
    <Slideover
      open={!!removal.stream}
      onClose={removal.cancel}
      title={removal.stream ? `Remove stream — ${removal.stream.title}` : "Remove stream"}
      testId="stream-remove-slideover"
      footer={(
        <>
          <button
            type="button"
            onClick={removal.cancel}
            disabled={removal.busy}
            style={{
              background: "var(--surface-card)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-subtle)",
              padding: "6px 12px",
              borderRadius: 6,
              cursor: removal.busy ? "not-allowed" : "pointer",
              fontFamily: "inherit",
              fontSize: "var(--text-xs)",
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="stream-remove-confirm"
            onClick={() => { void removal.confirm(); }}
            disabled={removal.busy}
            style={{
              background: "#b32a2a",
              color: "#fff",
              border: "1px solid transparent",
              padding: "6px 12px",
              borderRadius: 6,
              cursor: removal.busy ? "not-allowed" : "pointer",
              fontFamily: "inherit",
              fontSize: "var(--text-xs)",
              fontWeight: "var(--weight-medium)",
            }}
          >
            {removal.busy ? "Removing…" : "Remove"}
          </button>
        </>
      )}
    >
      {removal.stream ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 12, fontSize: "var(--text-xs)" }}>
          <p style={{ margin: 0, color: "var(--text-secondary)", lineHeight: 1.5 }}>
            The stream and every thread under it will be archived (hidden from the rail). History — requests, snapshots, and visit logs — stays intact.
          </p>
          <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <input
              type="checkbox"
              data-testid="stream-remove-delete-worktree"
              checked={removal.worktree}
              onChange={(e) => removal.setWorktree(e.target.checked)}
              disabled={removal.busy || removal.stream.kind !== "worktree"}
            />
            <span>
              Also delete the on-disk worktree
              {removal.stream.kind === "worktree" ? (
                <span style={{ color: "var(--text-secondary)" }}> ({removal.stream.worktree_path})</span>
              ) : (
                <span style={{ color: "var(--text-secondary)" }}> — primary stream has no worktree to delete</span>
              )}
            </span>
          </label>
          {removal.error ? (
            <div style={{ color: "#ff6b6b", whiteSpace: "pre-wrap" }}>{removal.error}</div>
          ) : null}
        </div>
      ) : null}
    </Slideover>
  );
}
