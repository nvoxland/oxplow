import type { Logger } from "../core/logger.js";
import type { Thread, ThreadStore } from "../persistence/thread-store.js";
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
 * Owns the parts of the thread queue that span multiple stores: commit
 * points, wait points, the shared sort_index space they occupy alongside
 * work items, and the synchronous execution of a commit once the agent
 * calls `newde__commit` (after the user approves in chat).
 */
export class ThreadQueueOrchestrator {
  constructor(
    private readonly streamStore: StreamStore,
    private readonly threadStore: ThreadStore,
    private readonly workItemStore: WorkItemStore,
    private readonly commitPointStore: CommitPointStore,
    private readonly waitPointStore: WaitPointStore,
    private readonly logger: Logger,
  ) {}

  // -------- commit points --------

  listCommitPoints(threadId: string): CommitPoint[] {
    return this.commitPointStore.listForThread(threadId);
  }

  createCommitPoint(threadId: string, mode: CommitPointMode = "approve"): CommitPoint {
    // A commit point with no preceding work items has nothing to commit;
    // refuse to create one as the very first queue entry. The mixed reorder
    // still lets users drag a point above all work items if they really
    // insist.
    if (this.workItemStore.listItems(threadId).length === 0) {
      throw new Error("cannot add a commit point before any work items exist");
    }
    return this.commitPointStore.create({
      threadId,
      sortIndex: this.nextQueueSortIndex(threadId),
      mode,
    });
  }

  /** Update mutable fields on a commit point (currently only `mode`).
   *  Returns the updated list of commit points for the thread so the UI can
   *  refresh in one round-trip. */
  updateCommitPoint(id: string, changes: { mode?: CommitPointMode }): CommitPoint[] {
    const updated = this.commitPointStore.update(id, changes);
    return this.commitPointStore.listForThread(updated.thread_id);
  }

  deleteCommitPoint(id: string): void {
    this.commitPointStore.delete(id);
  }

  // -------- wait points --------

  listWaitPoints(threadId: string): WaitPoint[] {
    return this.waitPointStore.listForThread(threadId);
  }

  createWaitPoint(threadId: string, note: string | null): WaitPoint {
    if (this.workItemStore.listItems(threadId).length === 0) {
      throw new Error("cannot add a wait point before any work items exist");
    }
    return this.waitPointStore.create({
      threadId,
      sortIndex: this.nextQueueSortIndex(threadId),
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
   * Reorder the mixed thread queue (work items + commit points + wait
   * points). `entries` is the desired top-to-bottom order; sort_indexes
   * are rewritten to match so the Stop-hook pipeline sees the new
   * positions immediately.
   */
  reorderThreadQueue(
    threadId: string,
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
    this.workItemStore.setItemSortIndexes(threadId, workEntries);
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
    const thread = this.threadStore.findById(cp.thread_id);
    if (!thread) throw new Error(`commit point ${cpId} has no thread`);
    const stream = this.streamStore.get(thread.stream_id);
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
  nextQueueSortIndex(threadId: string): number {
    const items = this.workItemStore.listItems(threadId);
    const commits = this.commitPointStore.listForThread(threadId);
    const waits = this.waitPointStore.listForThread(threadId);
    const maxItem = items.reduce((m, item) => Math.max(m, item.sort_index), -1);
    const maxCommit = commits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    const maxWait = waits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    return Math.max(maxItem, maxCommit, maxWait) + 1;
  }
}

// Re-exports to keep imports simple at call sites.
export type { Thread, Stream };
