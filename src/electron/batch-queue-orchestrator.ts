import type { Logger } from "../core/logger.js";
import type { Batch, BatchStore } from "../persistence/batch-store.js";
import {
  type CommitPoint,
  type CommitPointMode,
  type CommitPointStore,
} from "../persistence/commit-point-store.js";
import type { Stream, StreamStore } from "../persistence/stream-store.js";
import type { WaitPoint, WaitPointStore } from "../persistence/wait-point-store.js";
import type { WorkItemStore } from "../persistence/work-item-store.js";
import { gitCommitAll } from "../git/git.js";

/**
 * Owns the parts of the batch queue that span multiple stores: commit
 * points, wait points, the shared sort_index space they occupy alongside
 * work items, and the runtime-side execution of approved commits.
 *
 * Pulled out of `ElectronRuntime` so:
 *   - the cross-store reorder + execute paths stay close together rather
 *     than scattered across a god-object;
 *   - failure handling for `executeApprovedCommit` (which used to deadlock
 *     the queue when `git commit` failed) lives next to the proposal /
 *     approve / mark-done methods that produce the state it acts on;
 *   - tests can exercise the orchestrator with mock stores instead of
 *     spinning up an Electron runtime.
 *
 * The orchestrator does not subscribe to its own change events — the
 * runtime's existing subscription on `commitPointStore` continues to call
 * `executeApprovedCommit` when a point flips to `approved`.
 */
export class BatchQueueOrchestrator {
  constructor(
    private readonly streamStore: StreamStore,
    private readonly batchStore: BatchStore,
    private readonly workItemStore: WorkItemStore,
    private readonly commitPointStore: CommitPointStore,
    private readonly waitPointStore: WaitPointStore,
    private readonly logger: Logger,
  ) {}

  // -------- commit points --------

  listCommitPoints(batchId: string): CommitPoint[] {
    return this.commitPointStore.listForBatch(batchId);
  }

  createCommitPoint(batchId: string, mode: CommitPointMode): CommitPoint {
    // A commit point with no preceding work items has nothing to commit;
    // refuse to create one as the very first queue entry. The mixed reorder
    // still lets users drag a point above all work items if they really
    // insist.
    if (this.workItemStore.listItems(batchId).length === 0) {
      throw new Error("cannot add a commit point before any work items exist");
    }
    return this.commitPointStore.create({
      batchId,
      mode,
      sortIndex: this.nextQueueSortIndex(batchId),
    });
  }

  setCommitPointMode(id: string, mode: CommitPointMode): CommitPoint {
    return this.commitPointStore.setMode(id, mode);
  }

  approveCommitPoint(id: string, editedMessage?: string): CommitPoint {
    return this.commitPointStore.approve(id, editedMessage);
  }

  rejectCommitPoint(id: string, note: string): CommitPoint {
    return this.commitPointStore.reject(id, note);
  }

  resetCommitPoint(id: string): CommitPoint {
    return this.commitPointStore.resetToPending(id);
  }

  deleteCommitPoint(id: string): void {
    this.commitPointStore.delete(id);
  }

  // -------- wait points --------

  listWaitPoints(batchId: string): WaitPoint[] {
    return this.waitPointStore.listForBatch(batchId);
  }

  createWaitPoint(batchId: string, note: string | null): WaitPoint {
    if (this.workItemStore.listItems(batchId).length === 0) {
      throw new Error("cannot add a wait point before any work items exist");
    }
    return this.waitPointStore.create({
      batchId,
      sortIndex: this.nextQueueSortIndex(batchId),
      note,
    });
  }

  setWaitPointNote(id: string, note: string | null): WaitPoint {
    return this.waitPointStore.setNote(id, note);
  }

  deleteWaitPoint(id: string): void {
    this.waitPointStore.delete(id);
  }

  // -------- mixed reorder --------

  /**
   * Reorder the mixed batch queue (work items + commit points + wait
   * points). `entries` is the desired top-to-bottom order; sort_indexes
   * are rewritten to match so the Stop-hook pipeline sees the new
   * positions immediately.
   */
  reorderBatchQueue(
    batchId: string,
    entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>,
  ): void {
    const workEntries: Array<{ id: string; sortIndex: number }> = [];
    const commitEntries: Array<{ id: string; sortIndex: number }> = [];
    const waitEntries: Array<{ id: string; sortIndex: number }> = [];
    entries.forEach((entry, index) => {
      if (entry.kind === "work") workEntries.push({ id: entry.id, sortIndex: index });
      else if (entry.kind === "commit") commitEntries.push({ id: entry.id, sortIndex: index });
      else waitEntries.push({ id: entry.id, sortIndex: index });
    });
    this.workItemStore.setItemSortIndexes(batchId, workEntries);
    this.commitPointStore.setSortIndexes(commitEntries);
    this.waitPointStore.setSortIndexes(waitEntries);
  }

  // -------- commit execution --------

  /**
   * Run `git commit` for an `approved` commit point. Called both eagerly
   * (when a point flips to `approved` via the runtime's subscribe handler)
   * and at startup (for any approved-but-uncommitted points left over
   * from a crash).
   *
   * Failure path moves the point to `rejected` via `failExecution` so the
   * startup-recovery loop doesn't retry the same broken commit forever
   * — the user reads the rejection_note in the UI and clicks Retry.
   */
  executeApprovedCommit(cp: CommitPoint): void {
    const batch = this.batchStore.findById(cp.batch_id);
    if (!batch) {
      this.logger.warn("commit point has no batch; dropping", { id: cp.id });
      return;
    }
    const stream = this.streamStore.get(batch.stream_id);
    if (!stream) {
      this.logger.warn("commit point has no stream; dropping", { id: cp.id });
      return;
    }
    const message = cp.approved_message ?? cp.proposed_message;
    if (!message) {
      this.logger.warn("commit point approved with no message; skipping", { id: cp.id });
      return;
    }
    const result = gitCommitAll(stream.worktree_path, message);
    if (!result.ok || !result.sha) {
      this.logger.warn("git commit failed for commit point", {
        id: cp.id,
        stderr: result.stderr,
      });
      this.commitPointStore.failExecution(cp.id, `commit failed: ${result.stderr || "unknown"}`);
      return;
    }
    try {
      this.commitPointStore.markDone(cp.id, result.sha);
      this.logger.info("committed for commit point", { id: cp.id, sha: result.sha });
    } catch (err) {
      this.logger.warn("failed to mark commit point done", {
        id: cp.id,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  /** Drain any commit points left in `approved` from a prior run. */
  drainPendingExecutions(): void {
    for (const cp of this.commitPointStore.listApproved()) {
      this.executeApprovedCommit(cp);
    }
  }

  /**
   * Highest sort_index across all three queue tables, plus 1. Used by
   * createCommitPoint / createWaitPoint to append at the end. Public so
   * the runtime can use the same numbering when creating work items
   * through the work-item-api (today work items go via a different
   * path; this is here for symmetry as the orchestrator grows).
   */
  nextQueueSortIndex(batchId: string): number {
    const items = this.workItemStore.listItems(batchId);
    const commits = this.commitPointStore.listForBatch(batchId);
    const waits = this.waitPointStore.listForBatch(batchId);
    const maxItem = items.reduce((m, item) => Math.max(m, item.sort_index), -1);
    const maxCommit = commits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    const maxWait = waits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    return Math.max(maxItem, maxCommit, maxWait) + 1;
  }
}

// Re-exports to keep imports simple at call sites.
export type { Batch, Stream };
