import type { Thread } from "../persistence/thread-store.js";
import { ALWAYS_WRITE_INTENT_TOOL_NAMES } from "./filing-enforcement-tools.js";

/**
 * PreToolUse filing-enforcement guard. Fires before an Edit / Write /
 * MultiEdit / NotebookEdit lands when the writer thread has no
 * `in_progress` work item to claim the change. Catches the "started
 * editing without claiming work" misread at the moment it's
 * actionable — the agent files / transitions an item to in_progress,
 * then re-issues the edit — instead of at end-of-turn when the write
 * has already shipped and the Work panel was lying for the duration.
 *
 * Note: a `ready`-status filing call alone does NOT satisfy the
 * guard. `ready` is bookkeeping for later (a backlog row); only
 * `in_progress` is a commitment to ship now. Earlier versions of this
 * guard accepted "any filing call this turn" as sufficient, which let
 * the agent file a `ready` row and quietly edit against it without
 * ever transitioning. The `hasInProgressItem` predicate is computed
 * live from the work-item store on each PreToolUse, so a
 * `create_work_item` / `update_work_item` / `transition_work_items`
 * that lands at status=in_progress is reflected immediately.
 *
 * Bash is intentionally excluded: shell commands frequently mutate
 * the worktree as a side effect (`git merge`, `git pull`, codegen
 * scripts, formatters) without representing the kind of authored
 * change the Work panel is supposed to track. The Stop-hook audit
 * for any lingering `in_progress` item still runs, so a turn that
 * legitimately edits via Bash under an open item is unaffected.
 *
 * Edits during an in-flight git operation (merge / rebase /
 * cherry-pick / revert) are also exempt — the authored change is the
 * merge commit, not a separate work item, and forcing a filing call
 * dead-locks the agent that is mid-conflict-resolution. The runtime
 * passes `gitOperationInProgress` after stat-checking MERGE_HEAD and
 * friends in the worktree's gitdir.
 *
 * Read-only threads are out of scope here — the write-guard
 * (`buildWriteGuardResponse`) blocks them on a more fundamental
 * "you are not the writer" rule and runs first.
 */

export interface FilingEnforcementDenyBody {
  hookSpecificOutput: {
    hookEventName: "PreToolUse";
    permissionDecision: "deny";
    permissionDecisionReason: string;
  };
}

export interface FilingEnforcementContext {
  thread: Thread | null;
  toolName: string;
  hasInProgressItem: boolean;
  /**
   * Absolute path being written, when the tool input carries one
   * (Write/Edit/MultiEdit/NotebookEdit all do). Used to exempt writes
   * to the Plan-mode plan file (`~/.claude/plans/<slug>.md`) — that
   * file is owned by the harness's plan workflow, not by project work,
   * and the harness denies every other tool while plan mode is on, so
   * blocking the plan-file write here would dead-lock the workflow.
   */
  filePath?: string | null;
  /**
   * True when the writer's worktree is mid-merge / rebase / cherry-pick
   * / revert. Conflict resolution edits don't need a separate work
   * item — the authored change is the merge commit itself, and forcing
   * a filing call dead-locks the workflow when the agent is just
   * fixing markers.
   */
  gitOperationInProgress?: boolean;
}

/** True for paths under the harness's plan-mode plans directory. */
export function isPlanModePlanFile(filePath: string | null | undefined): boolean {
  if (!filePath) return false;
  // Stable convention surfaced via the plan-mode system message:
  // `~/.claude/plans/<slug>.md`. The harness owns the directory, so a
  // simple prefix match is sufficient — and intentionally narrow, so
  // the carve-out can't drift to other `.claude/` files.
  const home = process.env.HOME ?? "";
  if (!home) return false;
  const prefix = `${home}/.claude/plans/`;
  return filePath.startsWith(prefix) && filePath.endsWith(".md");
}

export function buildFilingEnforcementPreToolDeny(
  ctx: FilingEnforcementContext,
): FilingEnforcementDenyBody | null {
  if (!ctx.thread) return null;
  if (ctx.thread.status !== "active") return null;
  if (!ALWAYS_WRITE_INTENT_TOOL_NAMES.has(ctx.toolName)) return null;
  if (ctx.hasInProgressItem) return null;
  if (isPlanModePlanFile(ctx.filePath)) return null;
  if (ctx.gitOperationInProgress) return null;
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: buildFilingEnforcementPreToolReason(ctx.toolName),
    },
  };
}

export function buildFilingEnforcementPreToolReason(toolName: string): string {
  return [
    `BLOCKED: ${toolName} requires a tracked work item on this thread before edits can land.`,
    ``,
    `No \`in_progress\` work item exists on this thread. \`ready\`-status rows don't count — \`ready\` is backlog, \`in_progress\` is the actual claim. The Work panel needs to honestly reflect what's shipping while it ships, not after.`,
    ``,
    `Pick one before re-issuing the edit:`,
    `  • New concern → \`mcp__oxplow__create_work_item\` with status=in_progress, then re-run ${toolName}. Close to done via \`complete_task\` when settled.`,
    `  • Fix/redo of a recently-closed done item → \`mcp__oxplow__update_work_item\` → status=in_progress on that item, then re-run ${toolName}. Close back to done when settled.`,
    `  • Already dispatched against a ready row → \`mcp__oxplow__update_work_item\` → status=in_progress on that row first.`,
    ``,
    `Do not file a placeholder "untracked work" item — describe the real change you're about to make.`,
  ].join("\n");
}
