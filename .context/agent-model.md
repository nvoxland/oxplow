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
   fresh instruction to keep going). The default no-op response is
   `200 {}` (not `202` empty) — Claude Code prints a "non-blocking
   status code" warning into the user's terminal on every empty 202,
   which fills the xterm with noise on Edit/Write-heavy turns.
3. MCP tool responses (when the agent calls a `newde__*` tool).

Auto-progression through the queue is built entirely on (2). The agent
thinks it's about to stop; the harness says "actually, do this next."

**Caveat — the first turn still needs a user prompt.** "Runtime never
prompts" is about auto-progression, not cold-start. When the agent is
sitting idle at its shell prompt (e.g. just after `newde` opens a
fresh project, or after a `Stop` that didn't block), creating a work
item does **not** kick it off. Someone — a human, or a harness typing
into the xterm — has to send the first `UserPromptSubmit`. Stop-hook
chaining only begins after the agent has done at least one turn.

## Driving from automation

Everything a test harness (or another agent) needs to drive an inner
newde agent:

- **Where the agent runs.** Each batch has a tmux pane rendered in the
  first center-area tab. The renderer is `TerminalPane` attached to
  `selectedBatch.pane_target`; UI-side, it's an xterm.js inside
  `.xterm`. Click that element to focus, type with regular keystrokes —
  xterm pipes them through the PTY to claude.
- **When a turn is done.** `deriveBatchAgentStatus`
  (`src/session/agent-status.ts`) reduces hook events to
  `idle | working | waiting | done`; the UI surfaces it as the colored
  dot on each batch tab. Poll for the transition *out* of `working` to
  know a turn finished. Looking at terminal rows alone is fragile
  (scrollback, progress indicators, partial lines).
- **Committing from a driven session.** Two paths, pick one up front:
  - **Ad-hoc.** Tell the agent to run `git commit` directly. No commit
    point needed, no approval UI. Good for "do this one thing and
    land it."
  - **Automated multi-step.** Insert a commit point into the queue
    (`createCommitPoint`, or click `+ Commit when done` in the Plan
    panel) *before* prompting the agent. When the Stop hook sees a
    pending commit point, it blocks with a directive telling the
    agent to call `mcp__newde__propose_commit({ commit_point_id,
    message })`. The point then transitions to `approved` (auto-mode)
    or awaits user approval in the UI (approval-mode) before
    `executeApprovedCommit` runs `gitCommitAll`.
- **Work-item lifecycle.** Create → Stop-hook picks next ready item →
  agent marks `in_progress` → agent works → agent marks `human_check`
  when waiting for user review. Agents **never self-mark `done`** —
  `done` is user-only. Polling "is everything done?" must treat
  `human_check` as terminal-from-the-pipeline's-perspective.

## Common pitfalls

- **`propose_commit` with no active commit point silently fails.** The
  agent logs a suggested message to the terminal and moves on; the
  diff stays uncommitted. If you want an ad-hoc commit, ask for
  `git commit`, not `propose_commit`.
  - **UI signal:** the Work panel renders an inline hint
    (`data-testid="plan-no-commit-point-hint"`) above the bottom bar
    whenever the current batch has at least one `human_check` / `done`
    item and no live (non-`done`, non-`rejected`) commit point — a nudge
    to click `+ Commit when done` so the next Stop-hook turn has
    something to fire. Derived purely on the client from `batchWork` +
    `commitPoints` state; no new store/IPC.
- **Write-guard blocks Edit/Write/MultiEdit/NotebookEdit from any
  non-`active` batch.** See "Write guard" below. If the agent reports
  "permission denied" on a file write inside a non-writer batch,
  that's the hook doing its job — promote the batch to writer or
  switch to the writer batch instead.
- **Queueing work without a prompt does nothing if the agent is
  idle.** See the first-turn caveat above.
- **`human_check` is terminal for the pipeline, not for the user.**
  Pipeline considers all preceding work "done" once it's in
  `human_check`, so commit-point activation doesn't wait for user
  review. That's intentional — it lets the agent land code while the
  user catches up on review asynchronously.

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

0. **Auto-commit synthesis (writer batch only, pre-pass).** Before
   `decideStopDirective` runs, `computeStopDirective` in the runtime checks
   whether `batch.auto_commit` is true, the batch is `active`, no pending
   commit point exists, and settled work (`human_check`/`done` items) is
   present. If so, it calls `batchQueue.createCommitPoint(batchId)` to
   synthesize one. The rest of the pipeline then fires as normal (step 1
   picks it up). This means with auto-commit on the user never needs to
   place a commit point manually — every stop that has settled work will
   trigger a commit proposal.
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

## Orchestrator pattern

The batch agent is a long-lived process that must stay context-lean
across a work queue that could span dozens of items. The solution is
**orchestrator-style dispatch**: the main agent never does Read/Edit/Bash
work directly. Instead it:

1. Calls `newde__read_work_options` to get the next dispatch unit.
2. Launches one `general-purpose` subagent with all item ids, titles,
   descriptions, acceptance criteria, and standing MCP-write instructions.
3. Records the subagent's summary via `add_work_note` and loops.

Subagents run in isolated context windows — their tokens don't count
against the main agent. Main context accumulates only short summaries,
keeping it flat regardless of queue depth. Auto-compact still fires when
the main context fills, but it fires much later and on lean material.

`newde__read_work_options` (defined in `src/mcp/mcp-tools.ts`, backed by
`WorkItemStore.readWorkOptions`) returns one of three shapes:
- `{ mode: "epic", epic, children }` — the highest-priority ready item is
  an epic; all ready descendants (filtered for blocks links, transitively)
  are included as children. Dispatch the entire epic as one unit.
- `{ mode: "standalone", items }` — the head is not an epic; all ready
  non-epic items are returned with link edges inline so the agent can
  pick one or a link-related cluster. Epics are excluded from this list.
- `{ mode: "empty" }` — nothing ready; allow stop.

The Stop-hook directive (step 4 in the pipeline) now says "call
`read_work_options` and dispatch to a `general-purpose` subagent" rather
than naming a specific item. This keeps the directive stable across
epic/task/subtask distinctions and lets the tool do the grouping.

`list_ready_work` remains available for inspection but is no longer the
primary tool for queue-driven dispatch.

## MCP tools

`buildWorkItemMcpTools` (`src/mcp/mcp-tools.ts`) registers the agent's
`newde__*` tool surface:

- `get_batch_context`, `list_batch_work`,
  `list_ready_work`, `read_work_options`, `create_work_item`, `update_work_item`,
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

## Session-context injection

On every `UserPromptSubmit`, the runtime builds a fresh
`<session-context>` block (stream/batch/writer flag) and returns it as
`hookSpecificOutput.additionalContext` so the agent stays pointed at the
right ids mid-session. If a project really wants to save those tokens
(they're tiny but replayed on every turn), set
`injectSessionContext: false` in `newde.yaml` — default is `true`.

## Preamble vs skill split

`buildBatchAgentPrompt` is intentionally terse — session ids, writer
flag, a reminder to reference items by title, and a pointer to the
skill. Procedural policy (when to file, how to shape items, acceptance-
criteria style, status conventions) lives in the `newde-task-management`
skill (`src/session/agent-skills.ts`). Reason: the preamble is replayed
via cache-read on every turn; skills load only when the agent needs
them. Keep additions to the preamble situational, not educational.

## Custom prompt addendum

`config.agentPromptAppend` (loaded from `newde.yaml` via
`loadProjectConfig` in `src/config/config.ts`) is concatenated into every
agent's system prompt by `buildBatchAgentPrompt` (in `runtime.ts`). The
Settings modal (`src/ui/components/SettingsModal.tsx`) reads/writes this
via `runtime.setAgentPromptAppend` which calls `writeProjectConfig` to
persist back to YAML.

A new value applies to **agent sessions started after Save** — existing
sessions keep the prompt they launched with.

After `agentPromptAppend`, `buildBatchAgentPrompt` also appends:
- `# Stream instructions` + `stream.custom_prompt` if the stream has a
  non-empty custom prompt (set via the StreamRail right-click Settings modal,
  persisted to `streams.custom_prompt` — see data-model.md v18).
- `# Batch instructions` + `batch.custom_prompt` if the batch has a
  non-empty custom prompt (set via the BatchRail right-click Settings modal,
  persisted to `batches.custom_prompt` — see data-model.md v18).

These are the last sections before the prompt is finalized, so they can
provide finer-grained overrides without displacing earlier context.

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
