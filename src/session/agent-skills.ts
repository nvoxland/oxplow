// Model-invoked Claude Code skills shipped with the plugin. Unlike the
// always-on system prompt, skills are pulled in only when Claude's router
// matches the skill's `description` against what the agent is about to do —
// so policy text that only matters when filing tasks can live here without
// taxing every turn.
//
// The old monolithic `newde-task-management` skill was split into three
// smaller, triggered-on-demand skills so the common case loads ~1k of
// policy instead of ~4k: filing shape (create_work_item, epics, kind
// rubric, acceptance criteria), lifecycle (status transitions, epic
// rollup, note discipline), and dispatch (orchestrator vs subagent
// mode). Subagent-side invariants still live in their own skill below.

export const TASK_FILING_SKILL_NAME = "newde-task-filing";
export const TASK_LIFECYCLE_SKILL_NAME = "newde-task-lifecycle";
export const TASK_DISPATCH_SKILL_NAME = "newde-task-dispatch";
export const SKILL_FILE = "SKILL.md";

export const SUBAGENT_PROTOCOL_SKILL_NAME = "newde-subagent-work-protocol";
export const SUBAGENT_PROTOCOL_SKILL_FILE = "SKILL.md";

// Standing context for subagents dispatched off a newde work item. The
// orchestrator's brief references this skill instead of inlining the
// protocol, shrinking per-dispatch briefs from ~1000 tokens to ~150. The
// description is tuned to fire whenever a subagent is about to work on an
// item whose ids arrived in its brief — any mention of "work item",
// "in_progress", or the mcp__newde__ tool surface will match.
export function buildSubagentProtocolSkill(): string {
  return `---
name: ${SUBAGENT_PROTOCOL_SKILL_NAME}
description: Standing protocol for subagents executing newde work items. Loads whenever you see a work-item id (wi-…) in your brief, whenever you're told to mark an item in_progress/human_check/blocked, or whenever you call mcp__newde__update_work_item / mcp__newde__add_work_note. Covers status lifecycle, attribution rules, note discipline, and the blocked-on-error rule.
---

# Subagent work-item protocol

You were dispatched by an orchestrator with a brief listing work item
ids + titles + task-specific instructions. The brief deliberately does
NOT repeat the standing protocol — it lives here. Follow this protocol
for every item in the brief.

## Status lifecycle (mandatory order)

For each item, in this exact order:

1. **Mark \`in_progress\` FIRST.** Call
   \`mcp__newde__update_work_item\` with \`status: "in_progress"\`
   **before** any Read, Grep, Bash, Edit, or other tool call related to
   the item. The UI's "In Progress" section depends on this — if you
   do the work first and only flip the status at the end, the item
   visibly skips the In Progress section (ready → human_check in a
   blink) and the user loses visibility into what you're doing.
2. Do the investigation / edits / tests.
3. **Mark \`human_check\` the moment acceptance criteria are met —
   before returning to the orchestrator.** \`in_progress\` means you
   are actively working on it *right now*, not "I made progress."
   Don't return a "done, tests pass" summary while leaving the item
   parked in IN PROGRESS. Never mark \`done\` yourself; the user marks
   done after reviewing.

## One in_progress at a time

Never have two items marked \`in_progress\` simultaneously. Finish the
current one (\`human_check\` or \`blocked\`) before starting the next.

**Why it matters:** the Stop hook attributes file-change snapshots to
the sole in-progress item. If two are in-progress, attribution breaks
and local history can't tell which task produced which change.

## Epics

When the brief dispatches an epic unit (mode: "epic" in
read_work_options), after all child items are \`human_check\`, mark
the **epic itself** \`human_check\` too. Don't leave epics with all
children settled but the parent still \`in_progress\`/\`ready\`.

## Notes at meaningful milestones

Use \`mcp__newde__add_work_note\` for decisions, surprises, tricky
design choices, or a terse end-of-item summary — the way you'd leave a
pull-request comment. Don't log every step; the note stream is read
like a PR description, not a transcript.

## Blocked on error

If you hit an error you cannot resolve, **mark the item \`blocked\`
and add a note with the error details** before stopping. Do NOT leave
the item \`in_progress\` while stuck — that keeps the queue from
advancing and misleads the user about what's actively running.

## Commit points

If you encounter a commit point while working, draft a concise commit
message in your chat reply, ask the user to approve, and call
\`mcp__newde__commit\` with the commit_point_id + message once they
do. Do not run \`git commit\` yourself.

## Returning to the orchestrator

When done, return a short plain-text summary of what was done (the
orchestrator appends it as a note via \`add_work_note\`). Token
savings are biggest when the summary is tight.
`;
}

