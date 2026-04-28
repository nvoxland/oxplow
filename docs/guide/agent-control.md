# Agent control

Oxplow steers the agent through three mechanisms: the **Stop
hook**, the **MCP control plane**, and the **filing-enforcement
hook**. You don't normally touch them directly — but
understanding them is what makes the rest of oxplow's behavior
obvious instead of magical.

## Key invariant

**The runtime never sends prompts to the agent.** The only ways
the runtime steers the agent are:

1. The system prompt set at launch.
2. Hook responses returned over HTTP — especially the Stop
   hook's "block" form, which Claude treats as a fresh
   instruction to keep going.
3. MCP tool responses (when the agent calls one).

There is no auto-typing into the terminal. Auto-progression
through queued work happens because Stop directives keep the
agent from quitting until its obligations are settled.

## Stop hook

When the agent finishes a turn, Claude Code fires the Stop hook.
Oxplow's response decides whether the agent gets to stop or has
to keep working. The pipeline runs in priority order:

1. **Q&A short-circuit.** If the turn had no qualifying tool
   activity (no edits, no work-item filing, no dispatch), the
   agent answered or asked a question and is allowed to stop.
   Every directive is suppressed.
2. **Awaiting-user gate.** If the agent called
   `await_user({ question })`, the runtime allows the stop and
   suppresses every directive until the user replies.
3. **In-progress audit.** If the writer thread has any
   `in_progress` work items, the runtime blocks with an audit
   directive: reconcile each item — still active → leave alone;
   acceptance criteria met → `complete_task`; stuck →
   `blocked`; obsolete → `canceled`. A signature dedup prevents
   the same audit firing repeatedly when nothing changed.
4. **Filed-but-didn't-ship advisory.** Catches the misread
   where the agent logged a `ready` row instead of doing the
   work the user asked for.
5. **Otherwise.** Allow stop.

Cross-turn queue progression is **user-driven**. When the agent
finishes its obligations and Stops, it stops — you resume queue
work by sending a new prompt or running the bundled
`/work-next` slash command (which calls `read_work_options` and
dispatches the next ready cluster).

## Filing-enforcement hook (PreToolUse)

When the agent invokes `Edit` / `Write` / `MultiEdit` /
`NotebookEdit` on the writer thread without an `in_progress`
work item, the hook denies the edit before it lands. The agent
files an item at `in_progress` (or flips an existing `ready`
row) and re-issues the edit.

A `ready`-status filing alone does not satisfy the guard.
`ready` is "noticed for later" (backlog); only `in_progress` is
"committed to ship now."

`Bash` is exempt — shell commands routinely mutate the worktree
as a side effect (`git merge`, `git pull`, codegen, formatters)
without representing authored change worth filing.

## Write guard

Each thread has a role:

- **Writer.** Edits files. One per stream.
- **Read-only.** Cannot edit files; can read, search, query.

The role is enforced in the hook. When a non-writer thread
tries to edit, the hook returns an error before the edit runs.
This is non-bypassable from the agent side — the guard is
out-of-process. Promote a different thread to writer (or switch
to the writer thread) if you need to edit.

The wiki notes directory (`.oxplow/notes/`) is exempt — note
capture works on read-only threads too, because the wiki is
research output, not authored project change.

## MCP control plane

Oxplow exposes its primitives as MCP tools the agent can call
directly. The full list is in
[MCP tools](../reference/mcp-tools.md). Highlights:

- **Work items.** `create_work_item`, `update_work_item`,
  `complete_task`, `add_work_note`, `transition_work_items`,
  `file_epic_with_children`, `link_work_items`,
  `reorder_work_items`.
- **Dispatch.** `read_work_options`, `dispatch_work_item` for
  orchestrator-style flows where one agent hands a brief to a
  subagent.
- **Threads.** `get_thread_context`, `list_thread_work`,
  `fork_thread`, `await_user`.
- **Followups.** `add_followup`, `list_followups`,
  `remove_followup` for transient sub-asks.
- **Wiki notes.** `list_notes`, `search_notes`,
  `search_note_bodies`, `find_notes_for_file`,
  `get_note_metadata`, `resync_note`, `delete_note`. Bodies
  are written directly with the agent's `Write` / `Edit`
  tools — there is intentionally no create-note MCP call.
- **Subsystem docs.** `get_subsystem_doc({ name })` for cheap
  reads of `.context/<name>.md`.
- **LSP.** `lsp_definition`, `lsp_hover`, `lsp_references`,
  `lsp_diagnostics`.

All MCP tools take a `threadId` so the runtime can resolve the
right stream and writer status.

## Per-thread MCP server

Each thread gets its own MCP server endpoint, scoped to that
thread's `threadId`. Tool calls implicitly target the right
stream. From the agent's side, it's just
"`mcp__oxplow__create_work_item` works."

## What you can change

Most of this is infrastructure — set up once, then invisible.
You can:

- Switch the writer thread per stream (thread kebab → "Promote
  to writer").
- Promote a read-only thread when you want it to commit.
- Edit per-stream / per-thread custom prompts (Stream Settings
  / Thread Settings pages).
- Pause or resume a thread's agent process from the thread
  tab kebab.

The internal details (hook endpoints, MCP transport, snapshot
storage) are not user-configurable on purpose — changing them
would break the durability guarantees the rest of the product
depends on.

## What's deliberately not here

- **Auto-commit.** The runtime never runs `git commit`. Commits
  are user-driven — run them in the terminal yourself, click
  commit in the Uncommitted Changes page, or tell the agent
  "go run `git commit -m …`".
- **Wait points / commit points.** Removed in favor of
  user-driven commits and the in-progress-audit Stop directive.
- **Auto-progression through the queue.** When the agent
  Stops, it stops. The user resumes by prompting or running
  `/work-next`.
