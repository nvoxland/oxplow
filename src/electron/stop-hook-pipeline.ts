import type { Thread } from "../persistence/thread-store.js";
import type { CommitPoint } from "../persistence/commit-point-store.js";
import type { WaitPoint } from "../persistence/wait-point-store.js";
import type { WorkItem } from "../persistence/work-item-store.js";

/**
 * Pure decision function for the Claude Stop-hook. Lives outside the
 * runtime god object so each branch can be unit-tested with a fixture
 * snapshot — and so the decision and the side effects (e.g. flipping a
 * wait point to `triggered`) stay separate. The runtime's
 * `handleHookEnvelope` builds a `ThreadSnapshot` from the live stores,
 * passes it here, then applies the returned `sideEffects` and returns
 * `directive` to Claude.
 *
 * The pipeline runs in priority order:
 *   1. Pending commit point: block, ask the agent to draft a message
 *      (approve mode) or let the runtime auto-commit (auto mode).
 *   2. Pending wait point: side-effect "trigger" + allow stop. The
 *      user resumes by prompting the agent directly.
 *   3. Writer thread with `in_progress` work items: block, ask the
 *      agent to audit each (still in progress? blocked? done?
 *      paused? canceled?). Tasks persist across turn boundaries — the
 *      audit is the explicit settle step.
 *   4. Writer thread with no `in_progress` items but ready work: block
 *      and ask the agent to pick the next ready item up. Suppressed
 *      when the agent already saw the same ready set this turn (via
 *      `justReadReadySet`).
 *   5. Allow stop.
 *
 * `directive` of `null` lets Claude stop normally; otherwise it's the
 * hook body (typically `{ decision: "block", reason: string }`).
 */

export interface ThreadSnapshot {
  thread: Thread | null;
  commitPoints: CommitPoint[];
  waitPoints: WaitPoint[];
  /** Every work item in the thread — needed to evaluate whether a marker's
   *  preceding items are all terminal, AND to drive the in-progress audit
   *  branch (every item with status `in_progress` becomes a line in the
   *  audit list). */
  workItems: WorkItem[];
  /** Pre-filtered "ready to work on" items, in the order
   *  `WorkItemStore.listReady` returns. The pipeline picks the first as
   *  the next item to direct the agent to. Kept as a separate field so
   *  the link-aware filtering stays inside the store. */
  readyWorkItems: WorkItem[];
  /** When true and the thread is the writer, the stop-hook automatically
   *  proposes a commit whenever settled work (human_check/done) exists
   *  but no commit point is in the queue. */
  autoCommit?: boolean;
  /** Work-item ids the agent saw in the most recent `read_work_options` call
   *  during the immediately-preceding turn. When the current ready-set is
   *  identical, the ready-work directive is suppressed — the agent already
   *  has the list. Optional; absent means "no just-read suppression." */
  justReadReadySet?: string[];
  /** True when the orchestrator has dispatched a `Task` subagent that
   *  hasn't yet returned. While set, the in-progress audit and ready-work
   *  directives are suppressed — re-emitting them mid-flight produces a
   *  visual loop where the parent dutifully acks each Stop nudge while
   *  it's still waiting on the subagent. Commit/wait points still fire:
   *  those are user-placed markers, not queue-management nudges. */
  subagentInFlight?: boolean;
  /** True when the thread's worktree has no staged OR unstaged diff
   *  (ad-hoc `git commit` from Bash or the Files panel already landed).
   *  When set, the auto-commit directive is suppressed even if settled
   *  work appears unlinked — the next `git-refs.changed` backfill attaches
   *  the pending rows to the fresh sha. See wi-ec4c8e6f44fd. */
  worktreeClean?: boolean;
  /** False when the just-ended turn fired no qualifying tool calls
   *  (write-intent, oxplow filing, dispatch). A pure Q&A / question-asking
   *  turn — the agent answered or asked the user something and there's
   *  nothing more to do until the user replies. When false, the entire
   *  Stop directive pipeline is skipped so we don't force-march the agent
   *  past a question (commit, auto-commit, in-progress audit, ready-work
   *  are all suppressed). `undefined` is treated as "unknown → don't
   *  suppress" so behaviour stays stable when the activity flag wasn't
   *  threaded through (older tests, missing UserPromptSubmit, etc.). */
  turnHadActivity?: boolean;
  /** True when the turn read code (Read/Grep/Glob, read-only Bash) at
   *  least twice and produced zero write-intent activity. Combined with
   *  `turnHadActivity === false` this is the wiki-capture trigger: a
   *  read-heavy Q&A turn the agent should durably capture before stopping.
   *  Only checked when `turnHadActivity` is false; ignored otherwise so
   *  edits/commits still take precedence on real-work turns. */
  turnWasExploration?: boolean;
  /** True when the wiki-capture directive already fired earlier on this
   *  thread and hasn't been cleared by a fresh user prompt. Suppresses a
   *  second emission so the agent isn't asked to capture twice in a row
   *  (the first emission either produced a note or got the agent's
   *  `oxplow-note: skipped` reply; either way it's settled). */
  justEmittedWikiCapture?: boolean;
  /** Last auto-commit nag fingerprint we emitted for this thread (cp id +
   *  HEAD-or-"dirty"). When the current fingerprint matches, suppress —
   *  the agent already has the directive; repeating it without new diff
   *  to commit is pure noise. Cleared when the commit lands. */
  lastAutoCommitNagFingerprint?: string;
  /** Item id of the most recent ready-work nag, plus the consecutive count.
   *  After K (=3) fires for the same item without movement, suppress until
   *  the ready-set changes. Without this cap the same nag can fire 7+
   *  times in a session (transcript ff6fbc2a / 5557d7fb evidence). */
  recentReadyWorkNag?: { itemId: string; count: number };
  /** Last in-progress audit fingerprint we emitted (sorted item id list).
   *  When the current fingerprint matches AND no in_progress item was
   *  touched this turn, suppress — re-asking the agent to audit the same
   *  unchanged set every turn produces a status-transition ping-pong. */
  lastAuditFingerprint?: string;
  /** True when at least one `in_progress` work item was created or updated
   *  during this turn. Used together with `lastAuditFingerprint` to gate
   *  the audit nag — no movement on any in_progress row means the audit
   *  has nothing new to find. */
  inProgressTouchedThisTurn?: boolean;
}

