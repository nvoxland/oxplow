import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

// Commit points support two modes:
//   "approve" (default) — the agent proposes a draft message, shows it to the
//     user in chat, and waits for approval before committing.
//   "auto" — the agent proposes a message and the runtime commits immediately
//     without waiting for user approval (stop-hook pipeline handles this).
export type CommitPointStatus = "pending" | "proposed" | "done";
export type CommitPointMode = "auto" | "approve";

const COMMIT_POINT_STATUSES: ReadonlySet<CommitPointStatus> = new Set([
  "pending", "proposed", "done",
]);

const MESSAGE_MAX_LEN = 20_000;

export interface CommitPoint {
  id: string;
  batch_id: string;
  sort_index: number;
  mode: CommitPointMode;
  status: CommitPointStatus;
  proposed_message: string | null;
  commit_sha: string | null;
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

  /** Append a commit point at the end of the batch's queue. The caller passes
   *  the next sort_index (the runtime computes it across all three queue
   *  tables). Default mode is 'approve' (user reviews before commit). */
  create(input: { batchId: string; sortIndex: number; mode?: CommitPointMode }): CommitPoint {
    const id = createId("cp");
    const now = new Date().toISOString();
    const mode = input.mode ?? "approve";
    this.stateDb.run(
      `INSERT INTO commit_point (id, batch_id, sort_index, mode, status, created_at, updated_at)
       VALUES (?, ?, ?, ?, 'pending', ?, ?)`,
      id, input.batchId, input.sortIndex, mode, now, now,
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

  /** Update mutable fields on a commit point (mode and/or proposed_message).
   *  Returns the full updated commit point. */
  update(id: string, changes: { mode?: CommitPointMode; message?: string }): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status === "done") throw new Error("cannot update a completed commit point");
    const now = new Date().toISOString();
    if (changes.mode !== undefined) {
      this.stateDb.run(
        `UPDATE commit_point SET mode = ?, updated_at = ? WHERE id = ?`,
        changes.mode, now, id,
      );
    }
    if (changes.message !== undefined) {
      const trimmed = clampMessage(changes.message);
      this.stateDb.run(
        `UPDATE commit_point SET proposed_message = ?, updated_at = ? WHERE id = ?`,
        trimmed || null, now, id,
      );
    }
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** Agent drafts (or redrafts) a commit message. Status lands at `proposed`
   *  and the Stop-hook stops re-blocking so the user can respond in chat. */
  propose(id: string, message: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status === "done") throw new Error("commit point already completed");
    const trimmed = clampMessage(message);
    if (!trimmed) throw new Error("proposed message is empty");
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET proposed_message = ?, status = 'proposed', updated_at = ? WHERE id = ?`,
      trimmed, now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** Record that the git commit succeeded. Callers run `git commit`
   *  themselves and pass the resulting sha here. */
  markCommitted(id: string, message: string, sha: string): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status === "done") throw new Error("commit point already completed");
    const trimmed = clampMessage(message);
    if (!trimmed) throw new Error("commit message is empty");
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE commit_point SET proposed_message = ?, commit_sha = ?, status = 'done', completed_at = ?, updated_at = ? WHERE id = ?`,
      trimmed, sha, now, now, id,
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
  // Historical rows from pre-chat-approval builds may carry statuses
  // ("approved", "rejected") that are no longer valid. Coerce them forward:
  //   approved  → proposed  (message drafted; not yet committed)
  //   rejected  → pending   (start over)
  const rawStatus = String(row.status);
  const status: CommitPointStatus =
    rawStatus === "approved" ? "proposed"
    : rawStatus === "rejected" ? "pending"
    : COMMIT_POINT_STATUSES.has(rawStatus as CommitPointStatus) ? rawStatus as CommitPointStatus
    : (() => { throw new Error(`invalid commit_point.status: ${rawStatus}`); })();
  // Coerce unrecognised mode values to the safe default.
  const rawMode = String(row.mode ?? "approve");
  const mode: CommitPointMode = rawMode === "auto" ? "auto" : "approve";
  return {
    id: String(row.id),
    batch_id: String(row.batch_id),
    sort_index: Number(row.sort_index),
    mode,
    status,
    proposed_message: row.proposed_message == null ? null : String(row.proposed_message),
    commit_sha: row.commit_sha == null ? null : String(row.commit_sha),
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
  };
}
