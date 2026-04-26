import type { Thread } from "../persistence/thread-store.js";
import type { WorkItem } from "../persistence/work-item-store.js";

/**
 * Pure decision function for the Claude Stop-hook. Lives outside the
 * runtime god object so each branch can be unit-tested with a fixture
 * snapshot — and so the decision and the side effects (e.g. recording
 * the audit signature) stay separate. The runtime's `handleHookEnvelope`
 * builds a `ThreadSnapshot` from the live stores, passes it here, then
 * applies the returned `sideEffects` and returns `directive` to Claude.
 *
 * The pipeline runs in priority order:
 *   1. Awaiting-user gate: agent called `await_user`. Allow stop and
 *      suppress every directive.
 *   2. Q&A turn (no qualifying tool activity): allow stop, except for the
 *      wiki-capture exception (read-heavy / no-write turns get the
 *      capture directive).
 *   3. Filing-enforcement: write-intent edits without a filing call AND
 *      no in_progress claim → block.
 *   4. In-progress audit: any item in `in_progress` → block with audit
 *      directive (with no-change suppression by signature).
 *   5. Allow stop.
 *
 * The runtime never drives `git commit` and never queues commit/wait
 * markers. Commits are user-driven (CLI / Bash); cross-turn queue
 * progression is also user-driven via plain prompts or `/work-next`.
 *
 * `directive` of `null` lets Claude stop normally; otherwise it's the
 * hook body (typically `{ decision: "block", reason: string }`).
 */

export interface ThreadSnapshot {
  thread: Thread | null;
  /** Every work item in the thread — drives the in-progress audit branch
   *  (every item with status `in_progress` becomes a line in the audit
   *  list). */
  workItems: WorkItem[];
  /** Signature of the in_progress item set the runtime last emitted an audit
   *  directive for on this thread, in the same format the pipeline computes
   *  (sorted `id|updated_at|note_count` triples joined by newlines). When
   *  the next Stop sees an identical current signature, the audit directive
   *  is suppressed — no item changed, no agent activity, repeating the
   *  nudge produces a tight ack-loop that costs the user a wall of
   *  identical lines plus a lot of model tokens. Any change (set growing
   *  or shrinking, an item's `updated_at`, or `note_count` ticking up
   *  because a note landed) re-arms the audit. Optional; absent means
   *  "no prior audit recorded — fire normally." */
  lastInProgressAuditSignature?: string;
  /** True when the orchestrator has dispatched a `Task` subagent that
   *  hasn't yet returned. While set, the in-progress audit directive is
   *  suppressed — re-emitting it mid-flight produces a visual loop
   *  where the parent dutifully acks each Stop nudge while it's still
   *  waiting on the subagent. */
  subagentInFlight?: boolean;
  /** False when the just-ended turn fired no qualifying tool calls
   *  (write-intent, oxplow filing, dispatch). A pure Q&A / question-asking
   *  turn — the agent answered or asked the user something and there's
   *  nothing more to do until the user replies. When false, the entire
   *  Stop directive pipeline is skipped so we don't force-march the agent
   *  past a question. `undefined` is treated as "unknown → don't suppress"
   *  so behaviour stays stable when the activity flag wasn't threaded
   *  through (older tests, missing UserPromptSubmit, etc.). */
  turnHadActivity?: boolean;
  /** True when the turn read code (Read/Grep/Glob, read-only Bash) at
   *  least twice and produced zero write-intent activity. Combined with
   *  `turnHadActivity === false` this is the wiki-capture trigger: a
   *  read-heavy Q&A turn the agent should durably capture before stopping.
   *  Only checked when `turnHadActivity` is false; ignored otherwise so
   *  edits still take precedence on real-work turns. */
  turnWasExploration?: boolean;
  /** True when the wiki-capture directive already fired earlier on this
   *  thread and hasn't been cleared by a fresh user prompt. Suppresses a
   *  second emission so the agent isn't asked to capture twice in a row
   *  (the first emission either produced a note or got the agent's
   *  `oxplow-note: skipped` reply; either way it's settled). */
  justEmittedWikiCapture?: boolean;
  /** True when the agent explicitly signalled "I'm waiting on the user"
   *  via the `await_user` MCP tool during this turn. Top-priority Stop
   *  branch: when set, ALL directives are suppressed (audit, filing-
   *  enforcement, wiki-capture). The agent asked a real question and
   *  the user owns the next move — do not push onward. Cleared by the
   *  next UserPromptSubmit so the directive pipeline fires normally on
   *  the reply. */
  awaitingUser?: boolean;
  /** True when the turn invoked at least one write-intent tool
   *  (Edit/Write/MultiEdit, non-readonly Bash) on project files. Used
   *  by the filing-enforcement branch to detect "edited code without
   *  filing/transitioning a work item". */
  turnHadWrites?: boolean;
  /** True when the turn invoked at least one work-item-mutating tool
   *  that counts as "I filed/transitioned an item this turn":
   *  `create_work_item`, `update_work_item`, `complete_task`,
   *  `file_epic_with_children`, `transition_work_items`,
   *  `dispatch_work_item`. Used by the filing-enforcement branch. */
  turnHadFiling?: boolean;
}

