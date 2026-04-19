# Agent execution model

What this doc covers: how a Claude (or copilot) process is launched in a
batch, how the runtime steers it through the work queue without ever
sending it raw prompts, and the rules that keep non-writer batches from
clobbering the writer's worktree. If you're touching MCP tools or the
queue itself, also read [data-model.md](./data-model.md).

## Key invariant

**The runtime never sends prompts to the agent.** The only ways to steer
the agent are:

1. The system prompt set at launch (`--append-system-prompt`).
2. Hook responses returned to Claude over HTTP (especially the Stop hook's
   `{ decision: "block", reason: "…" }` form, which Claude treats as a
   fresh instruction to keep going).
3. MCP tool responses (when the agent calls a `newde__*` tool).

Auto-progression through the queue is built entirely on (2). The agent
thinks it's about to stop; the harness says "actually, do this next."

## Launching the agent

`buildAgentCommandForSession` in `src/agent/agent-command.ts` constructs a
shell command that:

- `cd`s into the stream's worktree
- runs `claude --plugin-dir <abs> --allowedTools mcp__newde__* --append-system-prompt <text> --mcp-config <json> [--resume <sid>]`
- exports `NEWDE_STREAM_ID`, `NEWDE_BATCH_ID`, `NEWDE_HOOK_TOKEN` so the
  plugin's HTTP hooks can identify themselves to the runtime

`copilot` is supported as an alternative agent kind but skips the plugin
plumbing — it just `cd && exec copilot`.

The command is launched in a tmux pane via `ensureAgentPane`
(`src/terminal/fleet.ts`). Switching streams or batches doesn't kill
existing agent sessions; tmux keeps them alive in the background.

## Plugin hook bridge

`createElectronPlugin` (`src/session/claude-plugin.ts`) writes a per-project
Claude Code plugin into `<projectDir>/.newde/runtime/claude-plugin/`. The
plugin's `hooks.json` registers HTTP hooks for `PreToolUse`, `PostToolUse`,
`UserPromptSubmit`, `SessionStart`, `SessionEnd`, `Stop`, and `Notification`.

Each hook POSTs to the runtime's MCP server with bearer-token auth via the
env-var-interpolated `NEWDE_HOOK_TOKEN` header, plus `X-Newde-Stream`,
`X-Newde-Batch`, `X-Newde-Pane`. The MCP server's `onHook` callback dispatches
to `runtime.handleHookEnvelope`, which:

1. Stores the event in `HookEventStore` (a ring buffer, also fed to the UI's
   Hook Events tool window via the `hook.recorded` EventBus event).
2. Updates the resume-session tracker so reconnects find the latest sid.
3. Calls `applyTurnTracking` to open/close `agent_turn` rows.
4. For `PreToolUse`: returns a deny response if `buildWriteGuardResponse`
   blocks the tool (see Write guard below).
5. For `UserPromptSubmit`: returns `additionalContext` from the editor focus
   tracker (`src/session/editor-focus.ts`) so the agent sees what the user
   has open/selected.
6. For `Stop`: runs `computeStopDirective` (below).

## Stop-hook pipeline

The decision logic lives in `decideStopDirective` (a pure function in
`src/electron/stop-hook-pipeline.ts`). The runtime's
`computeStopDirective(batchId)` builds a `BatchSnapshot` from the live
stores, calls the pure function, then applies any returned side effects
(currently only `trigger-wait-point`). Keeping the decision separate
from the side effects lets every branch be unit-tested with a fixture.

The pipeline runs in priority order:

1. **Pending commit point.** Block with a directive built by
   `buildCommitPointStopReason` telling the agent to inspect the diff,
   draft a commit message in Conventional-Commits style, and call
   `mcp__newde__propose_commit({ commit_point_id, message })`. Don't run
   `git` directly — the runtime will commit on receipt of an approved
   message.
2. **Pending wait point.** Flip it to `triggered` and **allow stop**. The
   UI shows "agent stopped here"; the user resumes by prompting the agent
   directly (`triggered` points are skipped on subsequent Stop hooks, so
   one prompt is enough — no "continue" button).
3. **Approval-mode commit awaiting user.** Allow stop while the user
   approves/rejects in the UI. After approve, the runtime commits and
   marks the point `done`. (The agent is now idle; the user re-engages
   to drain the rest of the queue.)
4. **Writer batch with a ready work item.** Block with a directive built
   by `buildNextWorkItemStopReason` telling the agent the next item's id,
   kind, and title, and to mark it `in_progress` before working.
5. **Otherwise.** Allow stop.

Auto-mode commit points pass straight through: the agent proposes →
`commitPointStore.propose` jumps the status to `approved` →
`runtime.executeApprovedCommit` runs `gitCommitAll` and marks `done` →
the agent's `propose_commit` MCP call returns → agent tries to Stop again
→ pipeline picks the next thing. No human in the loop.

## MCP tools

`buildWorkItemMcpTools` (`src/mcp/mcp-tools.ts`) registers the agent's
`newde__*` tool surface:

- `get_batch_context`, `record_batch_summary`, `list_batch_work`,
  `list_ready_work`, `create_work_item`, `update_work_item`,
  `get_work_item`, `delete_work_item`, `reorder_work_items`,
  `link_work_items`, `add_work_note`, `list_recent_file_changes`
- `propose_commit(commit_point_id, message)`, `list_commit_points(batchId)`

`buildLspMcpTools` (`src/mcp/lsp-mcp-tools.ts`) adds language-server
queries (definition, references, hover) the agent can use without
shelling out.

## Write guard

Non-writer batches share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** `buildWriteGuardResponse`
  (`src/electron/write-guard.ts`) returns a `PreToolUse` deny for `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` from any non-`active` batch.
- **Prompt enforcement.** `NON_WRITER_PROMPT_BLOCK` (same file) is
  appended to the system prompt for non-writer batches, telling the agent
  to avoid Bash mutations too (the hook can't reliably classify shell
  commands, so the prompt is the only line of defence there).
- MCP tools (`mcp__newde__*`) are always allowed: they write to the state
  DB, not the worktree.

## Custom prompt addendum

`config.agentPromptAppend` (loaded from `newde.yaml` via
`loadProjectConfig` in `src/config/config.ts`) is concatenated into every
agent's system prompt by `buildBatchAgentPrompt` (in `runtime.ts`). The
Settings modal (`src/ui/components/SettingsModal.tsx`) reads/writes this
via `runtime.setAgentPromptAppend` which calls `writeProjectConfig` to
persist back to YAML.

A new value applies to **agent sessions started after Save** — existing
sessions keep the prompt they launched with.

## Agent status

`deriveBatchAgentStatus` (`src/session/agent-status.ts`) reduces a stream
of hook events into one of `idle | working | waiting | done`. The runtime
recomputes on every hook arrival and emits `agent-status.changed`. The UI
shows it as a colored dot on each batch tab.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` and the
  approved-commit-point execution loop.
