# MCP tools

Oxplow exposes its primitives over the Model Context Protocol so
the agent can drive them directly. All tools are namespaced
under `mcp__oxplow__`. Each call carries a `threadId` so the
server can resolve which stream and writer status the action
should run against.

This page is the user-facing summary. It is not a generated
reference — for the authoritative schema, look at the tool
definitions surfaced by the MCP server itself.

## Work items

| Tool | Purpose |
|---|---|
| `create_work_item` | File a single task / bug / note / subtask in the current thread. |
| `file_epic_with_children` | Atomically file an epic plus its child items in one call. |
| `update_work_item` | Change title, description, status, priority, parent. Used to reopen `human_check` items. |
| `complete_task` | Final note + transition to `human_check` (or `blocked`) in one call. Optionally records `touchedFiles` for Local History attribution. |
| `add_work_note` | Append a note to a work item without changing status. |
| `delete_work_item` | Remove a row entirely (rare; usually you `archive` instead). |
| `list_thread_work` | List items in the current thread. |
| `get_work_item` | Fetch a single item by id. |
| `read_work_options` | Get valid statuses, kinds, priorities for the current item. |
| `transition_work_items` | Bulk status change. |
| `reorder_work_items` | Set `sort_index` for one or more items. |
| `link_work_items` | Add cross-links between items. |

## Commit and wait points

| Tool | Purpose |
|---|---|
| `list_commit_points` | Enumerate the commit points in the current stream's queue. |
| `tasks_since_last_commit` | Items closed since the last commit point — drives auto-commit messages. |
| `commit` | Trigger a commit at the current position. |

## Threads, dispatch, and queries

| Tool | Purpose |
|---|---|
| `get_thread_context` | Pull thread + stream context (current items, active turn, etc.). |
| `fork_thread` | Spawn a new read-only thread from the current writer. |
| `dispatch_work_item` | Build a ready brief for a subagent to execute against a specific item. |
| `delegate_query` | Hand a question to another thread. |
| `record_query_finding` | Persist the answer back to the originating thread. |
| `list_ready_work` | Items eligible for the orchestrator to pick up next. |

## Notes (wiki-style)

| Tool | Purpose |
|---|---|
| `list_notes` | List all notes in the project. |
| `get_note_metadata` | Title, links in/out, last update. |
| `search_notes` | Full-text search. |
| `resync_note` | Re-read storage for a single note. |
| `delete_note` | Remove a note. |

## LSP bridge

| Tool | Purpose |
|---|---|
| `lsp_definition` | Go-to-definition for a symbol at a file:line:col. |
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

There's no MCP surface for editing files (the agent uses Claude
Code's built-in edit tools, not oxplow's), running shells (use
the terminal pane), or changing oxplow settings (those are out
of scope for the agent to drive).

The split is deliberate: oxplow's tools are about *intent and
attribution* — work items, snapshots, threads, notes. Mechanics
that already have first-class tools elsewhere are left there.
