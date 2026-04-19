import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export type CommitPointMode = "auto" | "approval";
export type CommitPointStatus = "pending" | "proposed" | "approved" | "done" | "rejected";

const COMMIT_POINT_MODES: ReadonlySet<CommitPointMode> = new Set(["auto", "approval"]);
const COMMIT_POINT_STATUSES: ReadonlySet<CommitPointStatus> = new Set([
  "pending", "proposed", "approved", "done", "rejected",
]);

const MESSAGE_MAX_LEN = 20_000;
const NOTE_MAX_LEN = 2_000;

export interface CommitPoint {
  id: string;
  batch_id: string;
  sort_index: number;
  mode: CommitPointMode;
  status: CommitPointStatus;
  proposed_message: string | null;
  approved_message: string | null;
  commit_sha: string | null;
  rejection_note: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface CommitPointChange {
  batchId: string;
  kind: "created" | "updated" | "deleted" | "reordered";
  id: string | null;
}

export class CommitPointStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<CommitPointChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("commit point", logger);
  }

  subscribe(listener: (change: CommitPointChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emit(change: CommitPointChange): void {
    this.emitter.emit(change);
  }

  listForBatch(batchId: string): CommitPoint[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM commit_point WHERE batch_id = ? ORDER BY sort_index, created_at, id`,
        batchId,
      )
      .map(toCommitPoint);
  }

  get(id: string): CommitPoint | null {
    const row = this.stateDb.get<Record<string, unknown>>(`SELECT * FROM commit_point WHERE id = ?`, id);
    return row ? toCommitPoint(row) : null;
  }

  /**
   * Append a commit point at the end of the batch's work queue. The caller
   * passes the current max sort_index of work_items for that batch so we land
   * strictly after all existing items; the runtime is the right place to look
   * up that max (it has both stores).
   */
  create(input: { batchId: string; mode: CommitPointMode; sortIndex: number }): CommitPoint {
    if (!COMMIT_POINT_MODES.has(input.mode)) throw new Error(`invalid commit mode: ${input.mode}`);
    const id = createId("cp");
    const now = new Date().toISOString();
    this.stateDb.run(
      `INSERT INTO commit_point (id, batch_id, sort_index, mode, status, created_at, updated_at)
       VALUES (?, ?, ?, ?, 'pending', ?, ?)`,
      id, input.batchId, input.sortIndex, input.mode, now, now,
    );
    const row = this.get(id);
    if (!row) throw new Error("commit point not persisted");
    this.emit({ batchId: input.batchId, kind: "created", id });
    return row;
  }

  /** Bulk assign sort_index values. Caller provides entries in desired order;
   *  the store just writes the given index verbatim (no renumbering). */
  setSortIndexes(entries: Array<{ id: string; sortIndex: number }>): void {
    if (entries.length === 0) return;
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const entry of entries) {
        this.stateDb.run(
          `UPDATE commit_point SET sort_index = ?, updated_at = ? WHERE id = ?`,
          entry.sortIndex, now, entry.id,
        );
      }
    });
    // Emit one reordered change per distinct batch touched.
    const batches = new Set<string>();
    for (const entry of entries) {
      const cp = this.get(entry.id);
      if (cp) batches.add(cp.batch_id);
    }
    for (const batchId of batches) {
      this.emit({ batchId, kind: "reordered", id: null });
    }
  }

  setMode(id: string, mode: CommitPointMode): CommitPoint {
    if (!COMMIT_POINT_MODES.has(mode)) throw new Error(`invalid commit mode: ${mode}`);
    const existing = this.requireCommitPoint(id);
    if (existing.status !== "pending") {
      throw new Error(`cannot change mode once commit point is ${existing.status}`);
    }
    const now = new Date().toISOString();
    this.stateDb.run(`UPDATE commit_point SET mode = ?, updated_at = ? WHERE id = ?`, mode, now, id);
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  propose(id: string, message: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status === "done") throw new Error("commit point already completed");
    const trimmed = clampMessage(message);
    const now = new Date().toISOString();
    // Auto-mode: jump straight to approved so the runtime will commit on the
    // next poll. Approval mode: park at proposed for the user.
    const nextStatus: CommitPointStatus = cp.mode === "auto" ? "approved" : "proposed";
    this.stateDb.run(
      `UPDATE commit_point SET proposed_message = ?, approved_message = ?, status = ?, rejection_note = NULL, updated_at = ? WHERE id = ?`,
      trimmed,
      cp.mode === "auto" ? trimmed : null,
      nextStatus,
      now,
      id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** User approves (optionally editing the message). Moves to `approved`; the
   *  runtime's commit executor will pick it up and move it to `done`. */
  approve(id: string, editedMessage?: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status !== "proposed") throw new Error(`cannot approve commit point in status ${cp.status}`);
    const message = clampMessage(editedMessage ?? cp.proposed_message ?? "");
    if (!message) throw new Error("approved message is empty");
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET approved_message = ?, status = 'approved', updated_at = ? WHERE id = ?`,
      message, now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  reject(id: string, note: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status !== "proposed") throw new Error(`cannot reject commit point in status ${cp.status}`);
    return this.transitionToRejected(id, note);
  }

  /**
   * Move an `approved` point to `rejected` because the runtime's `git commit`
   * itself failed. Distinct from user-initiated reject (which only fires from
   * `proposed`) because the failure path needs to escape from `approved` —
   * otherwise the startup-recovery loop retries the same broken commit
   * forever.
   */
  failExecution(id: string, note: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status !== "approved") throw new Error(`cannot fail execution in status ${cp.status}`);
    return this.transitionToRejected(id, note);
  }

  private transitionToRejected(id: string, note: string): CommitPoint {
    const trimmed = note.slice(0, NOTE_MAX_LEN);
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET status = 'rejected', rejection_note = ?, updated_at = ? WHERE id = ?`,
      trimmed, now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** Called after runtime performs the git commit. */
  markDone(id: string, sha: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status !== "approved") throw new Error(`cannot mark done in status ${cp.status}`);
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET status = 'done', commit_sha = ?, completed_at = ?, updated_at = ? WHERE id = ?`,
      sha, now, now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** Transition a previously-rejected point back to pending so the agent can
   *  retry on the next Stop hook. */
  resetToPending(id: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status !== "rejected") throw new Error(`cannot reset commit point in status ${cp.status}`);
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET status = 'pending', proposed_message = NULL, approved_message = NULL, updated_at = ? WHERE id = ?`,
      now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  delete(id: string): void {
    const cp = this.get(id);
    if (!cp) return;
    if (cp.status === "done") throw new Error("cannot delete a completed commit point");
    this.stateDb.run(`DELETE FROM commit_point WHERE id = ?`, id);
    this.emit({ batchId: cp.batch_id, kind: "deleted", id });
  }

  /** Approved but not yet committed; the runtime's commit executor polls this. */
  listApproved(): CommitPoint[] {
    return this.stateDb
      .all<Record<string, unknown>>(`SELECT * FROM commit_point WHERE status = 'approved' ORDER BY updated_at`)
      .map(toCommitPoint);
  }

  private requireCommitPoint(id: string): CommitPoint {
    const cp = this.get(id);
    if (!cp) throw new Error(`commit point ${id} not found`);
    return cp;
  }
}

function clampMessage(s: string): string {
  return s.slice(0, MESSAGE_MAX_LEN).trim();
}

function toCommitPoint(row: Record<string, unknown>): CommitPoint {
  const mode = String(row.mode);
  const status = String(row.status);
  if (!COMMIT_POINT_MODES.has(mode as CommitPointMode)) throw new Error(`invalid commit_point.mode: ${mode}`);
  if (!COMMIT_POINT_STATUSES.has(status as CommitPointStatus)) throw new Error(`invalid commit_point.status: ${status}`);
  return {
    id: String(row.id),
    batch_id: String(row.batch_id),
    sort_index: Number(row.sort_index),
    mode: mode as CommitPointMode,
    status: status as CommitPointStatus,
    proposed_message: row.proposed_message == null ? null : String(row.proposed_message),
    approved_message: row.approved_message == null ? null : String(row.approved_message),
    commit_sha: row.commit_sha == null ? null : String(row.commit_sha),
    rejection_note: row.rejection_note == null ? null : String(row.rejection_note),
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
  };
}
