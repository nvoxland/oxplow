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

Gotcha: Claude Code silently drops HTTP hooks for `SessionStart` ("HTTP hooks
are not supported for SessionStart" in `claude --debug-file`). Only command-
type hooks are supported there. Everywhere else we rely on hook events to
learn the session id, so we adopt whichever id shows up on the *next* hook
that does fire (`UserPromptSubmit`, `PreToolUse`, `Stop`, `SessionEnd`, …) —
see `decideResumeUpdate` in `src/session/resume-tracker.ts`.

`newde__get_batch_context` returns, besides the caller's stream/batch
ids + summary, an `otherActiveBatches: Array<{ streamId, streamTitle,
batchId, batchTitle, activeBatchId }>` with one entry per peer stream —
handy when the agent suspects the "current stream" has drifted from
where it actually writes (the same phenomenon that motivated the
streamId-derivation in other MCP tools).

Each hook POSTs to the runtime's MCP server with bearer-token auth via the
env-var-interpolated `NEWDE_HOOK_TOKEN` header, plus `X-Newde-Stream`,
`X-Newde-Batch`, `X-Newde-Pane`. The MCP server's `onHook` callback dispatches
to `runtime.handleHookEnvelope`, which:

1. Stores the event in `HookEventStore` (a ring buffer, also fed to the UI's
   Hook Events tool window via the `hook.recorded` EventBus event).
2. If the normalized payload carries a session id that differs from
   `batch.resume_session_id`, persists the new id so a later newde restart
   relaunches claude with `--resume <id>`.
3. Calls `applyTurnTracking` to open/close `agent_turn` rows and flush
   turn-start / turn-end snapshots (see "Snapshot tracking" below). On `Stop`
   we also read the session transcript (path rides on the hook payload) and
   sum assistant-message `usage` since the turn's `started_at`, persisting
   input/output/cache-read token counts on `agent_turn`. See
   `summarizeTurnUsage` in `src/session/transcript-usage.ts`.
4. For `PreToolUse`: returns a deny response if `buildWriteGuardResponse`
   blocks the tool (see Write guard below).
5. For `UserPromptSubmit`: returns `additionalContext` made up of a
   live `<session-context>` block (stream + batch + writer, rebuilt
   from the stores — see `buildSessionContextBlock` in `runtime.ts`)
   followed by the editor-focus summary from
   `src/session/editor-focus.ts`. The session-context block refreshes
   on every turn so the agent notices when the user promoted a
   different batch to writer mid-session; the frozen ids in the
   launch-time system prompt no longer win.
6. For `Stop`: runs `computeStopDirective` (below).

## Stop-hook pipeline

The decision logic lives in `decideStopDirective` (a pure function in
`src/electron/stop-hook-pipeline.ts`). The runtime's
`computeStopDirective(batchId)` builds a `BatchSnapshot` from the live
stores, calls the pure function, then applies any returned side effects
(currently only `trigger-wait-point`). Keeping the decision separate
from the side effects lets every branch be unit-tested with a fixture.

The pipeline runs in priority order:

1. **Pending commit point (writer batch only).** Block with a directive
   built by `buildCommitPointStopReason` telling the agent to inspect the
   diff, draft a commit message in Conventional-Commits style, and call
   `mcp__newde__propose_commit({ commit_point_id, message })`. Don't run
   `git` directly — the runtime will commit on receipt of an approved
   message. **Gated on `batch.status === "active"`**: only the writer
   batch ever commits. Non-active batches fall through so the commit
   point stays pending until that batch is promoted to writer.
2. **Pending wait point.** Flip it to `triggered` and **allow stop**. The
   UI shows "agent stopped here"; the user resumes by prompting the agent
   directly (`triggered` points are skipped on subsequent Stop hooks, so
   one prompt is enough — no "continue" button). A marker (commit or wait)
   is "active" once every preceding work item is terminal, where terminal
   includes `human_check` — agents never self-mark `done`, so demanding
   `done` would let the agent march past the line indefinitely while the
   user catches up on verification.
3. **Approval-mode commit awaiting user.** Allow stop while the user
   approves/rejects in the UI. After approve, the runtime commits and
   marks the point `done`. (The agent is now idle; the user re-engages
   to drain the rest of the queue.)
4. **Writer batch with a ready work item.** Block with a directive built
   by `buildNextWorkItemStopReason` naming the item's id + kind + title +
   batch_id + stream_id, and telling the agent to mark it `in_progress`
   before working. Items the agent itself filed during the *current* turn
   are skipped (ready-list filtered by `created_by="agent"` AND
   `created_at >= currentTurnStartedAt`). Those are triage-inbox entries
   from flows like `/autoimprove`; forcing the agent to pick them up would
   invert the user's "leave in waiting" intent. "Queue" here means
   waiting + ready items only — `human_check` belongs to the user's
   review queue and is terminal from the pipeline's perspective.
5. **Otherwise.** Allow stop.

**`propose_commit` vs. direct `git commit`.** Commit points + the
"+ Commit when done" Work-panel button are for **automated workflows**
where the user wants the agent to pause and commit at a specific point
in a multi-step batch. For one-off "finish this and commit" work,
just tell the agent to run `git commit` directly — no commit point
needed, no approval UI. If you prompt the agent to `propose_commit`
and there's no active commit point, the agent will skip it and the
diff stays uncommitted until someone notices. The `+ Commit when done`
bar renders even when the To-do section is empty (so it's
discoverable), but the canonical path for ad-hoc commits is still
plain `git commit` via the agent.

Auto-mode commit points pass straight through: the agent proposes →
`commitPointStore.propose` jumps the status to `approved` →
`runtime.executeApprovedCommit` runs `gitCommitAll` and marks `done` →
the agent's `propose_commit` MCP call returns → agent tries to Stop again
→ pipeline picks the next thing. No human in the loop.

## BatchQueueOrchestrator

Cross-store queue logic (commit points + wait points + the
`reorderBatchQueue` that rewrites all three sort_index spaces) lives
in `src/electron/batch-queue-orchestrator.ts`. The runtime instantiates
it as `this.batchQueue` and delegates IPC-exposed methods through.
`executeApprovedCommit` (which runs the actual `git commit` after a
point flips to `approved`) is also there, called both eagerly via
the runtime's `commitPointStore.subscribe` handler and at startup
via `drainPendingExecutions()` to cover crashes mid-commit.

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

## Snapshot tracking

The runtime keeps a content-addressed history of worktree files so the
UI (and future tooling) can render turn-level diffs without relying on
git. Mechanics:

- A per-stream in-memory **dirty set** accumulates relative paths. It
  is populated by the workspace fs-watcher (always, regardless of
  batch state) and by the PostToolUse hook (`persistFileChange`).
- On `UserPromptSubmit`, after the new `agent_turn` row is opened,
  `flushSnapshotForStream(streamId, "turn-start", turnId, batchId)` is
  called. If the dirty set is non-empty it writes blobs + a manifest
  under `.newde/snapshots/`, inserts a `file_snapshot` row, updates
  `streams.current_snapshot_id`, and backfills `snapshot_id` on the
  `batch_file_change` rows whose paths match. The dirty set is then
  cleared.
- On `Stop`, after the turn is closed, `flushSnapshotForStream` is
  called again with `kind: "turn-end"`.
- On project open, `seedSnapshotTracking` runs once per stream. If the
  stream has no `current_snapshot_id`, every file in the worktree is
  marked dirty (the baseline case). Otherwise `reconcileWorktree`
  walks the tree and compares `(mtime_ms, size)` against the resolved
  entry map from the parent chain; drifted paths plus tombstone
  mismatches (file deleted / created while the app was closed) go
  into the dirty set so the next turn-start captures them.
- `runtime.getTurnFileDiff(turnId, path)` returns `{ before, after }`
  strings by resolving the turn-start snapshot's parent (the "before")
  and the turn-end snapshot (the "after") through `SnapshotStore.diffPath`.

See [data-model.md](./data-model.md) for the `file_snapshot` schema
and manifest layout, and [ipc-and-stores.md](./ipc-and-stores.md) for
the `file-snapshot.created` EventBus event and the `getTurnFileDiff`
IPC method.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` and the
  approved-commit-point execution loop.
