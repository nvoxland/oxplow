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
  /** ISO timestamp of the currently-open agent turn's `started_at`. When
   *  present, ready items whose `created_by === "agent"` AND
   *  `created_at >= currentTurnStartedAt` are skipped for auto-progression
   *  (the agent filed them for the user to triage — forcing continuation
   *  would invert that intent). Optional so callers that don't track turn
   *  lifecycle still get the old behaviour. */
  currentTurnStartedAt?: string | null;
  /** Paths the current agent_turn wrote to, relative to the worktree.
   *  Used to decide whether the "next work item" directive should carry a
   *  visual-verification nudge (any src/ui/** change ⇒ nudge). Optional;
   *  empty / absent means "no nudge". */
  currentTurnFilePaths?: string[];
}

export interface NextWorkItemContext {
  /** Set to true when the current turn wrote to any src/ui/** path — the
   *  builder should emit a "restart newde and exercise in the browser"
   *  banner before the normal directive text. */
  uiChangeNudge: boolean;
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
    buildNextWorkItemReason: (item: WorkItem, context: NextWorkItemContext) => string;
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

  const next = pickNextReadyItem(snapshot.readyWorkItems, snapshot.currentTurnStartedAt);
  if (!next) return { directive: null, sideEffects };

  const uiChangeNudge = turnTouchedUi(snapshot.currentTurnFilePaths);
  return {
    directive: {
      decision: "block",
      reason: builders.buildNextWorkItemReason(next, { uiChangeNudge }),
    },
    sideEffects,
  };
}

function turnTouchedUi(paths: string[] | undefined): boolean {
  if (!paths || paths.length === 0) return false;
  // Any repo-relative path under `src/ui/` — matches `src/ui/**`. Keep the
  // prefix narrow so touching shared utilities like
  // `src/electron/work-item-api.ts` doesn't falsely trip the banner.
  return paths.some((p) => p === "src/ui" || p.startsWith("src/ui/"));
}

// Skip items that the agent itself filed during the current turn. These are
// by convention "triage inbox" entries — the /autoimprove flow, bug reports
// filed during investigation, etc. — and forcing the agent to pick them up
// immediately would invert the user's triage intent. User-filed items and
// older agent-filed items still fire the directive.
function pickNextReadyItem(ready: WorkItem[], turnStartedAtIso: string | null | undefined): WorkItem | null {
  if (!turnStartedAtIso) return ready[0] ?? null;
  const turnStartMs = Date.parse(turnStartedAtIso);
  if (!Number.isFinite(turnStartMs)) return ready[0] ?? null;
  for (const item of ready) {
    if (item.created_by !== "agent") return item;
    const createdMs = Date.parse(item.created_at);
    if (!Number.isFinite(createdMs) || createdMs < turnStartMs) return item;
  }
  return null;
}

/**
 * Lowest-sort_index marker whose preceding work items are all terminal
 * AND that passes `eligible`. Shared by commit and wait point lookups
 * so the "preceding items are past" rule lives in one place.
 *
 * Terminal = `done`, `canceled`, or `human_check`. `human_check` is where
 * the agent parks finished work for the user to verify — from the queue's
 * perspective the agent is past it, so we must not keep marching
 * indefinitely just because the human hasn't clicked "done" yet.
 */
function findActiveMarker<T extends { sort_index: number }>(
  markers: T[],
  workItems: WorkItem[],
  eligible: (m: T) => boolean,
): T | null {
  for (const marker of markers) {
    if (!eligible(marker)) continue;
    const preceding = workItems.filter((item) => item.sort_index < marker.sort_index);
    const allTerminal = preceding.every(isTerminalStatus);
    if (!allTerminal) continue;
    return marker;
  }
  return null;
}

function isTerminalStatus(item: WorkItem): boolean {
  return item.status === "done" || item.status === "canceled" || item.status === "human_check";
}

