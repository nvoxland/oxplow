# Agent execution model

What this doc covers: how a Claude (or copilot) process is launched in a
thread, how the runtime steers it through the work queue without ever
sending it raw prompts, and the rules that keep non-writer threads from
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

- **Where the agent runs.** Each thread has a tmux pane rendered in the
  first center-area tab. The renderer is `TerminalPane` attached to
  `selectedBatch.pane_target`; UI-side, it's an xterm.js inside
  `.xterm`. Click that element to focus, type with regular keystrokes —
  xterm pipes them through the PTY to claude.
- **When a turn is done.** `deriveBatchAgentStatus`
  (`src/session/agent-status.ts`) reduces hook events to
  `idle | working | waiting | done`; the UI surfaces it as the colored
  dot on each thread tab. Poll for the transition *out* of `working` to
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
    whenever the current thread has at least one `human_check` / `done`
    item and no live (non-`done`) commit point — a nudge to click
    `+ Commit when done` so the next Stop-hook turn has something to
    fire. Derived purely on the client from `batchWork` +
    `commitPoints` state; no new store/IPC.
- **Write-guard blocks Edit/Write/MultiEdit/NotebookEdit from any
  non-`active` thread.** See "Write guard" below. If the agent reports
  "permission denied" on a file write inside a non-writer thread,
  that's the hook doing its job — promote the thread to writer or
  switch to the writer thread instead.
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
(`src/terminal/fleet.ts`). Switching streams or threads doesn't kill
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

`newde__get_batch_context` returns, besides the caller's stream/thread
ids + summary, an `otherActiveBatches: Array<{ streamId, streamTitle,
threadId, batchTitle, activeBatchId }>` with one entry per peer stream —
handy when the agent suspects the "current stream" has drifted from
where it actually writes (the same phenomenon that motivated the
streamId-derivation in other MCP tools).

Each hook POSTs to the runtime's MCP server with bearer-token auth via the
env-var-interpolated `NEWDE_HOOK_TOKEN` header, plus `X-Newde-Stream`,
`X-Newde-Thread`, `X-Newde-Pane`. The MCP server's `onHook` callback dispatches
to `runtime.handleHookEnvelope`, which:

1. Stores the event in `HookEventStore` (a ring buffer, also fed to the UI's
   Hook Events tool window via the `hook.recorded` EventBus event).
2. If the normalized payload carries a session id that differs from
   `thread.resume_session_id`, persists the new id so a later newde restart
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
   live `<session-context>` block (stream + thread + writer, rebuilt
   from the stores — see `buildSessionContextBlock` in `runtime.ts`)
   followed by the editor-focus summary from
   `src/session/editor-focus.ts`. The session-context block refreshes
   on every turn so the agent notices when the user promoted a
   different thread to writer mid-session; the frozen ids in the
   launch-time system prompt no longer win.
6. For `Stop`: runs `computeStopDirective` (below).

## Stop-hook pipeline

The decision logic lives in `decideStopDirective` (a pure function in
`src/electron/stop-hook-pipeline.ts`). The runtime's
`computeStopDirective(threadId)` builds a `BatchSnapshot` from the live
stores, calls the pure function, then applies any returned side effects
(currently only `trigger-wait-point`). Keeping the decision separate
from the side effects lets every branch be unit-tested with a fixture.

The pipeline runs in priority order:

1. **Pending commit point (writer thread only).** Block with an
   agent-drafted commit directive. Two shapes:
   - `mode="approve"` (default) → `buildCommitPointStopReason`: agent
     inspects the diff, drafts a message in chat, asks the user to
     approve, then calls `mcp__newde__commit({ commit_point_id, message })`.
   - `mode="auto"` → `buildAutoCommitStopReason` (same directive text
     used for the no-row ad-hoc case below): agent inspects the diff,
     drafts a message, and commits **in the same turn without asking**.
     Same tool call with the commit_point_id — the runtime flips the
     row to `done`.
   **Gated on `thread.status === "active"`**: only the writer thread
   ever commits. Non-active threads fall through so the commit point
   stays pending until that thread is promoted to writer.
