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
 *   2. Q&A turn (no qualifying tool activity): allow stop. Wiki-capture
 *      no longer fires from the Stop hook — proactive capture is driven
 *      by the UserPromptSubmit `<wiki-capture-hint>` and the skill's
 *      keyword router instead.
 *   3. In-progress audit: any item in `in_progress` → block with audit
 *      directive (with no-change suppression by signature).
 *   4. Allow stop.
 *
 * Filing enforcement runs in the PreToolUse hook (see
 * `filing-enforcement.ts`), not here — by the time Stop fires, any
 * write has already shipped, so blocking at end-of-turn was both
 * post-hoc theatre and false-positive on Bash-driven worktree change
 * (`git merge`, codegen, formatters).
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
  /** True when the agent explicitly signalled "I'm waiting on the user"
   *  via the `await_user` MCP tool during this turn. Top-priority Stop
   *  branch: when set, ALL directives are suppressed (audit, filing-
   *  enforcement). The agent asked a real question and
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
  /** Stricter subset of `turnHadFiling`: true when at least one new
   *  row was filed at `ready` status this turn (the default). Drives
   *  the "filed but didn't ship" advisory branch — catches turns
   *  where the agent logged work as backlog when the user's
   *  instruction was to do it now. */
  turnFiledReadyItem?: boolean;
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
    /** Emitted when the turn filed at least one new `ready` row,
     *  edited zero project files, and has no `in_progress` item.
     *  Catches the "user said 'do X', agent filed it as backlog and
     *  stopped" misread. Optional so older callers fall through. */
    buildFiledButDidntShipReason?: () => string;
    /** Emitted when at least one epic is in `human_check`/`blocked`
     *  but has children still in `ready` or `in_progress`. The
     *  `classifyEpic` rollup pulls such epics back into To Do, so
     *  the rail counts lie until the children are closed too.
     *  Server-side cascade guards on `complete_task`/`update_work_item`
     *  prevent this on the happy path; the Stop-hook lint is a
     *  belt-and-suspenders backstop for legacy state and non-MCP
     *  mutations. Receives the offending epic+children pairs. */
    buildStaleEpicChildrenReason?: (
      pairs: Array<{ epic: WorkItem; staleChildren: WorkItem[] }>,
    ) => string;
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
  // the user replies. Wiki-capture used to be a backstop branch here;
  // it now lives at UserPromptSubmit time as a `<wiki-capture-hint>`
  // additionalContext block (see `buildWikiCaptureHint` in runtime.ts)
  // so the nudge fires before the answer rather than after.
  if (snapshot.turnHadActivity === false) {
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

  // Filing enforcement is no longer a Stop branch — it ran at the wrong
  // moment (post-hoc, after the write had already shipped) and false-
  // positived on Bash-driven worktree mutation that doesn't represent
  // authored change (`git merge`, codegen, formatters). Enforcement
  // moved to the PreToolUse hook (`buildFilingEnforcementPreToolDeny`),
  // which intercepts Edit/Write/MultiEdit/NotebookEdit before the write
  // lands when no in_progress item exists. The pipeline keeps the
  // `inProgressAll` snapshot below for the in-progress audit branch.
  const inProgressAll = snapshot.workItems.filter((item) => item.status === "in_progress");

  // Filed-but-didn't-ship advisory branch: the turn filed at least one
  // new `ready` row but made zero project edits AND has nothing
  // in_progress. This is the "user said do X, agent logged it as
  // backlog and stopped" misread. The advisory text includes an
  // explicit escape hatch for the legitimate "user said log this"
  // case so the directive isn't a wall.
  if (
    snapshot.turnFiledReadyItem &&
    !snapshot.turnHadWrites &&
    inProgressAll.length === 0 &&
    builders.buildFiledButDidntShipReason
  ) {
    return {
      directive: { decision: "block", reason: builders.buildFiledButDidntShipReason() },
      sideEffects,
    };
  }

  // Stale-epic-children advisory: any epic in human_check / blocked
  // whose children include ready / in_progress rows. The classifyEpic
  // rollup will pull such epics back into To Do, hiding the
  // closed-epic state from the rail counts. Fires when the bad state
  // exists at Stop time regardless of what the turn did — this is
  // catching legacy state, not turn activity. Server-side cascade
  // guards on the MCP tools prevent fresh cases.
  const staleEpicPairs = findStaleEpicChildrenPairs(snapshot.workItems);
  if (staleEpicPairs.length > 0 && builders.buildStaleEpicChildrenReason) {
    return {
      directive: { decision: "block", reason: builders.buildStaleEpicChildrenReason(staleEpicPairs) },
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

/**
 * Find every epic that's been closed (human_check / blocked) but
 * still has at least one child sitting in `ready` or `in_progress`.
 * Used by the stale-epic-children Stop-hook advisory — pure helper
 * so callers can also surface the same data in UI banners
 * (PlanWorkPage / WorkGroupList) and tests can assert on the shape.
 */
export function findStaleEpicChildrenPairs(
  items: WorkItem[],
): Array<{ epic: WorkItem; staleChildren: WorkItem[] }> {
  const pairs: Array<{ epic: WorkItem; staleChildren: WorkItem[] }> = [];
  for (const epic of items) {
    if (epic.kind !== "epic") continue;
    if (epic.status !== "human_check" && epic.status !== "blocked") continue;
    const staleChildren = items.filter(
      (child) => child.parent_id === epic.id && (child.status === "ready" || child.status === "in_progress"),
    );
    if (staleChildren.length > 0) pairs.push({ epic, staleChildren });
  }
  return pairs;
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