// SKILL 1 — filing. Triggers on creating + shaping work items.
export function buildTaskFilingSkill(): string {
  return `---
name: ${TASK_FILING_SKILL_NAME}
description: File newde work items — when to create one, how to pick kind/priority/acceptance-criteria, when to file as an epic with child tasks, and how to choose the right batch. Triggers on creating work items via mcp__newde__create_work_item, planning multi-step changes, being handed a new task by the user, or discovering follow-up work mid-turn.
---

# Filing newde work items

The mechanism is \`mcp__newde__create_work_item\` (and sibling tools).
This skill is the **policy layer** — when to file, how to shape the
item, which batch it belongs in. Argument details live in each tool's
own description; don't duplicate them here.

## Filing is ALWAYS inline. Never dispatch a subagent to file.

**Creating, updating, noting, and status-flipping a work item are
always performed directly by the orchestrator** — call
\`create_work_item\`, \`update_work_item\`, \`add_work_note\` yourself.
Do NOT launch a \`general-purpose\` (or any other) subagent just to
wrap these tool calls.

Why: a subagent dispatch costs ~10-15k tokens (system prompt + skill
reload + brief + return) and ~15-25s of latency for what is a 2-3
MCP-call operation. If you already have the \`mcp__newde__*\` tools
loaded in your context, use them.

**Subagents are for *executing* a filed work item**, not for creating
one. See the \`newde-task-dispatch\` skill for the execution flow.

## When to create a work item

**If you are making any file changes, create a work item first — no
exceptions.** A two-line edit is still a work item. The user monitors
what you are doing through the work-item UI; if nothing is filed, they
have no visibility into the change. Traceability is the point —
local-history snapshots attribute file changes to the sole in-progress
work item, so every change needs a task to attribute to.

The only time you skip filing is when you are answering a question and
making zero file changes ("what does this function do?", "explain X").
Everything else gets a work item.

- **Single change** (a one-line fix, a rename, a trivial tweak) → one
  \`task\`. Don't bother with an epic.
- **Multi-step work** (a feature, a refactor, a bug that touches
  several files) → one \`epic\` with child tasks (see "Plans → epics"
  below).
- **Follow-up work discovered mid-turn** → file it and link it with
  \`discovered_from\` so the original's scope stays honest.

## Batching

Related small fixes belong in ONE task, not one-per-fix. "Fix 4 test
fixtures missing custom_prompt" is a single task; filing four
near-identical items clutters history and makes the work queue look
busier than it is. The inverse failure is also real — a task like
"refactor the persistence layer" is too broad to trace; decompose it
into an epic with children. Aim for tasks whose scope you could
describe in a one-line commit message.

## Plans → epics

**An epic without children is a bug.** If you file \`kind: "epic"\`,
you MUST also file its child tasks in the same turn — before starting
execution. An epic with no children shows up in the UI as one opaque
IN PROGRESS row for the whole initiative; the user can't see which
phase landed when, can't flag concerns mid-stream, and can't mark
individual phases \`human_check\` as they finish. If you can't break
the work into at least two children, it isn't an epic — file it as a
single \`task\`.

When the work naturally decomposes into multiple ordered steps, model
the plan as an epic with child tasks instead of cramming everything
into one task:

1. Create the epic (\`kind: "epic"\`) with the overall goal as the
   title and the high-level approach in the description. Acceptance
   criteria on the epic are the "done" bar for the whole initiative —
   keep them observable ("feature ships behind flag X", not
   "implement feature").
2. **Create each step as a task with \`parentId\` set to the epic's
   id — file them up front, before starting execution, not
   as-you-go.** Each child task gets its own observable acceptance
   criteria. The child's description is the "how"; the epic's
   description is the "why". Rough sizing heuristic: if a refactor
   touches N subsystems (migration, store, UI, docs), expect roughly
   N children.
3. If a task itself has meaningful sub-steps the user should see in
   the history, create them as \`subtask\` children of that task.
   Don't nest subtasks three levels deep — past one level of nesting,
   prefer another epic.
4. Use \`blocks\` links between children when there's a hard ordering
   (migration before the feature that uses it). Sibling tasks without
   hard ordering should not be over-linked — the ready queue sorts on
   \`sort_index\`/priority already.

(Status transitions + epic rollup rules live in the
\`newde-task-lifecycle\` skill.)

## Choosing the batch and stream

Most of the time the right batch is the session's current one — the
\`batchId\` shown in your session context. Two non-obvious cases:

- **Cross-batch filing.** If you're in an agent that isn't the writer
  (status != active) and you discover work that belongs to another
  batch, call \`newde__get_batch_context\` with no args to list peer
  batches, then \`create_work_item\` with that peer's \`batchId\`.
- **Backlog (stream-less).** The UI surfaces a backlog for work that
  isn't yet committed to any stream. \`create_work_item\` always
  requires a batchId; backlog filing is a UI action, not MCP.

Don't move an item between batches yourself — the UI handles that via
drag-and-drop. If you notice an item is in the wrong batch, add a
note (\`add_work_note\`) explaining why and let the user move it.

## Kind rubric

- **task** — the default.
- **epic** — multi-step work that decomposes into child tasks (see
  above; file children in the same turn).
- **subtask** — a step inside a large task.
- **bug** — a defect, phrased as an observation.
- **note** — something the user should see in the history but never
  enters the queue.

If you're unsure between task and epic, pick **task**. Promote later
by creating the epic and re-parenting.

## Priority

Default is \`medium\`. Reserve \`urgent\` for genuine blockers (broken
build, failing CI on main, data-loss bug). Use \`high\` sparingly for
things the user explicitly called out as time-sensitive. \`low\` is
fine for nice-to-haves the user won't miss if they slip.

Don't inflate priority to bump your own item up the queue — the user
orders the queue; priority is signal, not control.

## Acceptance criteria

Write them **observable**: a human reading just the criterion should
be able to tell whether it's met without re-reading the description.

- Good: "grep finds no remaining references to the old flag name",
  "the Archived section header renders only when N > 0", "\`bun
  test\` passes locally".
- Bad: "refactor the loader", "clean up the code", "make it work".

One criterion per line, plain text. Don't embed XML/Markdown
structure — the tool will accept it, but the UI renders it as-is and
the structure doesn't buy anything.

For epics, acceptance criteria describe the initiative's done bar,
not "all children are done" — that's implicit.

## What not to file

- A single message reply ("explain X"): no task — just answer.
- Your own scratch todos for the current turn: use the built-in
  TaskCreate tool instead. newde work items are the *user-visible,
  durable* record.
- Things the user explicitly asked you to *skip*: if they say "don't
  file this," don't file it.

## Referring to items in user-visible output

The user sees work items in the UI by **title**, not id. When you
mention an item in chat or summaries, quote the title ("Slim MCP
responses"), not the id. Ids are for MCP calls only — surfacing them
to the user is noise.
`;
}