export interface StopDirective {
  decision: "block";
  reason: string;
}

export type StopHookSideEffect =
  | { kind: "trigger-wait-point"; id: string }
  | { kind: "record-auto-commit-nag"; fingerprint: string }
  | { kind: "record-ready-work-nag"; itemId: string }
  | { kind: "record-audit-nag"; fingerprint: string };

export interface StopHookOutcome {
  directive: StopDirective | null;
  sideEffects: StopHookSideEffect[];
}

export function decideStopDirective(
  snapshot: ThreadSnapshot,
  builders: {
    buildCommitPointReason: (cp: CommitPoint) => string;
    buildNextWorkItemReason: (item: WorkItem) => string;
    /** Audit nudge: list of `in_progress` items the agent should reconcile
     *  before stopping (or moving on). Optional so tests / non-runtime
     *  callers can opt out of the audit branch entirely. */
    buildInProgressAuditReason?: (items: WorkItem[]) => string;
    /** Emitted for auto-commit: either a manually-placed commit_point row
     *  with mode="auto", OR the no-row auto_commit=true path (then `cp`
     *  is null). Optional so callers that don't route through the
     *  runtime (some unit tests) still work. */
    buildAutoCommitReason?: (cp: CommitPoint | null) => string;
    /** Emitted for read-heavy / no-write turns when wiki-capture is
     *  enabled. Optional so callers that don't wire this up (older tests)
     *  fall through to the plain Q&A "allow stop" path. */
    buildWikiCaptureReason?: () => string;
  },
): StopHookOutcome {
  const sideEffects: StopHookSideEffect[] = [];

  // Q&A turn: the agent answered or asked the user something with no
  // qualifying tool activity. The Stop here means "I'm waiting on you" —
  // suppress every directive (commit, auto-commit, in-progress audit,
  // ready-work) so the agent stays stopped until the user replies. The
  // wait-point side-effect is also skipped; it'll fire on the next turn
  // that actually does work.
  //
  // EXCEPTION: read-heavy exploration turns (`turnWasExploration`) get
  // the wiki-capture directive — the agent answered a "how does X work"
  // question and should durably capture findings into `.oxplow/notes/`
  // before the conversation moves on. The capture turn re-enters the
  // pipeline with `turnHadActivity === true` (it ran Write), so the
  // directive only fires once. `justEmittedWikiCapture` guards against
  // a second emission within the same user prompt.
  if (snapshot.turnHadActivity === false) {
    if (
      snapshot.turnWasExploration === true &&
      !snapshot.justEmittedWikiCapture &&
      snapshot.thread?.status === "active" &&
      builders.buildWikiCaptureReason
    ) {
      return {
        directive: { decision: "block", reason: builders.buildWikiCaptureReason() },
        sideEffects,
      };
    }
    return { directive: null, sideEffects };
  }

  const activeCommit = findActiveMarker(snapshot.commitPoints, snapshot.workItems, (cp) => cp.status !== "done");
  // Only the writer thread ever commits. Non-active threads share the worktree
  // read-only, so we must not prompt them to propose a commit; leave the
  // commit point pending for the thread that eventually becomes writer.
  if (activeCommit && activeCommit.status === "pending" && snapshot.thread?.status === "active") {
    if (activeCommit.mode === "auto" && builders.buildAutoCommitReason) {
      // Clean tree → nothing to commit. Skip the directive (wi-ec4c8e6f44fd).
      // Backfill of the junction runs via git-refs.changed on the prior
      // commit; the pending commit_point row stays "pending" until the
      // agent/runtime resolves it explicitly.
      if (snapshot.worktreeClean) {
        return { directive: null, sideEffects };
      }
      const fp = `cp:${activeCommit.id}`;
      if (snapshot.lastAutoCommitNagFingerprint === fp) {
        return { directive: null, sideEffects };
      }
      return {
        directive: { decision: "block", reason: builders.buildAutoCommitReason(activeCommit) },
        sideEffects: [...sideEffects, { kind: "record-auto-commit-nag", fingerprint: fp }],
      };
    }
    return {
      directive: { decision: "block", reason: builders.buildCommitPointReason(activeCommit) },
      sideEffects,
    };
  }

  // No commit_point row queued, but the thread is in auto_commit mode with
  // settled work — fire the auto-commit directive with cp=null. The agent
  // runs `mcp__oxplow__commit` with { auto: true } and the runtime commits
  // without touching a commit_point row. Only the writer thread commits.
  // Clean tree suppresses the misfire — the sha already landed via ad-hoc
  // git commit (Bash / Files panel) and the backfill path attaches the
  // junction rows. See wi-ec4c8e6f44fd.
  if (
    snapshot.autoCommit &&
    snapshot.thread?.status === "active" &&
    builders.buildAutoCommitReason &&
    hasSettledWork(snapshot.workItems) &&
    !snapshot.worktreeClean
  ) {
    const fp = "auto:no-cp";
    if (snapshot.lastAutoCommitNagFingerprint === fp) {
      return { directive: null, sideEffects };
    }
    return {
      directive: { decision: "block", reason: builders.buildAutoCommitReason(null) },
      sideEffects: [...sideEffects, { kind: "record-auto-commit-nag", fingerprint: fp }],
    };
  }

  const activeWait = findActiveMarker(snapshot.waitPoints, snapshot.workItems, (wp) => wp.status === "pending");
  if (activeWait) {
    sideEffects.push({ kind: "trigger-wait-point", id: activeWait.id });
    return { directive: null, sideEffects };
  }

  if (!snapshot.thread || snapshot.thread.status !== "active") {
    return { directive: null, sideEffects };
  }

  // Subagent-in-flight carve-out: when the orchestrator has dispatched a
  // `Task` subagent that hasn't returned, suppress the queue-management
  // nudges (audit + ready-work). Re-emitting them mid-flight makes the
  // parent ack each Stop with "still actively being worked" reply,
  // producing a visual loop. Markers (commit/wait points above) still
  // fire — those are user-placed and represent explicit work the agent
  // must address.
  if (snapshot.subagentInFlight) {
    return { directive: null, sideEffects };
  }

  // In-progress audit branch: when any work items are sitting in_progress,
  // the agent's job at Stop time is to reconcile them — confirm still
  // active, flip to human_check / blocked / ready / canceled as
  // appropriate. Tasks persist across turn boundaries; the audit is the
  // bookkeeping step that prevents stale in_progress rows piling up.
  const inProgress = snapshot.workItems.filter((item) => item.status === "in_progress");
  if (inProgress.length > 0 && builders.buildInProgressAuditReason) {
    const auditFp = inProgress.map((i) => i.id).sort().join("|");
    // Suppress when we already audited this exact set AND the agent
    // didn't touch any in_progress item this turn — re-asking produces
    // status-transition ping-pong with no new information.
    if (
      snapshot.lastAuditFingerprint === auditFp &&
      snapshot.inProgressTouchedThisTurn === false
    ) {
      return { directive: null, sideEffects };
    }
    return {
      directive: { decision: "block", reason: builders.buildInProgressAuditReason(inProgress) },
      sideEffects: [...sideEffects, { kind: "record-audit-nag", fingerprint: auditFp }],
    };
  }

  // No in_progress items left → free to surface ready work.
  const next = snapshot.readyWorkItems[0] ?? null;
  if (next) {
    if (shouldSuppressReadyWorkForJustRead(snapshot.readyWorkItems, snapshot.justReadReadySet)) {
      return { directive: null, sideEffects };
    }
    // Cap consecutive ready-work nags for the same item at 3. After
    // that the user clearly isn't going to act on it; further repeats
    // just burn tokens.
    const READY_NAG_CAP = 3;
    const recent = snapshot.recentReadyWorkNag;
    if (recent && recent.itemId === next.id && recent.count >= READY_NAG_CAP) {
      return { directive: null, sideEffects };
    }
    return {
      directive: { decision: "block", reason: builders.buildNextWorkItemReason(next) },
      sideEffects: [...sideEffects, { kind: "record-ready-work-nag", itemId: next.id }],
    };
  }

  return { directive: null, sideEffects };
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

function hasSettledWork(workItems: WorkItem[]): boolean {
  return workItems.some((item) => item.status === "human_check" || item.status === "done");
}

function shouldSuppressReadyWorkForJustRead(
  ready: WorkItem[],
  justRead: string[] | undefined,
): boolean {
  if (!justRead || justRead.length === 0) return false;
  const readySet = new Set(ready.map((item) => item.id));
  const readSet = new Set(justRead);
  if (readySet.size !== readSet.size) return false;
  for (const id of readSet) if (!readySet.has(id)) return false;
  return true;
}

function isTerminalStatus(item: WorkItem): boolean {
  return item.status === "done" || item.status === "canceled" || item.status === "archived" || item.status === "human_check";
}
