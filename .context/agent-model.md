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

### What we can't do from newde hooks

Claude Code inserts its own `<system-reminder>` blocks into user
messages — for example, the periodic "The task tools haven't been used
recently; consider using TaskCreate" nudge. **Hooks can add context to
a prompt but cannot edit existing system-reminders out**, so newde has
no way to suppress these from the agent's view. Related asks (e.g.
"don't nag about TaskCreate while a newde work item is in_progress")
require upstream Claude Code support; a newde-side "just inject a
counter-instruction" workaround would leave both the nag and the
counter-nag visible, which is worse than the status quo. If Claude
Code ever ships a hook-surface knob for this, revisit.

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
    agent to inspect the diff, draft a message in its reply, and ask
    the user to approve. On approval the agent calls
    `mcp__newde__commit({ commit_point_id, message })`, which runs
    `gitCommitAll` and flips the point to `done`. Auto-mode commit
    points skip the chat round-trip — the runtime commits immediately
    with a generated message.
- **Work-item lifecycle.** Create → Stop-hook picks next ready item →
  agent marks `in_progress` → agent works → agent marks `human_check`
  when waiting for user review. Agents **never self-mark `done`** —
  `done` is user-only. Polling "is everything done?" must treat
  `human_check` as terminal-from-the-pipeline's-perspective.

## Common pitfalls

- **`mcp__newde__commit` only works when a commit point is pending.**
  The tool requires an existing commit_point row; for ad-hoc commits
  have the agent run `git commit` directly via Bash.
  - **UI signal:** the Work panel renders an inline hint
    (`data-testid="plan-no-commit-point-hint"`) above the bottom bar
    whenever the current batch has at least one `human_check` / `done`
    item and no live (non-`done`) commit point — a nudge to click
    `+ Commit when done` so the next Stop-hook turn has something to
    fire. Derived purely on the client from `batchWork` +
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

0. **Auto-commit pre-pass (writer batch only, no DB row).** Before
   `decideStopDirective` runs, `computeStopDirective` in the runtime checks
   whether `batch.auto_commit` is true, the batch is `active`, and settled
   work (`human_check`/`done` items) is present. If so, it calls
   `runAutoCommit(batch, workItems)` directly —
   that generates a message from settled titles, runs `git commit`, and
   emits a `batch.changed` event with kind `auto-committed`. **No
   commit_point row is created** in auto-commit mode (the "Commit · Auto"
   row in the queue UI was pure noise). Manually-placed commit points with
   mode=auto still go through `executeAutoCommitPoint`, which calls the
   same `runAutoCommit` helper and then flips its own DB row to `done`.
1. **Pending commit point (writer batch only).** Block with a directive
   built by `buildCommitPointStopReason` telling the agent to inspect the
   diff, draft a concise commit message in its chat reply, and ask the
   user to approve. On approval the agent calls
   `mcp__newde__commit({ commit_point_id, message })` which runs
   `gitCommitAll` and flips the point to `done`. The drafted message
   lives only in chat — there is no DB column for it. **Gated on
   `batch.status === "active"`**: only the writer batch ever commits.
   Non-active batches fall through so the commit point stays pending
   until that batch is promoted to writer.
2. **Pending wait point.** Flip it to `triggered` and **allow stop**. The
   UI shows "agent stopped here"; the user resumes by prompting the agent
   directly (`triggered` points are skipped on subsequent Stop hooks, so
   one prompt is enough — no "continue" button). A marker (commit or wait)
   is "active" once every preceding work item is terminal, where terminal
   includes `human_check` — agents never self-mark `done`, so demanding
   `done` would let the agent march past the line indefinitely while the
   user catches up on verification.
3. **Writer batch with a ready work item.** Block with a terse directive
   built by `buildNextWorkItemStopReason` — a one-liner pointing at
   `mcp__newde__read_work_options` (with the embedded batchId) and the
   `newde-task-dispatch` + `newde-task-lifecycle` skills. Protocol
   (mark `in_progress` before work, `human_check` after, one-at-a-time
   attribution) lives in those skills, not the directive — keep the
   stop-hook message stable and cheap. Items the
   agent itself filed during the *current* turn
   are skipped (ready-list filtered by `created_by="agent"` AND
   `created_at >= currentTurnStartedAt`). Those are triage-inbox entries
   from flows like `/autoimprove`; forcing the agent to pick them up would
   invert the user's "leave in waiting" intent. "Queue" here means
   waiting + ready items only — `human_check` belongs to the user's
   review queue and is terminal from the pipeline's perspective.
4. **Otherwise.** Allow stop.