2. **Ad-hoc auto-commit (writer thread only, no DB row).** When
   `thread.auto_commit` is true AND settled work (`human_check`/`done`
   items) is present AND no pending commit_point is queued AND the
   worktree has staged/unstaged changes, the pipeline emits
   `buildAutoCommitStopReason(null)`. Clean-tree suppression: if
   `git diff --quiet && git diff --cached --quiet` both pass
   (`isWorktreeClean` in `src/git/git.ts`), the directive is skipped
   even when settled work exists — the sha already landed via an
   ad-hoc `git commit` (Bash / Files panel) and there's nothing
   to commit. Same suppression applies to the `mode="auto"`
   commit_point branch above. Agent drafts a message from the diff
   and calls `mcp__newde__commit({ auto: true, threadId, message })`,
   which routes to `executeAutoCommitForThread` — runs `git commit`
   and publishes a `thread.changed`/`auto-committed` event. **No
   commit_point row is created, and no item↔sha bookkeeping happens**
   (item↔commit attribution can't be made reliable when users commit
   outside newde — see data-model.md for why we removed the
   `work_item_commit` junction). This is the unified-flow endpoint:
   auto and approve modes now only differ in whether the agent asks
   the user first.
   - Supplementary context tool: `mcp__newde__tasks_since_last_commit`
     returns work items whose efforts closed after the most recent done
     commit_point (or all closed efforts when the thread has never
     committed). The agent uses it when it's lost memory of earlier
     completed tasks; the diff is still the primary source of truth.
   - Fallback: if the agent calls `newde__commit` with `auto: true` but
     an empty `message`, the runtime falls back to `buildAutoCommitMessage`
     — now filtered by the latest done commit_point's `completed_at`
     (items whose `updated_at` is strictly after it), so the body
     reflects *this* commit's changes rather than every settled item
     ever.
3. **Pending wait point.** Flip it to `triggered` and **allow stop**. The
   UI shows "agent stopped here"; the user resumes by prompting the agent
   directly (`triggered` points are skipped on subsequent Stop hooks, so
   one prompt is enough — no "continue" button). A marker (commit or wait)
   is "active" once every preceding work item is terminal, where terminal
   includes `human_check` — agents never self-mark `done`, so demanding
   `done` would let the agent march past the line indefinitely while the
   user catches up on verification.
4. **Writer thread with a ready work item.** Block with a terse directive
   built by `buildNextWorkItemStopReason` — a one-liner pointing at
   `mcp__newde__read_work_options` (with the embedded threadId) and the
   merged `newde-runtime` skill. Protocol
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

   **Ready-work suppression rules** (apply to this branch only — commit /
   wait / auto-commit are unaffected):

   - **Activity-based suppression.** The runtime tracks whether the
     currently-open turn has fired any mutation / filing / dispatch tool
     call (`turnActivityByThread`; seeded `false` on UserPromptSubmit,
     flipped to `true` on the first qualifying PostToolUse). Qualifying
     tools: every write-intent tool (Edit / Write / MultiEdit /
     NotebookEdit / Bash with a non-readonly command — see
     `isWriteIntentTool`), the newde filing tools
     (`create_work_item`, `file_epic_with_children`, `complete_task`,
     `update_work_item`, `transition_work_items`, `add_work_note`), and
     the dispatch tools (`Task`, `dispatch_work_item`). If the flag is
     still `false` at Stop, the ready-work directive is skipped — the
     turn was pure Q&A / planning / review and force-marching the agent
     to the next ready item inverts the user's intent. Any activity
     re-arms the directive even when the prompt looked conversational.
     Replaces an older regex-on-the-user-prompt check that over-fired
     whenever a question-shaped prompt produced real work. An
     `undefined` flag (UserPromptSubmit never fired) is treated as
     "unknown → do not suppress" so behaviour stays stable when turn
     tracking is missing.
   - **Just-read suppression.** The runtime keeps a per-thread record
     (`lastReadWorkOptionsByThread`) of the ready-item ids the agent saw
     in its most recent `read_work_options` call. On the next Stop, if
     the current ready-set is identical, the directive is suppressed and
     the record is cleared — the agent already has the list; re-emitting
     it would waste a turn.

