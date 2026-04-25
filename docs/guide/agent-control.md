# Agent control

Oxplow steers the agent through three mechanisms, in order of
how often they fire: the **Stop hook**, the **MCP control
plane**, and the **work-queue orchestrator**.

You don't normally interact with these directly — but understanding
them is what makes the rest of oxplow's behavior obvious instead
of magical.

## The Stop hook

Claude Code calls a hook when a turn ends. Oxplow registers a
local HTTP endpoint as that hook. Every time the agent stops,
oxplow runs through:

1. Capture the file snapshots (before/after) for files the
   agent touched this turn.
2. Update the open-turn row in the Plan pane (it disappears
   from the live list).
3. Run the **task audit**: nudge the writer thread to verify
   each `in_progress` work item is still really in progress.
4. Optionally fire a commit (auto-commit mode, or commit point
   reached).
5. Optionally fire the next item from the queue (if the agent
   left a directive saying "do the next one").

The hook is what makes oxplow's primitives durable. Without it,
work items would drift out of sync with reality.

## MCP control plane

Oxplow exposes its primitives as MCP tools the agent can call
directly:

- **Work items.** `create_work_item`, `update_work_item`,
  `complete_task`, `add_work_note`, `file_epic_with_children`.
- **Threads and notes.** `get_thread_context`,
  `list_thread_work`, `add_work_note`, wiki-note tools.
- **Snapshots.** `list_commit_points`, `tasks_since_last_commit`.
- **LSP.** `lsp_definition`, `lsp_hover`, `lsp_references`,
  `lsp_diagnostics`.
- **Dispatch.** `dispatch_work_item` for orchestrator-style
  flows where one agent hands a brief to a subagent.

See [MCP tools reference](../reference/mcp-tools.md) for the
full list and signatures.

## Write guard

Each thread has a role:

- **Writer.** Edits files. One per stream.
- **Read-only.** Cannot edit files; can read, search, query.

The role is enforced in the hook. When a non-writer thread tries
to edit, the hook returns an error to the agent before the edit
runs. This is non-bypassable from the agent side — the guard is
out-of-process.

## Work-queue orchestrator

The orchestrator decides what the agent works on next. Today it
is largely "consume the queue in `sort_index` order." When a
turn ends and the queue isn't empty, the orchestrator picks the
next ready item, brings it `in_progress`, and prompts the agent
to start.

This is what lets you queue several items and walk away — the
agent doesn't need a fresh prompt for each.

## Per-thread MCP server

Each thread gets its own MCP server instance, scoped to that
thread's `threadId`. Tool calls implicitly target the right
stream and writer status. From the agent's side, it's just
"`mcp__oxplow__create_work_item` works"; the routing is handled
by oxplow.

## What you can change

Most of this is infrastructure — set up once, then invisible.
You can:

- Toggle auto-commit mode per stream.
- Pause / resume the orchestrator (so the agent stops auto-
  progressing the queue).
- Set the writer thread per stream.
- Enable / disable the write guard for a thread (rarely useful,
  but supported).

The internal details (hook endpoints, MCP transport, snapshot
storage) are not user-configurable on purpose — changing them
breaks the durability guarantees the rest of the product depends
on.
