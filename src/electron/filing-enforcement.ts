import type { Thread } from "../persistence/thread-store.js";
import { ALWAYS_WRITE_INTENT_TOOL_NAMES } from "./filing-enforcement-tools.js";

/**
 * PreToolUse filing-enforcement guard. Fires before an Edit / Write /
 * MultiEdit / NotebookEdit lands when the writer thread has no
 * `in_progress` work item to claim the change AND no filing call has
 * happened this turn. Catches the "started editing without filing"
 * misread at the moment it's actionable — the agent can file the
 * item, then re-issue the edit — instead of at end-of-turn when the
 * write has already shipped and the Work panel was lying for the
 * duration.
 *
 * Bash is intentionally excluded: shell commands frequently mutate
 * the worktree as a side effect (`git merge`, `git pull`, codegen
 * scripts, formatters) without representing the kind of authored
 * change the Work panel is supposed to track. The Stop-hook audit
 * for any lingering `in_progress` item still runs, so a turn that
 * legitimately edits via Bash under an open item is unaffected.
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
  filedThisTurn: boolean;
}

export function buildFilingEnforcementPreToolDeny(
  ctx: FilingEnforcementContext,
): FilingEnforcementDenyBody | null {
  if (!ctx.thread) return null;
  if (ctx.thread.status !== "active") return null;
  if (!ALWAYS_WRITE_INTENT_TOOL_NAMES.has(ctx.toolName)) return null;
  if (ctx.hasInProgressItem) return null;
  if (ctx.filedThisTurn) return null;
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
    `No \`in_progress\` work item exists and no filing call has fired this turn. The Work panel needs to honestly reflect what's shipping while it ships, not after.`,
    ``,
    `Pick one before re-issuing the edit:`,
    `  • New concern → \`mcp__oxplow__create_work_item\` with status=in_progress, then re-run ${toolName}. Close to human_check via \`complete_task\` when settled.`,
    `  • Fix/redo of a recently-closed human_check item → \`mcp__oxplow__update_work_item\` → status=in_progress on that item, then re-run ${toolName}. Close back to human_check when settled.`,
    `  • Already dispatched against a ready row → \`mcp__oxplow__update_work_item\` → status=in_progress on that row first.`,
    ``,
    `Do not file a placeholder "untracked work" item — describe the real change you're about to make.`,
  ].join("\n");
}
