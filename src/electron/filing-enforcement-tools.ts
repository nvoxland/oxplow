/**
 * Tool names that require a tracked work item before they can land.
 * Edit/Write/MultiEdit/NotebookEdit produce real authored change to
 * the worktree and are expected to be attributable to a specific
 * effort. Bash is intentionally excluded — shell commands mutate the
 * tree as a side effect (`git merge`, codegen, formatters) without
 * representing authored work. See `filing-enforcement.ts` and
 * `runtime.ts`'s PreToolUse handler.
 *
 * Kept in its own module so the pure guard helper can import it
 * without pulling the runtime god-object's dependency surface.
 */
export const ALWAYS_WRITE_INTENT_TOOL_NAMES: ReadonlySet<string> = new Set([
  "Write",
  "Edit",
  "MultiEdit",
  "NotebookEdit",
]);
