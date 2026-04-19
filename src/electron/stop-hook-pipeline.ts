import type { Batch } from "../persistence/batch-store.js";
import type { CommitPoint } from "../persistence/commit-point-store.js";
import type { WaitPoint } from "../persistence/wait-point-store.js";
import type { WorkItem } from "../persistence/work-item-store.js";

/**
 * Pure decision function for the Claude Stop-hook. Lives outside the
 * runtime god object so each branch can be unit-tested with a fixture
 * snapshot — and so the decision and the side effects (e.g. flipping a
 * wait point to `triggered`) stay separate. The runtime's
 * `handleHookEnvelope` builds a `BatchSnapshot` from the live stores,
 * passes it here, then applies the returned `sideEffects` and returns
 * `directive` to Claude.
 *
 * The pipeline runs in priority order:
 *   1. Pending commit point: block, ask the agent to propose a commit.
 *   2. Pending wait point: side-effect "trigger" + allow stop. The
 *      user resumes by prompting the agent directly.
 *   3. Approval-mode commit at `proposed`: allow stop while the user
 *      reviews.
 *   4. Writer batch with a ready work item: block, ask the agent to
 *      pick it up (auto-progression).
 *   5. Allow stop.
 *
 * `directive` of `null` lets Claude stop normally; otherwise it's the
 * hook body (typically `{ decision: "block", reason: string }`).
 */

export interface BatchSnapshot {
  batch: Batch | null;
  commitPoints: CommitPoint[];
  waitPoints: WaitPoint[];
  /** Every work item in the batch — needed to evaluate whether a marker's
   *  preceding items are all terminal. */
  workItems: WorkItem[];
  /** Pre-filtered "ready to work on" items, in the order
   *  `WorkItemStore.listReady` returns. The pipeline picks the first as
   *  the next item to direct the agent to. Kept as a separate field so
   *  the link-aware filtering stays inside the store. */
  readyWorkItems: WorkItem[];
}

export interface StopDirective {
  decision: "block";
  reason: string;
}

export type StopHookSideEffect =
  | { kind: "trigger-wait-point"; id: string };

export interface StopHookOutcome {
  directive: StopDirective | null;
  sideEffects: StopHookSideEffect[];
}

export function decideStopDirective(
  snapshot: BatchSnapshot,
  builders: {
    buildCommitPointReason: (cp: CommitPoint) => string;
    buildNextWorkItemReason: (item: WorkItem) => string;
  },
): StopHookOutcome {
  const sideEffects: StopHookSideEffect[] = [];

  const activeCommit = findActiveMarker(snapshot.commitPoints, snapshot.workItems, (cp) => cp.status !== "done");
  // Only the writer batch ever commits. Non-active batches share the worktree
  // read-only, so we must not prompt them to propose a commit; leave the
  // commit point pending for the batch that eventually becomes writer.
  if (activeCommit && activeCommit.status === "pending" && snapshot.batch?.status === "active") {
    return {
      directive: { decision: "block", reason: builders.buildCommitPointReason(activeCommit) },
      sideEffects,
    };
  }

  const activeWait = findActiveMarker(snapshot.waitPoints, snapshot.workItems, (wp) => wp.status === "pending");
  if (activeWait) {
    sideEffects.push({ kind: "trigger-wait-point", id: activeWait.id });
    return { directive: null, sideEffects };
  }

  if (activeCommit && activeCommit.status === "proposed") {
    return { directive: null, sideEffects };
  }

  if (!snapshot.batch || snapshot.batch.status !== "active") {
    return { directive: null, sideEffects };
  }

  const next = snapshot.readyWorkItems[0] ?? null;
  if (!next) return { directive: null, sideEffects };

  return {
    directive: { decision: "block", reason: builders.buildNextWorkItemReason(next) },
    sideEffects,
  };
}

/**
 * Lowest-sort_index marker whose preceding work items are all terminal
 * AND that passes `eligible`. Shared by commit and wait point lookups
 * so the "preceding items must be done" rule lives in one place.
 */
function findActiveMarker<T extends { sort_index: number }>(
  markers: T[],
  workItems: WorkItem[],
  eligible: (m: T) => boolean,
): T | null {
  for (const marker of markers) {
    if (!eligible(marker)) continue;
    const preceding = workItems.filter((item) => item.sort_index < marker.sort_index);
    const allTerminal = preceding.every((item) => item.status === "done" || item.status === "canceled");
    if (!allTerminal) continue;
    return marker;
  }
  return null;
}

