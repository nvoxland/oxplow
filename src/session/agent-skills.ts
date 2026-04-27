// Model-invoked Claude Code skills shipped with the plugin. Unlike the
// always-on system prompt, skills are pulled in only when Claude's router
// matches the skill's `description` against what the agent is about to do —
// so policy text that only matters for overrides lives here and doesn't
// tax every turn.
//
// The runtime no longer synthesizes work items or tracks per-turn rows.
// These skills describe when to file durable work items explicitly. See
// `.context/agent-model.md` for details.

/**
 * Single merged skill covering filing / lifecycle / dispatch. Pre-merge
 * these were three separate skill directories, each contributing one
 * line to the per-turn skill index. Collapsing them to `oxplow-runtime`
 * cuts the index replay cost (4 entries → 2, counting the still-
 * separate subagent-work-protocol skill).
 *
 * The three legacy names are kept as exports — tests and any external
 * callers referencing them still compile, and the constants document
 * the old surface. The plugin writer emits one SKILL.md at
 * `skills/oxplow-runtime/`; the legacy names no longer produce on-disk
 * directories.
 */
export const RUNTIME_SKILL_NAME = "oxplow-runtime";
export const TASK_FILING_SKILL_NAME = RUNTIME_SKILL_NAME;
export const TASK_LIFECYCLE_SKILL_NAME = RUNTIME_SKILL_NAME;
export const TASK_DISPATCH_SKILL_NAME = RUNTIME_SKILL_NAME;
export const SKILL_FILE = "SKILL.md";

export const SUBAGENT_PROTOCOL_SKILL_NAME = "oxplow-subagent-work-protocol";
export const SUBAGENT_PROTOCOL_SKILL_FILE = "SKILL.md";

export function buildSubagentProtocolSkill(): string {
  return `---
name: ${SUBAGENT_PROTOCOL_SKILL_NAME}
description: Standing protocol for subagents executing a oxplow work item. Loads on any wi-… id in a brief or on mcp__oxplow__update_work_item / add_work_note calls.
---

# Subagent protocol

- Mark the item \`in_progress\` on entry; \`done\` on exit.
- Return ONE line: \`oxplow-result: {"ok":true,"itemId":"wi-…","…":…}\`.
- Keep notes terse: what you did, not how.
- On blocker, set \`blocked\` and leave a note — do not retry silently.
`;
}

/**
 * Unified orchestrator-side skill covering filing, lifecycle, and
 * dispatch. Trigger phrases from the three legacy skills are combined
 * in the description so Claude Code's router still matches any of the
 * original invocation contexts. The body preserves every section —
 * nothing in the merge drops content, only the index line count.
 */
