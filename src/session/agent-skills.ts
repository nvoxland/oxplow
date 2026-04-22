// Model-invoked Claude Code skills shipped with the plugin. Unlike the
// always-on system prompt, skills are pulled in only when Claude's router
// matches the skill's `description` against what the agent is about to do —
// so policy text that only matters for overrides lives here and doesn't
// tax every turn.
//
// As of the "observational newde" reset, most of what these skills used
// to enforce is now auto-synthesized by the runtime (first write-intent
// tool call auto-files a work item; Stop auto-completes it). The skills
// are deliberately minimal — they only describe MANUAL overrides. See
// the plan at `.context/agent-model.md` §"Auto-file / auto-complete".

/**
 * Single merged skill covering filing / lifecycle / dispatch. Pre-merge
 * these were three separate skill directories, each contributing one
 * line to the per-turn skill index. Collapsing them to `newde-runtime`
 * cuts the index replay cost (4 entries → 2, counting the still-
 * separate subagent-work-protocol skill).
 *
 * The three legacy names are kept as exports — tests and any external
 * callers referencing them still compile, and the constants document
 * the old surface. The plugin writer emits one SKILL.md at
 * `skills/newde-runtime/`; the legacy names no longer produce on-disk
 * directories.
 */
export const RUNTIME_SKILL_NAME = "newde-runtime";
export const TASK_FILING_SKILL_NAME = RUNTIME_SKILL_NAME;
export const TASK_LIFECYCLE_SKILL_NAME = RUNTIME_SKILL_NAME;
export const TASK_DISPATCH_SKILL_NAME = RUNTIME_SKILL_NAME;
export const SKILL_FILE = "SKILL.md";

export const SUBAGENT_PROTOCOL_SKILL_NAME = "newde-subagent-work-protocol";
export const SUBAGENT_PROTOCOL_SKILL_FILE = "SKILL.md";

export function buildSubagentProtocolSkill(): string {
  return `---
name: ${SUBAGENT_PROTOCOL_SKILL_NAME}
description: Standing protocol for subagents executing a newde work item. Loads on any wi-… id in a brief or on mcp__newde__update_work_item / add_work_note calls.
---

# Subagent protocol

- Mark the item \`in_progress\` on entry; \`human_check\` on exit.
- Return ONE line: \`newde-result: {"ok":true,"itemId":"wi-…","…":…}\`.
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
description: Newde runtime — work-item filing, status transitions, and orchestrator dispatch. Loads on mcp__newde__create_work_item, file_epic_with_children, update_work_item, add_work_note, read_work_options, or dispatch_work_item calls, and when composing a subagent brief.
---

# Filing newde work items

Newde auto-files a work item on the first write-intent tool call of a
turn, so you don't need to file one per request. Use the explicit
tools only when you want to:

- Split a bigger ask into an epic + children (\`file_epic_with_children\`).
- Pre-queue work the user wants done in a later turn (\`create_work_item\`).
- Add a distinct second item on top of the auto-filed one (rare).

The auto-filed row is adopted in place: calling \`create_work_item\`
or \`file_epic_with_children\` during the same turn overwrites the
existing row's title/description/AC/kind/priority and keeps its id.

## Shaping the row

- \`title\`: imperative, ≤60 chars (\`Fix wait-point overflow\`).
- \`description\`: what and why; keep it terse.
- \`acceptanceCriteria\`: one observable criterion per line.
- \`kind\`: \`epic\` only with children (use \`file_epic_with_children\`);
  otherwise \`task\`.
- \`priority\`: \`medium\` unless the user signalled otherwise.

# Work-item transitions

Newde auto-progresses the runtime-filed item: opens it at
\`in_progress\` on the first write-intent tool call, flips it to
\`human_check\` at Stop with a diff-derived summary note. You don't
need to narrate transitions.

Override manually via \`update_work_item\` when you want to:

- Park work as \`blocked\` with an explanatory note.
- Ship a hand-written completion note via \`complete_task\`
  (overrides the auto-summary).
- Mark a second \`in_progress\` item (rare — prefer one at a time).

Never self-mark \`done\` — the user owns that transition.

# Dispatch mode

- **Inline**: small fixes (≤20 lines, ≤2 files, no risk). Orchestrator
  edits directly.
- **Subagent**: anything bigger or risky. Call
  \`mcp__newde__dispatch_work_item({threadId, itemId})\` to get a ready
  brief; pass \`prompt\` to the general-purpose Agent tool. The brief
  already contains the item fields, AC, recent notes, and the
  subagent protocol preamble.

Subagents return a one-line \`newde-result: { ok, itemId, … }\`.
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