5. **Otherwise.** Allow stop.

### fork_thread + high-cache-read hint

At scale a single thread accumulates cache-read cost across its
lifetime; once it's climbed past ~20M the replay-on-every-turn tax
starts to dominate. To let the orchestrator shed the tail, the runtime
exposes `mcp__newde__fork_thread({ sourceThreadId, title, summary,
moveItemIds? })` — one transaction that:

1. Creates a new thread on the same stream, status `queued` (never
   auto-writer — promote explicitly if you want it to commit).
2. Seeds the new thread with a single `note`-kind work item titled
   "Context from fork" whose description is the caller-supplied
   `summary` (no schema change — the `note` kind already exists on
   `work_items`).
3. Optionally moves each `moveItemIds` entry over via
   `WorkItemStore.moveItemToThread`. Items must currently be `ready` or
   `blocked` on the source thread; `in_progress` / `human_check` /
   terminal items are rejected with an error listing the offenders so
   the caller can settle them first.
4. For each moved item, copies its last 3 notes (by `created_at DESC`,
   re-inserted in chronological order) as fresh rows on the same item
   id via `WorkItemStore.copyLastItemNotes`. Source rows are untouched.
   Items with fewer than 3 notes copy all; items with none are no-ops.
   The user landing in the forked thread sees decisions/rationale
   carried over rather than a bare title.

Returns `{ newThreadId }`. Implementation lives on
`ElectronRuntime.forkThread` (`src/electron/runtime.ts`); the MCP tool
is just a thin surface.

**Cumulative cache-read hint.** `TurnStore.getCumulativeCacheRead(threadId)`
sums `cache_read_input_tokens` across every closed turn on the thread.
When ≥20M AND the ready-work directive is about to be emitted (i.e.
not suppressed by the rules above), the directive text has a
trailing line appended:

> `note: this thread has burned <N.N>M cache-read. If upcoming work is unrelated, consider newde__fork_thread({ sourceThreadId: "<id>", title: "...", summary: "short carry-over context" })`

The hint is a nudge, not a requirement — the orchestrator decides
whether the tail really is unrelated. Commit-point and wait-point
directives don't carry the hint; only the ready-work branch does.

Numbering note: ad-hoc auto-commit (no DB row) runs *after* the
commit_point branch because a pending commit_point always wins — if the
user has explicitly queued a commit marker, honour it; ad-hoc
auto-commit is the "draft me something" catch-all that fires only when
nothing more specific is in the queue.

## BatchQueueOrchestrator

Cross-store queue logic (commit points + wait points + the
`reorderBatchQueue` that rewrites all three sort_index spaces) lives
in `src/electron/thread-queue-orchestrator.ts`. The runtime instantiates
it as `this.batchQueue` and delegates IPC-exposed methods through.
`executeCommit` (which runs `git commit` synchronously from the
`newde__commit` MCP handler once the user has approved in chat) is
also there.

## Orchestrator pattern

The thread agent is a long-lived process that must stay context-lean
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
stuck) is identical for both modes and lives in the merged
`newde-runtime` skill (orchestrator side — filing + lifecycle +
dispatch combined) plus the `newde-subagent-work-protocol` skill
(scoped to subagents). Briefs no longer need to repeat it.

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
  `link_work_items`, `add_work_note`, `list_recent_file_changes`,
  `dispatch_work_item`, `file_epic_with_children`, `complete_task`,
  `transition_work_items`
- `dispatch_work_item({ threadId, itemId, extraContext?, autoStart? })` composes
  a subagent brief server-side (preamble + item fields + children + last notes
  + optional extra context) so the orchestrator doesn't have to Read the item
  description/AC/notes into chat context. Default `autoStart=true` atomically
  transitions `ready`/`blocked` items to `in_progress`; other statuses are
  left alone. Callers pass the returned `prompt` directly to Agent(prompt=…).
  Pure composition lives in `composeDispatchBrief` (same file) so tests can
  exercise it without spinning up MCP.
- `commit({ commit_point_id, message } | { auto: true, threadId, message })`,
  `list_commit_points(threadId)`, `tasks_since_last_commit(threadId)`.