export function buildRuntimeSkill(): string {
  return `---
name: ${RUNTIME_SKILL_NAME}
description: Oxplow runtime — work-item filing, status transitions, and orchestrator dispatch. Loads on mcp__oxplow__create_work_item, file_epic_with_children, update_work_item, add_work_note, read_work_options, or dispatch_work_item calls, and when composing a subagent brief.
---

# Filing oxplow work items

Active agent turns render as live rows in the Work panel passively —
no synthesized work items. File durable work items explicitly when
you want to:

- Split pre-planned or multi-phase work into an epic + children
  (\`file_epic_with_children\`).
- Pre-queue work the user wants done in a later turn (\`create_work_item\`).
- Record a follow-up you noticed but can't fix right now.

## Task vs epic

Pick by structure, not by whether the work was planned first. Plenty
of plan-mode outputs describe a single task.

- **\`create_work_item\` with \`kind: "task"\`** — one coherent change,
  even if it touches a few files. Rename, bug fix, small feature in one
  subsystem. Sequential chores (edit → typecheck → test) are still one
  task, not sub-steps.
- **\`file_epic_with_children\`** — ≥3 sub-steps a reviewer would
  naturally check off independently: distinct phases, clear handoffs,
  or separable subsystems (e.g. schema → runtime → IPC → UI → docs).
  Each child closes to \`done\` on its own as it ships.
- Decision test: could a single child close to \`done\` and
  have the user meaningfully inspect just that piece? If yes, epic.
  If no, it's a task and the bullets are just an execution outline.
- Don't retroactively wrap a task in an epic mid-execution if it turns
  out to be small — just finish it.

## Shaping the row

- \`title\`: imperative, ≤60 chars (\`Fix login redirect loop\`).
- \`description\`: what and why; keep it terse.
- \`acceptanceCriteria\`: one observable criterion per line.
- \`kind\`: \`epic\` only with children (use \`file_epic_with_children\`);
  otherwise \`task\`.
- \`priority\`: \`medium\` unless the user signalled otherwise.

## One QA-separate concern per row

Siblings under an epic still need to be independently reviewable:
two things a reviewer would accept/reject separately go in two child
tasks, not one "misc" child. Same rule as top-level items.

# Work-item transitions

Mark an explicit item \`in_progress\` when you start executing it and
\`done\` (via \`update_work_item\` or \`complete_task\`) when
you finish. Use \`blocked\` for items parked on user input.

**Close the row in the same turn the work actually ships.** An
\`in_progress\` row with finished work parked in it looks stuck to the
user. Call \`complete_task\` the moment the code change lands —
don't wait for a later turn.

**Pass \`touchedFiles\` when you close.** \`complete_task\`,
\`update_work_item\`, and \`create_work_item\` all accept an optional
\`touchedFiles: string[]\` of repo-relative paths you edited for this
effort. The runtime attaches them to the closing effort so Local
History can attribute writes to this specific item when multiple
items ran in parallel. Skip only if you edited >100 files (the
assume-all fallback handles big change sets).

For retroactive splits or "file and close in one call" rows (where
the edits already shipped and you just want a durable row with
attribution), pass \`touchedFiles\` directly into \`create_work_item\`
along with \`status: "done"\` (or \`"blocked"\`) — the server
synthesizes the \`in_progress → target\` transition so attribution
lands exactly as it would for a normal close. Without
\`touchedFiles\`, items filed directly into \`done\` never open
an effort, so attribution is impossible; the Local History panel
falls back to "assume all" for that item.

Legitimate reasons to *stay* \`in_progress\` across a stop boundary:

- You have a question the user must answer before you can finish.
- The work is genuinely multi-turn and you're pausing partway through.

In either case, leave a note (\`add_work_note\`) explaining what's
pending so the stop-hook nudge suppresses itself — it only fires for
items the agent didn't touch during the turn.

## Talking about items in chat

When you mention a work item to the user, refer to it by its quoted
title (e.g. \`"Fix login redirect loop"\`), **never** by its \`wi-…\`
id. The id is an internal handle for tool calls; the user doesn't see
it in their UI and won't know what you're pointing at. This applies
everywhere: confirming a fix, asking whether to proceed, summarizing
what shipped, naming the item you just reopened, etc.

## Redos on a just-shipped item

When the user pushes back on work you just closed to \`done\`
(asks you to fix, redo, revert, or take a different approach to the
same concern), **reopen the existing item** — don't file a new one.

Flow:

1. \`update_work_item\` the item back to \`in_progress\` (this opens a
   fresh effort; the \`done → in_progress\` transition is the documented
   reopen path).
2. Do the new round of edits.
3. \`complete_task\` back to \`done\` with \`touchedFiles\` for the new
   effort.

The item row gets a second effort recording the redo, attributed
correctly. Filing a new "Fix the thing I just did" task fragments the
history and makes the Work panel lie about how many concerns the user
actually raised. A *new* concern still gets a new item — the rule is
scoped to "user rejected my last attempt at this same item."

# Dispatch mode

- **Inline**: small fixes (≤20 lines, ≤2 files, no risk). Orchestrator
  edits directly.
- **Subagent**: anything bigger or risky. Call
  \`mcp__oxplow__dispatch_work_item({threadId, itemId})\` to get a ready
  brief; pass \`prompt\` to the general-purpose Agent tool. The brief
  already contains the item fields, AC, recent notes, and the
  subagent protocol preamble.

Subagents return a one-line \`oxplow-result: { ok, itemId, … }\`.
Record that as a work note via \`add_work_note\`.
`;
}

// Legacy exports — kept so callers that imported the per-topic builders
// still compile. They now all return the merged skill body; the plugin
// writer dedupes via a single emit, but direct callers (e.g. tests)
// still get the expected frontmatter.
export const buildTaskFilingSkill = buildRuntimeSkill;
export const buildTaskLifecycleSkill = buildRuntimeSkill;
export const buildTaskDispatchSkill = buildRuntimeSkill;

/**
 * Plugin-emitted slash command — available in any project running
 * oxplow, not just the oxplow repo's `.claude/commands/`. Replaces the
 * old Stop-hook ready-work directive: instead of the harness force-
 * marching the agent onto the next ready item, the user types
 * `/work-next` (or composes it via `/loop /work-next`) when they want
 * the queue to advance.
 */
export const WORK_NEXT_COMMAND_NAME = "work-next";
export const WORK_NEXT_COMMAND_FILE = "work-next.md";

export function buildWorkNextCommand(): string {
  return `---
description: Pick up the next ready oxplow work item and dispatch it.
---

Call \`mcp__oxplow__read_work_options\` for this thread and dispatch
the resulting unit to a \`general-purpose\` subagent per the
\`oxplow-runtime\` skill. The skill carries the protocol (mark
\`in_progress\` before work, \`done\` after, never two items
\`in_progress\` at once); follow it.

If the tool returns \`{ mode: "empty" }\` there's nothing ready —
report that and stop.
`;
}