Auto-mode commit points are handled by the runtime's Stop-hook pre-pass
before the pipeline runs: `runAutoCommit` generates a message from the
settled work items, `gitCommitAll` runs, and the point flips straight to
`done`. No human in the loop and no agent chat round-trip.

## BatchQueueOrchestrator

Cross-store queue logic (commit points + wait points + the
`reorderBatchQueue` that rewrites all three sort_index spaces) lives
in `src/electron/batch-queue-orchestrator.ts`. The runtime instantiates
it as `this.batchQueue` and delegates IPC-exposed methods through.
`executeCommit` (which runs `git commit` synchronously from the
`newde__commit` MCP handler once the user has approved in chat) is
also there.

## Orchestrator pattern

The batch agent is a long-lived process that must stay context-lean
across a work queue that could span dozens of items. Every file change
is filed as a work item first (traceability IS the point — local
history attributes snapshots back to the sole in-progress item). Past
that, the orchestrator has two modes:

1. **Inline small-fix shortcut.** For mechanical, low-risk changes (≤
   ~20 lines across ≤ 2 files — test fixtures, import cleanup, label
   renames), the orchestrator does the Read/Edit/Bash directly under
   the work item. Mark `in_progress`, edit, run tests, mark
   `human_check`. Snapshots still fire with correct attribution; we
   just skip the subagent round-trip.
2. **Subagent dispatch for bigger work.** For multi-file/multi-step/
   risky changes, the orchestrator calls `newde__read_work_options`,
   launches one `general-purpose` subagent with the brief, and
   records the summary via `add_work_note`. Subagents run in isolated
   context windows — their tokens don't count against the orchestrator,
   so main context stays flat regardless of queue depth.

The dispatch protocol (mark `in_progress` before work, `human_check`
after, never two items `in_progress` at once, blocked + note on
stuck) is identical for both modes and lives in the
`newde-task-lifecycle` skill (orchestrator side) plus the
`newde-subagent-work-protocol` skill (scoped to subagents). Briefs no
longer need to repeat it.

Related small fixes get batched into one task ("fix 4 test fixtures" =
one item, not four). Claude Code's built-in `TaskCreate` is a
within-turn micro-planner and never mirrors newde items.

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
- `commit(commit_point_id, message)`, `list_commit_points(batchId)`

`buildLspMcpTools` (`src/mcp/lsp-mcp-tools.ts`) adds language-server
queries (definition, references, hover) the agent can use without
shelling out.

## Write guard

Non-writer batches share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** `buildWriteGuardResponse`
  (`src/electron/write-guard.ts`) returns a `PreToolUse` deny for `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` from any non-`active` batch. When
  the tool's target path resolves OUTSIDE the project root AND outside
  the project's `.newde/`, the call is allowed (e.g. writing to
  `~/.claude/plans/foo.md`); the deny message names the specific
  absolute path. Containment checks use `isInsideWorktree` from
  `src/electron/runtime-paths.ts`, shared with the runtime's hook-path
  filter.
- **Prompt enforcement.** `NON_WRITER_PROMPT_BLOCK` (same file) is
  appended to the system prompt for non-writer batches, telling the agent
  to avoid Bash mutations too (the hook can't reliably classify shell
  commands, so the prompt is the only line of defence there).
- MCP tools (`mcp__newde__*`) are always allowed: they write to the state
  DB, not the worktree.

## MCP tool deferral is a harness decision

Claude Code defers MCP tool schemas (surfacing them as names only until
`ToolSearch` fetches the schema) based on its own heuristics — it is
**not** a signal the MCP server sends. `tools/list` already reports
every newde tool with full `inputSchema`; the harness picks which to
eagerly inline vs defer. There is no MCP-spec annotation and no plugin
config knob to declare a tool "always loaded". If this ever becomes
tunable, the wiring is `src/mcp/mcp-server.ts` `tools/list` response +
`src/mcp/mcp-tools.ts` tool registrations (see wi-2998dfa502da).

## Harness-injected system-reminders (not ours)

A few system-reminders come from the Claude Code harness itself, not
newde hooks, and are **not suppressible** from the plugin side:

- "The task tools haven't been used recently…" — harness nudge about
  `TaskCreate`/`TaskUpdate`. Noise in newde projects where work items
  live in `mcp__newde__*` tools instead. No hook, env var, or plugin
  config lets us silence it; it fires on its own schedule. If a future
  Claude Code release exposes a suppression hook, revisit wi-2a0262ae2ac2.