// SKILL 2 — lifecycle. Triggers on status transitions + notes.
export function buildTaskLifecycleSkill(): string {
  return `---
name: ${TASK_LIFECYCLE_SKILL_NAME}
description: Work-item status transitions (ready → in_progress → human_check), epic rollup, when to use blocked / canceled / archived, and note discipline. Triggers on calls to mcp__newde__update_work_item with a status change, mcp__newde__add_work_note, finishing work on a task, or thinking about how to close out an item.
---

# Work-item lifecycle

This skill covers status transitions and notes for already-filed
items. For creating new items see \`newde-task-filing\`; for
orchestrator-vs-subagent execution modes see \`newde-task-dispatch\`.

## Status conventions

- New items default to \`ready\`. Don't pre-mark \`in_progress\` on
  creation unless you're literally about to start it this turn.
- **Set \`in_progress\` BEFORE any other work on the item.** Your
  first tool call when picking up an item must be
  \`update_work_item\` with \`status: "in_progress"\` — *before* any
  Read, Grep, Bash, or Edit. If you do the work first and only flip
  the status right before \`human_check\`, the item visibly skips
  the In Progress section in the UI (ready → human_check in a
  blink) and the user loses visibility into what you're doing. The
  Stop hook also uses the sole-in-progress item to attribute file
  changes, so leaving an old item in_progress while you work on a
  new one misattributes changes.
- **Never self-mark \`done\`.** Push to \`human_check\` when you
  believe the acceptance criteria are met. The user marks done
  after reviewing. If you set done, you will be corrected.
- **\`in_progress\` means you are actively working on it right now —
  not "I made progress on it."** The moment the acceptance criteria
  are met, move the item to \`human_check\` in the same turn, before
  ending your response. Don't narrate completion ("all done, tests
  pass") while leaving the item parked in IN PROGRESS — the Work
  panel's IN PROGRESS section is the user's live signal of what's
  in motion, and finished work hiding there defeats its purpose.
- **\`blocked\`** is for "can't proceed until X" (missing context,
  waiting on another item, waiting on the user). Add a note that
  explains what the blocker is. If you hit an error you cannot
  resolve, mark the item \`blocked\` and call \`add_work_note\` with
  the error details — do NOT leave the item \`in_progress\` while
  stuck.
- **\`canceled\`** is for work the user has decided against. Prefer
  \`delete_work_item\` for things *you* decided against before
  anyone else saw them.
- **\`archived\`** hides the item from the default Work view. Use
  it to clear out clutter (old bugs, resolved notes) without losing
  the history. Don't archive items the user hasn't seen yet.

Rewrite title / description / acceptance criteria whenever your
understanding shifts. Stale state in the queue costs more than a
tool call to fix it.

## Epic rollup

While children are in flight, the epic stays \`in_progress\`. When
every child has reached \`human_check\` / \`done\` / \`canceled\` /
\`archived\`, move the epic itself to \`human_check\` in the same
turn — don't leave a settled epic parked in IN PROGRESS waiting for
the user to notice.

## Batch transitions

When you need to flip several items at once (e.g., rolling an epic
and all its children to \`human_check\` at the end of a phase), use
\`mcp__newde__transition_work_items\` with an array of
\`{batchId, itemId, status}\`. It emits the same side effects (events,
effort open/close) as individual update_work_item calls but avoids N
round trips.

## Notes and history

Use \`add_work_note\` at meaningful milestones — decisions you made,
surprises you hit, snags that changed the approach, a terse summary
before marking \`human_check\`. Don't log every step-by-step; the
user reads the note stream the way you'd skim a pull-request
description.

Notes also suppress the Stop-hook "did you forget to settle this?"
nudge — if you're intentionally leaving an item \`in_progress\`
across turns (e.g., long-running work), drop a note this turn
summarizing what's still needed.

## Claude Code TaskCreate vs newde work items

This project uses BOTH, for different grains — never mirror between
them.

- **Claude Code \`TaskCreate\`/\`TaskUpdate\`** is the built-in
  within-turn step tracker. Use it for ephemeral micro-planning:
  "read pipeline.ts → write failing test → run test → fix → update
  doc." It is invisible to the user, discarded when the turn ends,
  and costs nothing durable.
- **newde work items** (\`mcp__newde__*_work_item\`) are the
  user-visible, durable, cross-session record. "Slim the MCP
  response." "Fix the wait-point bug." Persisted to SQLite, survive
  restart, the user approves and marks done. This is the canonical
  record of work done on the user's behalf — local history
  attributes file changes back to these.

If you create a newde work item for "fix X" and then also create a
TaskCreate entry "fix X" you're duplicating. Pick one surface per
piece of work: TaskCreate for the within-turn recipe, newde for the
deliverable.
`;
}

