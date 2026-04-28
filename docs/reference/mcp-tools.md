# MCP tools

Oxplow exposes its primitives over the Model Context Protocol
so the agent can drive them directly. All tools are
namespaced `mcp__oxplow__`. Each call carries a `threadId` so
the server can resolve which stream and writer status the
action runs against.

This page is the user-facing summary. For the authoritative
schema, look at the tool definitions surfaced by the running
MCP server.

## Work items

| Tool | Purpose |
|---|---|
| `create_work_item` | File a single task / bug / note / subtask. Default `kind` is `task`. Pass `status: "in_progress"` to commit to ship now (and satisfy the filing guard). |
| `file_epic_with_children` | Atomically file an epic plus its child items in one call. Use when the work has ≥3 sub-steps a reviewer would inspect independently. |
| `update_work_item` | Change title, description, status, priority, parent, touchedFiles. Used to reopen a `done` item by flipping back to `in_progress`. |
| `complete_task` | Final note + transition to `done` (or `blocked`) in one call. `touchedFiles` records files for Local History attribution. Rejects items already in a terminal status. |
| `transition_work_items` | Bulk status change across multiple items. |
| `add_work_note` | Append a note to a work item without changing status. |
| `delete_work_item` | Remove a row entirely (rare; usually `archive` instead). |
| `reorder_work_items` | Set `sort_index` for one or more items. |
| `link_work_items` | Add cross-links (relates / blocks / etc.) between items. |
| `list_thread_work` | List items in the current thread. |
| `get_work_item` | Fetch a single item by id. |
| `list_ready_work` | Items eligible for inspection / dispatch. |
| `read_work_options` | Returns the next dispatch unit — either an epic + ready descendants, or the standalone ready cluster. |
| `list_recent_file_changes` | Files recently modified in this stream. |

## Threads, dispatch, and queries

| Tool | Purpose |
|---|---|
| `get_thread_context` | Pull thread + stream context (active items, writer, peer threads). |
| `list_batch_work` | List work for the current batch. |
| `get_batch_context` | Stream/thread/batch summary plus `otherActiveBatches` across peer streams. |
| `fork_thread` | Spawn a new queued thread on the same stream. Optionally seeds a "Context from fork" note and moves `ready` / `blocked` items across in one transaction (carries forward the last 3 notes per moved item). |
| `dispatch_work_item` | Compose a subagent brief server-side (preamble + item fields + children + last notes + extra context). Atomically transitions `ready`/`blocked` items to `in_progress` (default). |
| `delegate_query` | Hand a question to another thread. |
| `record_query_finding` | Persist the answer back to the originating thread. |
| `await_user` | Tell the runtime the agent is asking the user a question — Stop allows clean stop and suppresses every directive until the next user prompt. |

## Followups

Transient, in-memory sub-asks (no DB row, lost on restart).
Surface as italic muted "↳ follow-up: …" lines at the top of
the Work panel's To Do section.

| Tool | Purpose |
|---|---|
| `add_followup` | Stash a deferred sub-ask the orchestrator wants to come back to in this same turn. |
| `remove_followup` | Pop a follow-up when handled. |
| `list_followups` | List the current thread's follow-ups. |

Not exposed to subagents — the dispatch brief deliberately
omits any mention of follow-ups.

## Wiki notes

| Tool | Purpose |
|---|---|
| `list_notes` | List all notes in the project. |
| `get_note_metadata` | Title, body length, refs in/out, last update. |
| `search_notes` | Title search. |
| `search_note_bodies` | Full-body search. |
| `find_notes_for_file` | Notes that reference a given repo path (backlinks for files). |
| `resync_note` | Force an immediate re-baseline of a single note's index. |
| `delete_note` | Remove a note. |

There is intentionally **no** create-note or update-note tool.
The agent writes bodies directly to `.oxplow/notes/<slug>.md`
with its `Write` / `Edit` tools — the notes watcher syncs
metadata + body on every file event.

## Subsystem docs

| Tool | Purpose |
|---|---|
| `get_subsystem_doc` | Cheap read of `.context/<name>.md` — returns `{ name, path, content, exists }` and never hard-errors on missing. |

## LSP bridge

| Tool | Purpose |
|---|---|
| `lsp_definition` | Go-to-definition for a symbol at file:line:col. |
| `lsp_hover` | Hover info. |
| `lsp_references` | Find references. |
| `lsp_diagnostics` | Diagnostics for a file. |

The LSP tools talk to the same servers the editor uses, so
answers stay consistent between the agent and you.

## Health

| Tool | Purpose |
|---|---|
| `ping` | Confirm the MCP server is reachable. |

## What's not exposed

- **No file-edit tools.** The agent uses Claude Code's built-in
  `Edit` / `Write` / `MultiEdit` (gated by oxplow's filing-
  enforcement and write-guard hooks).
- **No commit tool.** Commits are user-driven (CLI / Bash /
  Uncommitted Changes page).
- **No settings tools.** Configuration is the human's job.
- **No queue auto-progression directive.** When the agent
  finishes its obligations, it stops — you resume the queue by
  prompting or running `/work-next`.

The split is deliberate: oxplow's tools are about *intent and
attribution* — work items, snapshots, threads, notes,
backlinks. Mechanics that already have first-class tools
elsewhere are left there.
