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
 * work items, and the synchronous execution of a commit once the agent
 * calls `newde__commit` (after the user approves in chat).
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

  createCommitPoint(batchId: string): CommitPoint {
    // A commit point with no preceding work items has nothing to commit;
    // refuse to create one as the very first queue entry. The mixed reorder
    // still lets users drag a point above all work items if they really
    // insist.
    if (this.workItemStore.listItems(batchId).length === 0) {
      throw new Error("cannot add a commit point before any work items exist");
    }
    return this.commitPointStore.create({
      batchId,
      sortIndex: this.nextQueueSortIndex(batchId),
    });
  }

  proposeCommit(id: string, message: string): CommitPoint {
    return this.commitPointStore.propose(id, message);
  }

  /** Update mutable fields on a commit point (mode and/or message).
   *  Returns the updated list of commit points for the batch so the UI can
   *  refresh in one round-trip. */
  updateCommitPoint(id: string, changes: { mode?: CommitPointMode; message?: string }): CommitPoint[] {
    const updated = this.commitPointStore.update(id, changes);
    return this.commitPointStore.listForBatch(updated.batch_id);
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
   * Run `git commit` for a commit point. Called synchronously from the
   * `newde__commit` MCP handler after the user has approved the drafted
   * message in chat. Throws if git commit fails; the caller surfaces the
   * error to the agent so it can retry.
   */
  executeCommit(cpId: string, message: string): CommitPoint {
    const cp = this.commitPointStore.get(cpId);
    if (!cp) throw new Error(`commit point ${cpId} not found`);
    if (cp.status === "done") throw new Error(`commit point ${cpId} already committed`);
    const batch = this.batchStore.findById(cp.batch_id);
    if (!batch) throw new Error(`commit point ${cpId} has no batch`);
    const stream = this.streamStore.get(batch.stream_id);
    if (!stream) throw new Error(`commit point ${cpId} has no stream`);
    // The agent that authored the queue is the only writer in its worktree,
    // so picking up untracked files is the intent here. The Files-commit
    // dialog keeps its own narrower default.
    const result = gitCommitAll(stream.worktree_path, message, { includeUntracked: true });
    if (!result.ok || !result.sha) {
      throw new Error(`git commit failed: ${result.stderr || "unknown"}`);
    }
    const updated = this.commitPointStore.markCommitted(cpId, message, result.sha);
    this.logger.info("committed for commit point", { id: cpId, sha: result.sha });
    return updated;
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
