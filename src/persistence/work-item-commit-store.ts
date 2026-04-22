import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

/**
 * `work_item_commit` junction rows link a work item to the git commit(s)
 * that landed its changes. Populated by the runtime's auto-commit path
 * after a successful `gitCommitAll`. Enables blame / activity-panel UX
 * that attributes a commit sha back to the contributing work items.
 *
 * Columns: `work_item_id`, `sha`, `committed_at`. Composite PK on
 * `(work_item_id, sha)`.
 */
export interface WorkItemCommit {
  work_item_id: string;
  sha: string;
  committed_at: string;
}

export class WorkItemCommitStore {
  private readonly stateDb;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
  }

  /** Idempotent via `INSERT OR IGNORE` — re-inserting the same (item, sha) is a no-op. */
  insert(workItemId: string, sha: string, committedAt?: string): void {
    const ts = committedAt ?? new Date().toISOString();
    this.stateDb.run(
      `INSERT OR IGNORE INTO work_item_commit (work_item_id, sha, committed_at) VALUES (?, ?, ?)`,
      workItemId,
      sha,
      ts,
    );
  }

  listShasForItem(workItemId: string): WorkItemCommit[] {
    return this.stateDb
      .all<WorkItemCommit>(
        `SELECT work_item_id, sha, committed_at FROM work_item_commit WHERE work_item_id = ? ORDER BY committed_at DESC, sha`,
        workItemId,
      )
      .map((r) => ({
        work_item_id: String(r.work_item_id),
        sha: String(r.sha),
        committed_at: String(r.committed_at),
      }));
  }

  listItemsForSha(sha: string): WorkItemCommit[] {
    return this.stateDb
      .all<WorkItemCommit>(
        `SELECT work_item_id, sha, committed_at FROM work_item_commit WHERE sha = ? ORDER BY work_item_id`,
        sha,
      )
      .map((r) => ({
        work_item_id: String(r.work_item_id),
        sha: String(r.sha),
        committed_at: String(r.committed_at),
      }));
  }
}