// SKILL 3 — dispatch. Triggers when the orchestrator is about to decide
// between inline execution and subagent dispatch.
export function buildTaskDispatchSkill(): string {
  return `---
name: ${TASK_DISPATCH_SKILL_NAME}
description: Orchestrator dispatch protocol for executing filed newde work items — when to do the work inline vs when to launch a general-purpose subagent, and how to compose a terse subagent brief. Triggers on calls to newde__read_work_options, deciding how to execute a work item, or composing a subagent brief that mentions work item ids.
---

# Executing filed work items (orchestrator side)

**This skill is for EXECUTING filed items only.** Creating, updating,
and status-flipping work items is always inline — see the
\`newde-task-filing\` skill's "Filing is ALWAYS inline" rule.

You are the **orchestrator**. For each ready work item, pick one of
two execution modes:

- **Inline small fix.** Mechanical, low-risk change (≤ ~20 lines
  across ≤ 2 files, e.g. fix a test fixture, remove an unused
  import, rename a label). Do the Read/Edit/Bash yourself under the
  work item — mark \`in_progress\`, edit, run tests, mark
  \`human_check\`. Snapshots still attribute the changes to the
  task.
- **Subagent dispatch.** Larger / multi-file / risky work. Launch a
  \`general-purpose\` subagent.

**Step 0 — file before you execute.** If the user just handed you a
new task that isn't in the queue yet, create the work items *first*
(see the \`newde-task-filing\` skill) before calling
\`read_work_options\` or starting any inline edit. The user monitors
progress through the work-item UI; if nothing is filed, they have no
visibility.

## For a subagent dispatch

1. Call \`newde__read_work_options\` (batchId=your batch) to get the
   next dispatch unit:
   - \`{ mode: "epic", epic, children }\` — dispatch the whole epic.
   - \`{ mode: "standalone", items }\` — pick one item or a
     link-related cluster.
   - \`{ mode: "empty" }\` — nothing left; allow stop.

   The response is **slim by default**
   (id/title/kind/priority/parent_id/status/sort_index). Call
   \`newde__get_work_item\` per id for descriptions + acceptance
   criteria when composing the brief, so you only pull detail for
   items you'll actually dispatch. Use \`full=true\` on
   \`read_work_options\` only if you want everything in a single
   call.

2. Assemble a **terse** brief. The subagent will call
   \`get_work_item\` on each id to fetch description + acceptance
   criteria itself, and it can \`Read\`/\`Grep\` \`.context/*.md\` on
   its own — do not hand-copy any of that into the brief.

   A brief contains only:
   - The work item ids (and titles, for readability).
   - Ordering, if multiple items.
   - Task-specific context that is **not already in the item
     description** (cross-item coordination, a pointer to a doc the
     description doesn't mention, a decision the user made in chat).
   - The batchId, so \`get_work_item\` calls have the right scope.

   **Do NOT repeat** (a) the item description or acceptance criteria
   verbatim — the subagent pulls them via \`get_work_item\`; (b) the
   status-lifecycle / in_progress-first / attribution / note-discipline
   / blocked-on-error / epic-rollup rules — the
   \`newde-subagent-work-protocol\` skill covers those and auto-loads
   when the subagent sees a \`wi-…\` id; (c) project-standard file
   locations, conventions, or \`.context/\` doc contents — subagents
   discover those with Read/Grep.

   Example brief (≈60 tokens):

   > Work items in batchId=b-abc123, in order:
   >   1. wi-111 — Slim read_work_options response
   >   2. wi-222 — Update task-management skill language
   >
   > Fetch details with get_work_item. Protocol lives in the
   > newde-subagent-work-protocol skill.

   If the user gave specific guidance not captured in any work-item
   description (e.g. "prefer debounce over dedupe here"), append
   one short line with that.

3. Launch one \`general-purpose\` subagent with that brief.

4. When the subagent returns, call \`newde__add_work_note\` on each
   item with its summary.

5. Loop from step 1.

## For an inline small fix

1. Call \`update_work_item\` with \`status: "in_progress"\` FIRST —
   before any Read, Grep, Bash, or Edit.
2. Do the edit. Run any relevant tests.
3. Call \`update_work_item\` with \`status: "human_check"\` once
   acceptance criteria are met.

Same invariants either mode: never two items \`in_progress\` at
once (file-change attribution uses the sole in-progress item), mark
\`blocked\` with a note on unresolved errors, never self-mark
\`done\`.
`;
}
