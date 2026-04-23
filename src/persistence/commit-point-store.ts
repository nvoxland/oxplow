import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

// Commit points support two modes:
//   "approve" (default) — the agent drafts a message in chat, waits for user
//     approval, then calls `oxplow__commit` to run git.
//   "auto" — the runtime commits immediately without user approval (stop-hook
//     pipeline handles this with an auto-generated message).
export type CommitPointStatus = "pending" | "done";
export type CommitPointMode = "auto" | "approve";

const COMMIT_POINT_STATUSES: ReadonlySet<CommitPointStatus> = new Set([
  "pending", "done",
]);

const MESSAGE_MAX_LEN = 20_000;

export interface CommitPoint {
  id: string;
  thread_id: string;
  sort_index: number;
  mode: CommitPointMode;
  status: CommitPointStatus;
  commit_sha: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface CommitPointChange {
  threadId: string;
  kind: "created" | "updated" | "deleted" | "reordered";
  id: string | null;
}

export class CommitPointStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<CommitPointChange>;

  constructor(projectDir: string, logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("commit point", logger);
  }

  subscribe(listener: (change: CommitPointChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emit(change: CommitPointChange): void {
    this.emitter.emit(change);
  }

  listForThread(threadId: string): CommitPoint[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM commit_point WHERE thread_id = ? ORDER BY sort_index, created_at, id`,
        threadId,
      )
      .map(toCommitPoint);
  }

  /**
   * Most recently completed commit_point for a thread (by `completed_at`
   * DESC). Returns null when the thread has never committed. Used by the
   * `tasks_since_last_commit` MCP tool and by the auto-commit fallback
   * message builder to bound "what changed since last commit."
   */
  getLatestDoneForThread(threadId: string): CommitPoint | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM commit_point
       WHERE thread_id = ? AND status = 'done' AND completed_at IS NOT NULL
       ORDER BY completed_at DESC, rowid DESC
       LIMIT 1`,
      threadId,
    );
    return row ? toCommitPoint(row) : null;
  }

  get(id: string): CommitPoint | null {
    const row = this.stateDb.get<Record<string, unknown>>(`SELECT * FROM commit_point WHERE id = ?`, id);
    return row ? toCommitPoint(row) : null;
  }

  /** Append a commit point at the end of the thread's queue. The caller passes
   *  the next sort_index (the runtime computes it across all three queue
   *  tables). Default mode is 'approve' (user reviews before commit). */
  create(input: { threadId: string; sortIndex: number; mode?: CommitPointMode }): CommitPoint {
    const id = createId("cp");
    const now = new Date().toISOString();
    const mode = input.mode ?? "approve";
    this.stateDb.run(
      `INSERT INTO commit_point (id, thread_id, sort_index, mode, status, created_at, updated_at)
       VALUES (?, ?, ?, ?, 'pending', ?, ?)`,
      id, input.threadId, input.sortIndex, mode, now, now,
    );
    const row = this.get(id);
    if (!row) throw new Error("commit point not persisted");
    this.emit({ threadId: input.threadId, kind: "created", id });
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
    // Emit one reordered change per distinct thread touched.
    const threads = new Set<string>();
    for (const entry of entries) {
      const cp = this.get(entry.id);
      if (cp) threads.add(cp.thread_id);
    }
    for (const threadId of threads) {
      this.emit({ threadId, kind: "reordered", id: null });
    }
  }

  /** Update mutable fields on a commit point. Mode is the only editable field
   *  now that drafted messages live in chat instead of the DB. */
  update(id: string, changes: { mode?: CommitPointMode }): CommitPoint {
    const cp = this.requireCommitPoint(id);
    if (cp.status === "done") throw new Error("cannot update a completed commit point");
    const now = new Date().toISOString();
    if (changes.mode !== undefined) {
      this.stateDb.run(
        `UPDATE commit_point SET mode = ?, updated_at = ? WHERE id = ?`,
        changes.mode, now, id,
      );
    }
    const updated = this.requireCommitPoint(id);
    this.emit({ threadId: updated.thread_id, kind: "updated", id });
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
      `UPDATE commit_point SET commit_sha = ?, status = 'done', completed_at = ?, updated_at = ? WHERE id = ?`,
      sha, now, now, id,
    );
    const updated = this.requireCommitPoint(id);
    this.emit({ threadId: updated.thread_id, kind: "updated", id });
    return updated;
  }

  delete(id: string): void {
    const cp = this.get(id);
    if (!cp) return;
    if (cp.status === "done") throw new Error("cannot delete a completed commit point");
    this.stateDb.run(`DELETE FROM commit_point WHERE id = ?`, id);
    this.emit({ threadId: cp.thread_id, kind: "deleted", id });
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
  // ("approved", "rejected", "proposed") that are no longer valid. Coerce
  // them forward to the current two-state machine:
  //   approved / proposed  → pending  (waiting for chat-side approval)
  //   rejected             → pending  (start over)
  const rawStatus = String(row.status);
  const status: CommitPointStatus =
    rawStatus === "approved" || rawStatus === "rejected" || rawStatus === "proposed" ? "pending"
    : COMMIT_POINT_STATUSES.has(rawStatus as CommitPointStatus) ? rawStatus as CommitPointStatus
    : (() => { throw new Error(`invalid commit_point.status: ${rawStatus}`); })();
  // Coerce unrecognised mode values to the safe default.
  const rawMode = String(row.mode ?? "approve");
  const mode: CommitPointMode = rawMode === "auto" ? "auto" : "approve";
  return {
    id: String(row.id),
    thread_id: String(row.thread_id),
    sort_index: Number(row.sort_index),
    mode,
    status,
    commit_sha: row.commit_sha == null ? null : String(row.commit_sha),
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
  };
}
