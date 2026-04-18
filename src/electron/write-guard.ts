import type { Batch } from "../persistence/batch-store.js";

/**
 * Tools that mutate the shared worktree. Every batch in a stream checks out
 * the same worktree, so agents running in non-writer batches must not call
 * these — their writes would land on top of the writer's in-progress edits.
 *
 * Bash is intentionally not in this set. Detecting destructive shell commands
 * is unreliable (redirects, pipes, nested shells, `sed -i`, build tools
 * writing artifacts, etc.), and blocking Bash outright would kill the utility
 * of read-only inspection (`git status`, `ls`, tests). Instead, non-writer
 * batches get a strong system-prompt instruction to avoid any shell command
 * that mutates state.
 */
export const WORKTREE_MUTATING_TOOLS: ReadonlySet<string> = new Set([
  "Write",
  "Edit",
  "MultiEdit",
  "NotebookEdit",
]);

export interface WriteGuardDenyBody {
  hookSpecificOutput: {
    hookEventName: "PreToolUse";
    permissionDecision: "deny";
    permissionDecisionReason: string;
  };
}

/**
 * Returns a Claude Code PreToolUse deny response when the batch is not the
 * stream's writer and the tool would mutate the shared worktree. Returns
 * `null` to let the tool call proceed.
 *
 * The caller reads `batch` fresh on every invocation so promoting another
 * batch to writer takes effect immediately on the next tool call — no agent
 * restart required.
 */
export function buildWriteGuardResponse(batch: Batch | null, toolName: string): WriteGuardDenyBody | null {
  if (!batch) return null;
  if (batch.status === "active") return null;
  if (!toolName) return null;
  // MCP tools (work items, backlog, etc.) write to the state DB, not the
  // worktree — always allowed.
  if (toolName.startsWith("mcp__")) return null;
  if (!WORKTREE_MUTATING_TOOLS.has(toolName)) return null;
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        "This batch is read-only — only the stream's writer batch may mutate the worktree. " +
        "Record the change as a note on the current work item via mcp__newde tools (or stop this turn). " +
        "Promote this batch to writer from the batch rail if you need to edit.",
    },
  };
}

/**
 * Extra prompt text appended to non-writer batches. Hook enforcement covers
 * Write/Edit/MultiEdit/NotebookEdit; this is the only line of defence for
 * Bash, which the hook leaves alone. Writer batches get no extra prompt —
 * default behaviour is already to act.
 */
export const NON_WRITER_PROMPT_BLOCK = [
  ``,
  `READ-ONLY BATCH — DO NOT MUTATE LOCAL STATE.`,
  `You are running in a non-writer batch that shares the git worktree with the writer batch. Any write you perform corrupts their in-progress work.`,
  ``,
  `The harness WILL DENY Write, Edit, MultiEdit, and NotebookEdit via a PreToolUse hook. Bash is allowed only for read-only inspection (ls, cat, grep, git status, git diff, git log, read-only test runs that don't write artifacts).`,
  ``,
  `You MUST NOT, via Bash or any other tool:`,
  `- write, create, delete, move, or truncate any file (>, >>, tee, sed -i, rm, mv, cp into the tree, mkdir, touch, …)`,
  `- run git subcommands that change refs, index, or the working tree (add, commit, checkout -- , reset, restore, stash apply, rebase, merge, cherry-pick, tag, branch, push)`,
  `- install dependencies, run build/format/lint commands that write artifacts or modify lockfiles (npm i, bun install, cargo build, etc.), or run tests that write snapshots/coverage`,
  `- set environment state or touch configs`,
  ``,
  `Read-only tools (Read, Grep, Glob) and mcp__newde__* (work items, backlog) remain fully available. Record proposed changes as a note on the current work item or as a new work item's description; the writer batch will pick them up.`,
  ``,
  `If you are unsure whether a command writes, do not run it.`,
].join("\n");