- The file-in-IDE reminder ("The user opened the file X in the IDE.
  This may or may not be related to the current task.") — same story,
  harness-injected on IDE focus, not a newde hook. Revisit if Claude
  Code adds a customization hook.

## Session-context injection

On every `UserPromptSubmit`, the runtime builds a fresh
`<session-context>` block (stream/batch/writer flag) and returns it as
`hookSpecificOutput.additionalContext` so the agent stays pointed at the
right ids mid-session. The runtime caches the last-emitted block per
Claude session id (`lastSessionContextBySessionId`) and **skips emission
when the candidate block is byte-identical to what was already sent** —
re-sending the same string is pure overhead since the agent's prompt
cache still holds the prior value. The first turn on a session, and any
turn after the block's contents change (batch flip, writer promotion,
title edit), emits normally. If a project wants to disable injection
entirely, set `injectSessionContext: false` in `newde.yaml` — default is
`true`.

## Preamble vs skill split

`buildBatchAgentPrompt` is intentionally terse — session ids, writer
flag, and a pointer to the skills. Procedural policy is split across
three focused orchestrator-side skills (all in
`src/session/agent-skills.ts`):
`newde-task-filing` (when to file, how to shape items,
acceptance-criteria style, epic-with-children rule),
`newde-task-lifecycle` (status conventions, epic rollup, notes), and
`newde-task-dispatch` (orchestrator vs subagent execution mode,
brief composition). Each skill has a targeted description so only
the relevant one loads per turn.
Reason: the preamble is replayed via cache-read on every turn; skills
load only when the agent needs them. Keep additions to the preamble
situational (what changes per batch), not educational (how to use the
tools).

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
UI (and future tooling) can render turn- and effort-level diffs without
relying on git. Snapshots are **time-ordered** and deduplicated by a
`version_hash` over `(path, hash, size, state)` tuples — there is no
parent chain, and two flushes of an unchanged worktree return the same
snapshot id. Mechanics:

- A per-stream in-memory **dirty set** accumulates relative paths. It
  is populated by the workspace fs-watcher (always, regardless of
  batch state) and by the PostToolUse hook's `markDirty` branch in
  `applyTurnTracking`. No separate per-path log is kept — the dirty
  set is passed to `SnapshotStore.flushSnapshot` as an optimizer hint
  so only changed paths need restat, and every other entry carries
  forward from the previous snapshot.
- On `UserPromptSubmit`, after the new `agent_turn` row is opened,
  `flushSnapshotForStream(streamId, "turn-start")` runs. The returned
  id (whether freshly created or a dedup match against the latest
  existing snapshot for the stream) is stored on
  `agent_turn.start_snapshot_id`, and `linkOpenEffortsToTurn` attaches
  every currently-open `work_item_effort` to the new turn via
  `work_item_effort_turn`.
- On `Stop`, after the turn is closed, `flushSnapshotForStream` runs
  again with `source: "turn-end"`; the id is written to
  `agent_turn.end_snapshot_id`.
- On project open, `takeStartupSnapshot` runs once per stream — a full
  worktree walk that emits `source: "startup"`. If nothing changed
  while the app was down, `version_hash` dedup returns the existing
  snapshot and no new row is written; otherwise a fresh one is
  recorded so the "changes during downtime" are visible.
- On work-item status transitions, `handleStatusTransition` (and the
  pure `applyStatusTransition` helper it delegates to) runs. A
  transition *into* `in_progress` flushes `source: "task-start"` and
  opens a new `work_item_effort` row pointing at it; a transition
  *out of* `in_progress` (to `human_check`, `done`, `canceled`,
  `blocked`, etc.) flushes `source: "task-end"` and closes the effort.
  Re-entering `in_progress` creates a second effort — efforts are a
  per-cycle record, not a single lifetime span. A DB-level UNIQUE
  partial index on `work_item_effort(work_item_id) WHERE ended_at IS
  NULL` enforces "at most one open effort per item."
- Turn-level diffs come from
  `getSnapshotPairDiff(turn.start_snapshot_id, turn.end_snapshot_id,
  path)` and `getSnapshotSummary(endId, startId)`; effort-level diffs
  are the analogous pair on `work_item_effort.start_snapshot_id` /
  `end_snapshot_id`, exposed to the UI via `workItemApi.listWorkItemEfforts`.

See [data-model.md](./data-model.md) for the `file_snapshot`,
`work_item_effort`, and `work_item_effort_turn` schemas, and
[ipc-and-stores.md](./ipc-and-stores.md) for the `file-snapshot.created`
EventBus event and the snapshot/effort IPC methods.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` and the
  approved-commit-point execution loop.
