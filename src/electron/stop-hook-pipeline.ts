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
 *   3. Writer thread with a ready work item: block, ask the agent to
 *      pick it up (auto-progression).
 *   4. Writer thread with a still-in_progress work item the agent didn't
 *      touch this turn (and no blocker raised): block and nudge the
 *      agent to either move it to `human_check` or leave a note
 *      explaining what's left.
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
  /** When true and the thread is the writer, the stop-hook automatically
   *  proposes a commit whenever settled work (human_check/done) exists
   *  but no commit point is in the queue. */
  autoCommit?: boolean;
  /** The raw user prompt for the current turn, if known. Used ONLY by the
   *  ready-work directive to suppress conversational-starter prompts
   *  (why/how/explain/…) — agents shouldn't be force-marched to the next
   *  ready item when the user is asking a question. Optional; absent means
   *  "no conversational suppression." */
  currentTurnPrompt?: string | null;
  /** Work-item ids the agent saw in the most recent `read_work_options` call
   *  during the immediately-preceding turn. When the current ready-set is
   *  identical, the ready-work directive is suppressed — the agent already
   *  has the list. Optional; absent means "no just-read suppression." */
  justReadReadySet?: string[];
  /** Cumulative `cache_read_input_tokens` across all closed turns for this
   *  thread. When ≥20M, the emitted ready-work directive carries a
   *  fork_thread hint so the orchestrator can shed the tail. Optional;
   *  absent / 0 means "no hint." */
  cumulativeCacheRead?: number;
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
  snapshot: ThreadSnapshot,
  builders: {
    buildCommitPointReason: (cp: CommitPoint) => string;
    buildNextWorkItemReason: (item: WorkItem, context: NextWorkItemContext) => string;
    buildHumanCheckNudgeReason?: (item: WorkItem) => string;
    /** Emitted for auto-commit: either a manually-placed commit_point row
     *  with mode="auto", OR the no-row auto_commit=true path (then `cp`
     *  is null). Optional so callers that don't route through the
     *  runtime (some unit tests) still work. */
    buildAutoCommitReason?: (cp: CommitPoint | null) => string;
  },
): StopHookOutcome {
  const sideEffects: StopHookSideEffect[] = [];

  const activeCommit = findActiveMarker(snapshot.commitPoints, snapshot.workItems, (cp) => cp.status !== "done");
  // Only the writer thread ever commits. Non-active threads share the worktree
  // read-only, so we must not prompt them to propose a commit; leave the
  // commit point pending for the thread that eventually becomes writer.
  if (activeCommit && activeCommit.status === "pending" && snapshot.thread?.status === "active") {
    if (activeCommit.mode === "auto" && builders.buildAutoCommitReason) {
      return {
        directive: { decision: "block", reason: builders.buildAutoCommitReason(activeCommit) },
        sideEffects,
      };
    }
    return {
      directive: { decision: "block", reason: builders.buildCommitPointReason(activeCommit) },
      sideEffects,
    };
  }

  // No commit_point row queued, but the thread is in auto_commit mode with
  // settled work — fire the auto-commit directive with cp=null. The agent
  // runs `mcp__newde__commit` with { auto: true } and the runtime commits
  // without touching a commit_point row. Only the writer thread commits.
  if (
    snapshot.autoCommit &&
    snapshot.thread?.status === "active" &&
    builders.buildAutoCommitReason &&
    hasSettledWork(snapshot.workItems)
  ) {
    return {
      directive: { decision: "block", reason: builders.buildAutoCommitReason(null) },
      sideEffects,
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

  const next = pickNextReadyItem(snapshot.readyWorkItems, snapshot.currentTurnStartedAt);
  if (next) {
    // Suppression rules (ready-work directive only — commit/wait are
    // unaffected and ran above).
    if (shouldSuppressReadyWorkForPrompt(snapshot.currentTurnPrompt)) {
      return { directive: null, sideEffects };
    }
    if (shouldSuppressReadyWorkForJustRead(snapshot.readyWorkItems, snapshot.justReadReadySet)) {
      return { directive: null, sideEffects };
    }
    const uiChangeNudge = turnTouchedUi(snapshot.currentTurnFilePaths);
    const baseReason = builders.buildNextWorkItemReason(next, { uiChangeNudge });
    const forkHint = buildForkHint(snapshot);
    const reason = forkHint ? `${baseReason}\n\n${forkHint}` : baseReason;
    return {
      directive: { decision: "block", reason },
      sideEffects,
    };
  }

  // No ready items left. If there's a still-in_progress item the agent
  // didn't touch this turn (and no blocker raised anywhere in the thread
  // this turn), nudge toward human_check so a forgotten settle doesn't
  // silently park the item. Opt-in via the builder — callers that don't
  // need the nudge can omit `buildHumanCheckNudgeReason`.
  if (builders.buildHumanCheckNudgeReason) {
    const stale = pickHumanCheckNudge(snapshot);
    if (stale) {
      return {
        directive: { decision: "block", reason: builders.buildHumanCheckNudgeReason(stale) },
        sideEffects,
      };
    }
  }

  return { directive: null, sideEffects };
}

/**
 * Pick the work item (if any) the Stop hook should nudge the agent to
 * settle. Fires only when:
 *   - the current turn has a known `started_at` (we can tell "touched this
 *     turn" from `updated_at`);
 *   - exactly one item is `in_progress` (the "sole in-progress" convention);
 *   - that item wasn't touched during the current turn (updated_at <
 *     turnStart); and
 *   - no item in the thread is `blocked` with an update timestamp ≥ the
 *     turn start (an agent-raised blocker suppresses the nudge — "not
 *     done yet, here's why" is the correct state, not a forgotten settle).
 */
function pickHumanCheckNudge(snapshot: ThreadSnapshot): WorkItem | null {
  const turnStartedAtIso = snapshot.currentTurnStartedAt;
  if (!turnStartedAtIso) return null;
  const turnStartMs = Date.parse(turnStartedAtIso);
  if (!Number.isFinite(turnStartMs)) return null;
  const inProgress = snapshot.workItems.filter((item) => item.status === "in_progress");
  if (inProgress.length !== 1) return null;
  const candidate = inProgress[0]!;
  const updatedMs = Date.parse(candidate.updated_at);
  if (Number.isFinite(updatedMs) && updatedMs >= turnStartMs) return null;
  const blockerRaisedThisTurn = snapshot.workItems.some((item) => {
    if (item.status !== "blocked") return false;
    const itemUpdated = Date.parse(item.updated_at);
    return Number.isFinite(itemUpdated) && itemUpdated >= turnStartMs;
  });
  if (blockerRaisedThisTurn) return null;
  return candidate;
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

function hasSettledWork(workItems: WorkItem[]): boolean {
  return workItems.some((item) => item.status === "human_check" || item.status === "done");
}

/**
 * Classifier for the ready-work directive. Three layers, evaluated in
 * order:
 *
 * 1. **Conversational-starter** — prompt begins with a question/lookup
 *    verb (why/how/explain/what/look/tell/show/can you/does/is/should/
 *    could/would). Case-insensitive, leading whitespace trimmed, word
 *    boundary required so `whyever` doesn't match.
 * 2. **Imperative-question** — anywhere in the prompt, patterns like
 *    `help me …`, `tell me about …`, `walk me through …` that read as
 *    directive-shaped but ask for explanation, not mutation.
 * 3. **No-change-verb default** — if neither (1) nor (2) fired AND the
 *    prompt contains no change-verb (fix/add/change/rename/delete/
 *    implement/build/remove/update/refactor/create/write), default to
 *    suppressed. Rationale: a turn that has no imperative verb of
 *    mutation is almost always discussion / planning / review; the
 *    ready-work force-march inverts the user's intent. Presence of a
 *    change-verb re-arms the directive.
 *
 * Kept as regex — an LLM classifier is a follow-up if this still
 * misfires. Anchoring with \b on both sides keeps noun forms ("a
 * change") from tripping the verb match.
 */
const CONVERSATIONAL_STARTER_RE =
  /^(why|how|explain|what|look|tell|show|can you|does|is|should|could|would)\b/i;

const IMPERATIVE_QUESTION_RE =
  /\b(help me|tell me|walk me through|show me)\b/i;

const CHANGE_VERB_RE =
  /\b(fix|add|change|rename|delete|implement|build|remove|update|refactor|create|write)\b/i;

function shouldSuppressReadyWorkForPrompt(prompt: string | null | undefined): boolean {
  if (typeof prompt !== "string") return false;
  const trimmed = prompt.replace(/^\s+/, "");
  if (trimmed.length === 0) return false;
  if (CONVERSATIONAL_STARTER_RE.test(trimmed)) return true;
  if (IMPERATIVE_QUESTION_RE.test(trimmed)) return true;
  // No change-verb default: if the prompt lacks any recognisable
  // mutation verb, treat it as discussion and suppress.
  if (!CHANGE_VERB_RE.test(trimmed)) return true;
  return false;
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

const FORK_HINT_THRESHOLD = 20_000_000;

function buildForkHint(snapshot: ThreadSnapshot): string | null {
  const cum = snapshot.cumulativeCacheRead;
  if (typeof cum !== "number" || cum < FORK_HINT_THRESHOLD) return null;
  const mStr = (cum / 1_000_000).toFixed(1);
  const threadId = snapshot.thread?.id ?? "<threadId>";
  return `note: this thread has burned ${mStr}M cache-read. If upcoming work is unrelated, consider newde__fork_thread({ sourceThreadId: "${threadId}", title: "...", summary: "short carry-over context" })`;
}

function isTerminalStatus(item: WorkItem): boolean {
  return item.status === "done" || item.status === "canceled" || item.status === "archived" || item.status === "human_check";
}