- `fork_thread({ sourceThreadId, title, summary, moveItemIds? })` — see
  "fork_thread + high-cache-read hint" above. Creates a new queued
  thread on the same stream, seeds a note item, optionally moves ready/
  blocked items across in one transaction.
  The auto shape is used for ad-hoc commits when `thread.auto_commit=true`
  and no commit_point row is queued — it just runs `git commit` on the
  thread's worktree and publishes `auto-committed`. `tasks_since_last_commit`
  is supplementary context for the auto-commit draft (not the primary
  source — the diff is).

`buildLspMcpTools` (`src/mcp/lsp-mcp-tools.ts`) adds language-server
queries (definition, references, hover) the agent can use without
shelling out.

## Write guard

Non-writer threads share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** `buildWriteGuardResponse`
  (`src/electron/write-guard.ts`) returns a `PreToolUse` deny for `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` from any non-`active` thread. When
  the tool's target path resolves OUTSIDE the project root AND outside
  the project's `.newde/`, the call is allowed (e.g. writing to
  `~/.claude/plans/foo.md`); the deny message names the specific
  absolute path. Containment checks use `isInsideWorktree` from
  `src/electron/runtime-paths.ts`, shared with the runtime's hook-path
  filter.
- **Prompt enforcement.** `NON_WRITER_PROMPT_BLOCK` (same file) is
  appended to the system prompt for non-writer threads, telling the agent
  to avoid Bash mutations too (the hook can't reliably classify shell
  commands, so the prompt is the only line of defence there).
- MCP tools (`mcp__newde__*`) are always allowed: they write to the state
  DB, not the worktree.

## Dev-time MCP live-reload (opt-in)

Set `NEWDE_DEV_RELOAD=1` before launching the runtime to watch
`src/mcp/` and `src/persistence/` recursively. On any `.ts`/`.tsx`
change, a debounced (250ms) restart stops the current MCP server and
calls `startMcpServer` again so the rebuilt tool registrations and a
fresh TCP port + lockfile are live.

**Known limitation.** ESM caches imported modules by URL, so
re-invoking `buildWorkItemMcpTools` returns the *same* in-memory
module graph — an edit to handler source still needs a full runtime
restart to actually pick up new logic. The watcher still has value: it
logs the triggering file loudly so the dev knows a restart is due,
and it rebinds the port + lockfile (useful after a stale lockfile
survives a crash). Full hot-reload would require either a child-
process MCP model or a `bun --hot`-style process reload, both bigger
changes than this dev convenience warrants. Tracked on
wi-4c3a6289871f.

Zero runtime cost when the env var is unset; the source-root probe
doesn't run at all in that case.

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
`<session-context>` block (stream/thread/writer flag) and returns it as
`hookSpecificOutput.additionalContext` so the agent stays pointed at the
right ids mid-session. The runtime caches the last-emitted block per
Claude session id (`lastSessionContextBySessionId`) and **skips emission
when the candidate block is byte-identical to what was already sent** —
re-sending the same string is pure overhead since the agent's prompt
cache still holds the prior value. The first turn on a session, and any
turn after the block's contents change (thread flip, writer promotion,
title edit), emits normally. If a project wants to disable injection
entirely, set `injectSessionContext: false` in `newde.yaml` — default is
`true`.

### ROLE CHANGE banner

The initial system prompt's `NON_WRITER_PROMPT_BLOCK` is frozen at
launch and replayed via cache-read on every turn, so a mid-session
writer promotion used to leave the agent acting read-only long after
the UI flipped it. To supersede the stale block in-place,
`buildSessionContextBlock` accepts an `initialRole` input and appends a
loud `ROLE CHANGE:` line before `</session-context>` when the current
role differs from it. `buildRefreshedSessionContext` (in `runtime.ts`)
captures the role once per Claude session id in
`initialRoleBySessionId`, keyed off the first `UserPromptSubmit` the
runtime sees for that session, so the comparison baseline is stable
across subsequent turns. Both directions are covered:

- **read-only → writer.** "The NON_WRITER block in your initial system
  prompt is SUPERSEDED — you may now use Write/Edit/Bash to mutate the
  worktree."