export interface StopDirective {
  decision: "block";
  reason: string;
}

export type StopHookSideEffect =
  | { kind: "record-audit-signature"; signature: string };

export interface StopHookOutcome {
  directive: StopDirective | null;
  sideEffects: StopHookSideEffect[];
}

export function decideStopDirective(
  snapshot: ThreadSnapshot,
  builders: {
    /** Audit nudge: list of `in_progress` items the agent should reconcile
     *  before stopping. Optional so tests / non-runtime callers can opt
     *  out of the audit branch entirely. */
    buildInProgressAuditReason?: (items: WorkItem[]) => string;
    /** Emitted for read-heavy / no-write turns when wiki-capture is
     *  enabled. Optional so callers that don't wire this up (older tests)
     *  fall through to the plain Q&A "allow stop" path. */
    buildWikiCaptureReason?: () => string;
    /** Emitted when the turn made writes but never filed/transitioned a
     *  work item AND no in_progress item exists to claim the work.
     *  Optional so older callers fall through. */
    buildFilingEnforcementReason?: () => string;
  },
): StopHookOutcome {
  const sideEffects: StopHookSideEffect[] = [];

  // Awaiting-user gate: the agent explicitly signalled it's waiting on the
  // user via the `await_user` MCP tool. Allow stop and suppress every
  // directive. The agent asked a real question; the user owns the next
  // move. Once the user replies, UserPromptSubmit clears the flag and the
  // pipeline runs normally on the reply.
  if (snapshot.awaitingUser) {
    return { directive: null, sideEffects };
  }

  // Q&A turn: the agent answered or asked the user something with no
  // qualifying tool activity. Allow stop so the agent stays stopped until
  // the user replies.
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

  if (!snapshot.thread || snapshot.thread.status !== "active") {
    return { directive: null, sideEffects };
  }

  // Subagent-in-flight carve-out: when the orchestrator has dispatched a
  // `Task` subagent that hasn't returned, suppress the in-progress audit.
  // Re-emitting it mid-flight makes the parent ack each Stop with "still
  // actively being worked" reply, producing a visual loop.
  if (snapshot.subagentInFlight) {
    return { directive: null, sideEffects };
  }

  // Filing-enforcement branch: the turn made write-intent edits to project
  // files but never created/transitioned a work item AND no in_progress
  // item exists to claim the work. Block with a hard directive — the
  // agent must file (or transition) an item or revert before stopping.
  // This sits BEFORE the in-progress audit so the audit branch can
  // continue to handle the legitimate "filed and still open" path.
  const inProgressAll = snapshot.workItems.filter((item) => item.status === "in_progress");
  if (
    snapshot.turnHadWrites &&
    !snapshot.turnHadFiling &&
    inProgressAll.length === 0 &&
    builders.buildFilingEnforcementReason
  ) {
    return {
      directive: { decision: "block", reason: builders.buildFilingEnforcementReason() },
      sideEffects,
    };
  }

  // In-progress audit branch: when any work items are sitting in_progress,
  // the agent's job at Stop time is to reconcile them — confirm still
  // active, flip to human_check / blocked / ready / canceled as
  // appropriate. Tasks persist across turn boundaries; the audit is the
  // bookkeeping step that prevents stale in_progress rows piling up.
  const inProgress = inProgressAll;
  if (inProgress.length > 0 && builders.buildInProgressAuditReason) {
    const signature = computeAuditSignature(inProgress);
    if (snapshot.lastInProgressAuditSignature === signature) {
      // Nothing changed since the last audit fire on this thread — no agent
      // activity ticked any item's updated_at or note_count, and the set
      // membership is identical. Re-emitting the same nudge is the
      // ack-loop the user hit; suppress and leave the recorded signature
      // alone so a real change re-arms it.
      return { directive: null, sideEffects };
    }
    return {
      directive: { decision: "block", reason: builders.buildInProgressAuditReason(inProgress) },
      sideEffects: [...sideEffects, { kind: "record-audit-signature", signature }],
    };
  }

  // No in_progress items, nothing to enforce → allow stop.
  // Cross-turn queue progression is the user's call: they prompt the
  // agent (or run `/work-next`) when they want the next ready item
  // picked up. Commits are user-driven via CLI / Bash.
  return { directive: null, sideEffects };
}

/** Per-thread fingerprint of the in_progress set used to detect "nothing
 *  changed since the last audit fire" and skip a duplicate nudge. Sort by
 *  id for stable ordering regardless of `listItems` ordering tweaks; combine
 *  `updated_at` (covers status/title/AC edits via update_work_item /
 *  complete_task) and `note_count` (covers add_work_note, which doesn't
 *  bump updated_at). */
export function computeAuditSignature(items: WorkItem[]): string {
  return items
    .map((item) => `${item.id}|${item.updated_at}|${item.note_count}`)
    .sort()
    .join("\n");
}
