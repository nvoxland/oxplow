// Model-invoked Claude Code skills shipped with the plugin. Unlike the
// always-on system prompt, skills are pulled in only when Claude's router
// matches the skill's `description` against what the agent is about to do —
// so policy text that only matters when filing tasks can live here without
// taxing every turn.

export const TASK_MANAGEMENT_SKILL_NAME = "newde-task-management";
export const TASK_MANAGEMENT_SKILL_FILE = "SKILL.md";

export function buildTaskManagementSkill(): string {
  return `---
name: ${TASK_MANAGEMENT_SKILL_NAME}
description: File and manage newde work items. Use whenever the user hands you a task, describes a plan you intend to execute, proposes a multi-step change, or whenever you discover follow-up work mid-turn. Covers how to shape epics/tasks/subtasks, how to pick the right batch, how to write observable acceptance criteria, and the status conventions (never self-mark done; use human_check).
---

# Filing and managing newde work items

The mechanism is the \`newde__*_work_item\` MCP tools. This skill is the
**policy layer** — when to file, how to shape the item, which batch/stream
it belongs in. Tool argument details live in the tool descriptions; don't
duplicate them here.

## When to create a work item

File a new work item **before** you start coding whenever any of these are
true:

- The user handed you a task. Every top-level request = one work item.
- You discovered follow-up work mid-turn (bug, refactor, doc drift). Don't
  silently fold it into the current item — file it and link it with
  \`discovered_from\` so the original's scope stays honest.
- You are about to execute a plan of more than one or two steps. Convert
  the plan into an epic + child tasks before you touch code (see
  "Plans → epics" below).

Skip only these cases:

- A one-shot factual question ("what does this file do?") — no task
  intent.
- You're already inside an in_progress item whose acceptanceCriteria
  already cover the ask.

## Plans → epics

When the work naturally decomposes into multiple ordered steps, **model
the plan as an epic with child tasks** instead of cramming everything
into one task:

1. Create the epic (\`kind: "epic"\`) with the overall goal as the title
   and the high-level approach in the description. Acceptance criteria
   on the epic are the "done" bar for the whole initiative — keep them
   observable ("feature ships behind flag X", not "implement feature").
2. Create each step as a task with \`parentId\` set to the epic's id.
   Each child task gets its own observable acceptance criteria. The
   child's description is the "how"; the epic's description is the
   "why".
3. If a task itself has meaningful sub-steps the user should see in the
   history, create them as \`subtask\` children of that task. Don't
   nest subtasks three levels deep — past one level of nesting, prefer
   another epic.
4. Use \`blocks\` links between children when there's a hard ordering
   (migration before the feature that uses it). Sibling tasks without
   hard ordering should not be over-linked — the ready queue sorts on
   \`sort_index\`/priority already.

For single-step work (a rename, a one-line fix, a trivial doc tweak)
don't bother with an epic — a single task is clearer.

## Choosing the batch and stream

Most of the time the right batch is the session's current one — the
\`batchId\` shown in your session context. Two non-obvious cases:

- **Cross-batch filing.** If you're in an agent that isn't the writer
  (status != active) and you discover work that belongs to another
  batch, call \`newde__get_batch_context\` with no args to list peer
  batches, then \`create_work_item\` with that peer's \`batchId\`. The
  tool validates the batch exists; you don't need to switch streams.
- **Backlog (stream-less).** The UI surfaces a backlog for work that
  isn't yet committed to any stream. You normally don't file into the
  backlog from an MCP tool — \`create_work_item\` always requires a
  batchId. If the user explicitly says "file this in the backlog,"
  that's a UI action, not something the agent should try to route via
  MCP.

Don't move an item between batches yourself — the UI handles that via
drag-and-drop. If you notice an item is in the wrong batch, add a note
(\`add_work_note\`) explaining why and let the user move it.

## Kind rubric

See \`.newde/runtime/claude-plugin/AGENT_GUIDE.md\` for the short kind
catalog. Summary:

- **task** — the default.
- **epic** — when the work expands to multiple tasks (see above).
- **subtask** — a step inside a large task.
- **bug** — a defect, phrased as an observation.
- **note** — something the user should see in the history but never
  enters the queue.

If you're unsure between task and epic, pick **task**. Promote to epic
later by creating the epic and re-parenting.

## Priority

Default is \`medium\`. Reserve \`urgent\` for genuine blockers (broken
build, failing CI on main, data-loss bug). Use \`high\` sparingly for
things the user explicitly called out as time-sensitive. \`low\` is
fine for nice-to-haves the user won't miss if they slip.

Don't inflate priority to bump your own item up the queue — the user
orders the queue; priority is signal, not control.

## Acceptance criteria

Write them **observable**: a human reading just the criterion should be
able to tell whether it's met without re-reading the description.

- Good: "grep finds no remaining references to the old flag name",
  "the Archived section header renders only when N > 0", "\`bun
  test\` passes locally".
- Bad: "refactor the loader", "clean up the code", "make it work".

One criterion per line, plain text. Don't embed XML/Markdown structure
— the tool will accept it, but the UI renders it as-is and the
structure doesn't buy anything.

For epics, acceptance criteria describe the initiative's done bar, not
"all children are done" — that's implicit.

## Status conventions

- New items default to \`ready\`. Don't pre-mark \`in_progress\` on
  creation unless you're literally about to start it this turn.
- **Set \`in_progress\` the moment you start.** The Stop hook uses the
  sole-in-progress item to attribute file changes correctly, so don't
  leave old items in_progress while you work on a new one.
- **Never self-mark \`done\`.** Push to \`human_check\` when you
  believe the acceptance criteria are met. The user marks done after
  reviewing. If you set done, you will be corrected.
- **\`blocked\`** is for "can't proceed until X" (missing context,
  waiting on another item, waiting on the user). Add a note that
  explains what the blocker is.
- **\`canceled\`** is for work the user has decided against. Prefer
  \`delete_work_item\` for things *you* decided against before anyone
  else saw them.
- **\`archived\`** hides the item from the default Work view. Use it
  to clear out clutter (old bugs, resolved notes) without losing the
  history. Don't archive items the user hasn't seen yet.

Rewrite title / description / acceptance criteria whenever your
understanding shifts. Stale state in the queue costs more than a tool
call to fix it.

## Referring to items in user-visible output

The user sees work items in the UI by **title**, not id. When you mention an
item in chat or summaries, quote the title ("Slim MCP responses"), not the
id. Ids are for MCP calls only — surfacing them to the user is noise.

## Notes and history

Use \`add_work_note\` at meaningful milestones — decisions you made,
surprises you hit, snags that changed the approach. Don't log every
step-by-step; the user reads the note stream the way you'd skim a
pull-request description.

## What not to file

- A single message reply ("explain X"): no task — just answer.
- Your own scratch todos for the current turn: use the built-in
  TaskCreate tool instead. \`newde\` work items are the
  *user-visible, durable* record.
- Things the user explicitly asked you to *skip*: if they say "don't
  file this," don't file it.

## Subagent dispatch protocol

You are the **orchestrator**. You never do Read/Edit/Bash/test work
directly — that all happens inside subagents, so your context stays flat
across a long work queue.

**Step 0 — file before you dispatch.** If the user just handed you a new
task that isn't in the queue yet, create the work items *first* (see
"Plans → epics" above) before calling \`read_work_options\`. Never skip
straight from plan approval to subagent dispatch without filing. The user
monitors progress through the work-item UI; if nothing is filed, they
have no visibility.

**For each work unit:**

1. Call \`newde__read_work_options\` (batchId=your batch) to get the next
   dispatch unit. Three possible shapes:
   - \`{ mode: "epic", epic, children }\` — dispatch the entire epic as one unit.
   - \`{ mode: "standalone", items }\` — pick one item, or a link-related
     cluster, to dispatch together.
   - \`{ mode: "empty" }\` — nothing left; allow stop.

2. Assemble a subagent brief containing: item ids, titles, descriptions,
   acceptance criteria, and these standing instructions:
   - Mark each item \`in_progress\` via \`mcp__newde__update_work_item\` before starting.
   - Mark \`human_check\` when acceptance criteria are met.
   - When dispatched as an epic unit: after all child tasks are marked
     \`human_check\`, mark the epic itself \`human_check\` too.
   - Use \`mcp__newde__add_work_note\` for decisions, surprises, or summaries.
   - Use \`mcp__newde__propose_commit\` when a commit point is due.
   - Return a short plain-text summary of what was done.

3. Launch one \`general-purpose\` subagent with that brief.

4. When the subagent returns, call \`newde__add_work_note\` on each item
   with the returned summary.

5. Loop from step 1.

Never mark items \`in_progress\` yourself before dispatching — let the
subagent do it so file-change attribution works correctly.
`;
}