- **writer → read-only.** "The NON_WRITER block applies now even though
  it wasn't in your initial system prompt — Write/Edit/Bash mutations
  to the worktree will be blocked."

No banner is emitted when the role has not changed, so steady-state
turns don't grow.

### last_turn_cache_read cost hint

`buildSessionContextBlock` also accepts an optional `lastTurnCacheRead`
input — populated by `buildRefreshedSessionContext` via
`TurnStore.getLastClosedTurnCacheRead(threadId)` (the most recent
`agent_turn` row for the thread with `ended_at IS NOT NULL`, returning
its `cache_read_input_tokens` or null). When ≥1000 it renders a
`last_turn_cache_read: <N>K` line; ≥1,000,000 flips to `<N.N>M`. Values
below 1000 are omitted as noise. When the value hits ≥10,000,000 a
separate `tip: dispatch new work to subagents — inline turns compound
cache-read cost` line is appended before `</session-context>` — at
that scale the cache-read cost of replaying conversation history on
every inline tool call starts to dominate, and dispatching to a
subagent amortizes the replay. The hint is a nudge, not a hard rule;
the orchestrator decides based on the work shape.

`buildSessionContextBlock` also accepts an optional `currentTurnBytes`
— a rough running estimate of the current turn's tool-result bytes so
far, accumulated by the runtime in a per-thread
`currentTurnBytesByThread` Map on each `PostToolUse` envelope
(`estimateToolResponseBytes` just measures the serialized
`tool_response`). When non-trivial it's rendered as a suffix on the
`last_turn_cache_read` line, e.g. `last_turn_cache_read: 19.3M (this
turn: ~2.0M so far)`, so a mid-turn session-context refresh surfaces a
non-stale cost signal. The map is cleared on `UserPromptSubmit` (new
turn) and `Stop` (turn closed). First turn of a session renders only
the last-turn value, since `currentTurnBytes` is undefined.

## Preamble vs skill split

