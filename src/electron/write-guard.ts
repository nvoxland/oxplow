import { resolve } from "node:path";
import type { Thread } from "../persistence/thread-store.js";
import { isInsideWorktree } from "./runtime-paths.js";

/**
 * Tools that mutate the shared worktree. Every thread in a stream checks out
 * the same worktree, so agents running in non-writer threads must not call
 * these — their writes would land on top of the writer's in-progress edits.
 *
 * Bash is intentionally not in this set. Detecting destructive shell commands
 * is unreliable (redirects, pipes, nested shells, `sed -i`, build tools
 * writing artifacts, etc.), and blocking Bash outright would kill the utility
 * of read-only inspection (`git status`, `ls`, tests). Instead, non-writer
 * threads get a strong system-prompt instruction to avoid any shell command
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

export interface WriteGuardContext {
  /** Absolute path to the project root (i.e. the shared worktree root). */
  projectDir?: string;
  /** Raw `tool_input` from the PreToolUse payload. */
  toolInput?: unknown;
}

/**
 * Extract the absolute target path for a Write/Edit/MultiEdit/NotebookEdit
 * tool call. Returns null if no path was provided (in which case we can't
 * prove it's safe, so we fall back to deny behaviour).
 */
function extractAbsTargetPath(toolInput: unknown, projectDir: string): string | null {
  if (!toolInput || typeof toolInput !== "object") return null;
  const obj = toolInput as Record<string, unknown>;
  const raw =
    (typeof obj.file_path === "string" && obj.file_path) ||
    (typeof obj.notebook_path === "string" && obj.notebook_path) ||
    (typeof obj.path === "string" && obj.path) ||
    null;
  if (!raw) return null;
  return resolve(projectDir, raw);
}

/**
 * Returns a Claude Code PreToolUse deny response when the thread is not the
 * stream's writer and the tool would mutate the shared worktree. Returns
 * `null` to let the tool call proceed.
 *
 * The caller reads `thread` fresh on every invocation so promoting another
 * thread to writer takes effect immediately on the next tool call — no agent
 * restart required.
 *
 * For Write/Edit/MultiEdit/NotebookEdit, agent-private paths outside the
 * shared worktree AND outside the project's `.newde/` are allowed even on
 * read-only threads (e.g. writing to `~/.claude/plans/foo.md`). Bash stays
 * conservatively allowed regardless — the write guard doesn't try to parse
 * shell commands.
 */
export function buildWriteGuardResponse(
  thread: Thread | null,
  toolName: string,
  context: WriteGuardContext = {},
): WriteGuardDenyBody | null {
  if (!thread) return null;
  if (thread.status === "active") return null;
  if (!toolName) return null;
  // MCP tools (work items, backlog, etc.) write to the state DB, not the
  // worktree — always allowed.
  if (toolName.startsWith("mcp__")) return null;
  if (!WORKTREE_MUTATING_TOOLS.has(toolName)) return null;

  // When we know the project root, allow writes to agent-private paths that
  // don't touch the shared worktree (e.g. ~/.claude/plans/foo.md). A path
  // inside the project's .newde/ is still blocked — that's shared state.
  const { projectDir, toolInput } = context;
  if (projectDir) {
    const abs = extractAbsTargetPath(toolInput, projectDir);
    if (abs) {
      const newdeDir = resolve(projectDir, ".newde");
      const insideProject = isInsideWorktree(abs, projectDir);
      const insideNewde = isInsideWorktree(abs, newdeDir);
      if (!insideProject && !insideNewde) {
        return null;
      }
      return {
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "deny",
          permissionDecisionReason:
            `path \`${abs}\` is inside the shared worktree and this thread is read-only — ` +
            "only the stream's writer thread may mutate the worktree. " +
            "Record the change as a note on the current work item via mcp__newde tools (or stop this turn). " +
            "Promote this thread to writer from the thread rail if you need to edit.",
        },
      };
    }
  }

  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        "This thread is read-only — only the stream's writer thread may mutate the worktree. " +
        "Record the change as a note on the current work item via mcp__newde tools (or stop this turn). " +
        "Promote this thread to writer from the thread rail if you need to edit.",
    },
  };
}

/**
 * Extra prompt text appended to non-writer threads. Hook enforcement covers
 * Write/Edit/MultiEdit/NotebookEdit; this is the only line of defence for
 * Bash, which the hook leaves alone. Writer threads get no extra prompt —
 * default behaviour is already to act.
 */
export const NON_WRITER_PROMPT_BLOCK = [
  ``,
  `READ-ONLY THREAD — DO NOT MUTATE LOCAL STATE.`,
  `You are running in a non-writer thread that shares the git worktree with the writer thread. Any write you perform corrupts their in-progress work.`,
  ``,
  `The harness WILL DENY Write, Edit, MultiEdit, and NotebookEdit via a PreToolUse hook. Bash is allowed only for read-only inspection (ls, cat, grep, git status, git diff, git log, read-only test runs that don't write artifacts).`,
  ``,
  `You MUST NOT, via Bash or any other tool:`,
  `- write, create, delete, move, or truncate any file (>, >>, tee, sed -i, rm, mv, cp into the tree, mkdir, touch, …)`,
  `- run git subcommands that change refs, index, or the working tree (add, commit, checkout -- , reset, restore, stash apply, rebase, merge, cherry-pick, tag, branch, push)`,
  `- install dependencies, run build/format/lint commands that write artifacts or modify lockfiles (npm i, bun install, cargo build, etc.), or run tests that write snapshots/coverage`,
  `- set environment state or touch configs`,
  ``,
  `Read-only tools (Read, Grep, Glob) and mcp__newde__* (work items, backlog) remain fully available. Record proposed changes as a note on the current work item or as a new work item's description; the writer thread will pick them up.`,
  ``,
  `If you are unsure whether a command writes, do not run it.`,
].join("\n");