`buildBatchAgentPrompt` is intentionally terse — session ids, writer
flag, and a pointer to the skills. Procedural policy is consolidated in
one orchestrator-side skill (`src/session/agent-skills.ts`):
`newde-runtime` merges filing (when to file, how to shape items,
acceptance-criteria style, epic-with-children rule), lifecycle
(status conventions, epic rollup, notes), and dispatch (orchestrator
vs subagent execution mode, brief composition). Its description
combines all trigger contexts so it still loads when any of the
legacy invocation paths apply, but contributes a single index line
per turn instead of three.
Reason: the preamble is replayed via cache-read on every turn; skills
load only when the agent needs them. Keep additions to the preamble
situational (what changes per thread), not educational (how to use the
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
- `# Thread instructions` + `thread.custom_prompt` if the thread has a
  non-empty custom prompt (set via the BatchRail right-click Settings modal,
  persisted to `threads.custom_prompt` — see data-model.md v18).

These are the last sections before the prompt is finalized, so they can
provide finer-grained overrides without displacing earlier context.

## Agent status

`deriveBatchAgentStatus` (`src/session/agent-status.ts`) reduces a stream
of hook events into one of `idle | working | waiting | done`. The runtime
recomputes on every hook arrival and emits `agent-status.changed`. The UI
shows it as a colored dot on each thread tab.

## Snapshot tracking

The runtime keeps a content-addressed history of worktree files so the
UI (and future tooling) can render turn- and effort-level diffs without
relying on git. Snapshots are **time-ordered** and deduplicated by a
`version_hash` over `(path, hash, size, state)` tuples — there is no
parent chain, and two flushes of an unchanged worktree return the same
snapshot id. Mechanics:

- A per-stream in-memory **dirty set** accumulates relative paths. It
  is populated by the workspace fs-watcher (always, regardless of
  thread state) and by the PostToolUse hook's `markDirty` branch in
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

## Per-effort write log

Snapshot pair-diffs over-report when two subagents edit the same worktree
in parallel: both efforts share the same window, so each shows the
union. To attribute writes correctly the agent declares its touched
files on the status transition that closes the effort; the runtime
stores them in `work_item_effort_file` (see data-model.md).

**Agent-declared payload.** When calling `update_work_item` with
`status: "human_check"`, the agent passes `touchedFiles: string[]` —
the repo-relative paths it wrote or edited during this effort.
`applyStatusTransition` (in `runtime.ts`) captures the open effort id,
flushes the task-end snapshot, closes the effort, and then inserts
`work_item_effort_file` rows for each deduped path via `INSERT OR
IGNORE`. Payloads larger than `TOUCHED_FILES_CAP` (100 paths) drop all
rows, so the "assume all" fallback engages in `computeEffortFiles`.
The PostToolUse hook no longer writes to `work_item_effort_file`; the
previous active-effort heuristic was unreliable whenever ≥2 efforts
were in_progress (the common case the log is meant to cover).

**1-vs-many rendering rule.** The Local History panel renders one row
per effort ending at a snapshot, *not* one row per snapshot. For a
snapshot `S`:

- 0 efforts end at S → single "External Change" / source-labelled row
  (unchanged from pre-write-log behaviour).
- 1 effort ends at S → one row labelled with the work item title;
  detail pane uses `getEffortFiles(effortId)`, which short-circuits to
  the raw pair-diff.
- ≥2 efforts end at S → one row per effort, each labelled with its
  work item title; detail panes call `getEffortFiles(effortId)`. If
  the effort has ≥1 `work_item_effort_file` row the pair-diff is
  filtered to those paths; if it has 0 rows (agent skipped the
  `touchedFiles` payload, or list exceeded the cap) we fall back to
  the raw pair-diff — better to over-report than silently show empty.

`getEffortFiles` is exported from `runtime.ts` as `computeEffortFiles`
(pure helper over the two stores) for test reuse, and wired to IPC via
the same pattern as `getSnapshotSummary`.

## Active turns in the in_progress bucket (observational)

Newde passively tracks what the agent is doing: no synthesized work
items, no auto-file / auto-complete / adoption. The Work panel's
in_progress bucket renders the union of real work items plus **open
`agent_turn` rows** — each turn with `ended_at IS NULL` AND
`started_at >= runtime.startedAt` shows up as a synthetic row
displaying the user's prompt, a "thinking…" indicator, elapsed time,
and the current TaskCreate breakdown (from `agent_turn.task_list_json`).
When the turn Stops, `ended_at` is set and the row disappears from the
bucket. No status flips, no notes, no cleanup.

The `runtime.startedAt` cutoff is load-bearing: when newde crashes
mid-turn, `ended_at` never gets set; filtering to turns started after
the current runtime boot keeps those orphans out of the UI.

**TaskCreate/TaskUpdate bridge.** The PostToolUse hook writes the
declarative TodoWrite payload to `agent_turn.task_list_json` on every
call (not only at Stop), so the open-turn row in the Work panel
renders the live sub-list as the agent ticks steps off. The column
persists after the turn closes for a later History view.

If a turn spawns real follow-up work, the agent calls
`mcp__newde__create_work_item` / `file_epic_with_children` the way it
always has. Those land as first-class work items alongside the turn
row, with a normal ready → in_progress → human_check lifecycle.

## Recent answers — inactive closed turns

The Work panel renders a "Recent answers" section below the in_progress
bucket (open-turn rows) and above Human Check, populated by
`runtime.listRecentInactiveTurns(threadId, limit?)` which delegates to
`TurnStore.listRecentInactiveTurns`. The section surfaces closed
`agent_turn` rows with `produced_activity = 0` — turns where the agent
answered a question / did planning / reviewed without firing any
mutation / filing / dispatch tool call.

At Stop, the runtime captures `turnActivityByThread[threadId]` (the
same flag that drives the ready-work suppression rule) and persists
it via `setProducedActivity(turnId, flag)` right after `closeTurn` and
before `computeStopDirective` reads+clears the in-memory entry. A
missing map entry (UserPromptSubmit never fired) is stored as NULL;
NULL rows are excluded from the Recent-answers query so pre-migration
turns don't back-fill the list.

UI: double-click a row to open a modal with the full prompt + answer
(Escape-dismissible, no drag, no right-click — observational only).
See `src/ui/components/Plan/RecentAnswersList.tsx`.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` and the
  approved-commit-point execution loop.
