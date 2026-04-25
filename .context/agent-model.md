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
3. MCP tool responses (when the agent calls a `oxplow__*` tool).

Auto-progression through the queue is built entirely on (2). The agent
thinks it's about to stop; the harness says "actually, do this next."

### What we can't do from oxplow hooks

Claude Code inserts its own `<system-reminder>` blocks into user
messages — for example, the periodic "The task tools haven't been used
recently; consider using TaskCreate" nudge. **Hooks can add context to
a prompt but cannot edit existing system-reminders out**, so oxplow has
no way to suppress these from the agent's view. Related asks (e.g.
"don't nag about TaskCreate while a oxplow work item is in_progress")
require upstream Claude Code support; a oxplow-side "just inject a
counter-instruction" workaround would leave both the nag and the
counter-nag visible, which is worse than the status quo. If Claude
Code ever ships a hook-surface knob for this, revisit.

**Caveat — the first turn still needs a user prompt.** "Runtime never
prompts" is about auto-progression, not cold-start. When the agent is
sitting idle at its shell prompt (e.g. just after `oxplow` opens a
fresh project, or after a `Stop` that didn't block), creating a work
item does **not** kick it off. Someone — a human, or a harness typing
into the xterm — has to send the first `UserPromptSubmit`. Stop-hook
chaining only begins after the agent has done at least one turn.

## Driving from automation

Everything a test harness (or another agent) needs to drive an inner
oxplow agent:

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
    `mcp__oxplow__commit({ commit_point_id, message })`, which runs
    `gitCommitAll` and flips the point to `done`. Auto-mode commit
    points skip the chat round-trip — the runtime commits immediately
    with a generated message.
- **Work-item lifecycle.** Create → Stop-hook picks next ready item →
  agent marks `in_progress` → agent works → agent marks `human_check`
  when waiting for user review. Agents **never self-mark `done`** —
  `done` is user-only. Polling "is everything done?" must treat
  `human_check` as terminal-from-the-pipeline's-perspective.

## Common pitfalls

- **`mcp__oxplow__commit` only works when a commit point is pending.**
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
- runs `claude --plugin-dir <abs> --allowedTools mcp__oxplow__* --append-system-prompt <text> --mcp-config <json> [--resume <sid>]`
- exports `OXPLOW_STREAM_ID`, `OXPLOW_BATCH_ID`, `OXPLOW_HOOK_TOKEN` so the
  plugin's HTTP hooks can identify themselves to the runtime

`copilot` is supported as an alternative agent kind but skips the plugin
plumbing — it just `cd && exec copilot`.

The command is launched in a tmux pane via `ensureAgentPane`
(`src/terminal/fleet.ts`). Switching streams or threads doesn't kill
existing agent sessions; tmux keeps them alive in the background.

## Plugin hook bridge

`createElectronPlugin` (`src/session/claude-plugin.ts`) writes a per-project
Claude Code plugin into `<projectDir>/.oxplow/runtime/claude-plugin/`. The
plugin's `hooks.json` registers HTTP hooks for `PreToolUse`, `PostToolUse`,
`UserPromptSubmit`, `SessionStart`, `SessionEnd`, `Stop`, and `Notification`.

Gotcha: Claude Code silently drops HTTP hooks for `SessionStart` ("HTTP hooks
are not supported for SessionStart" in `claude --debug-file`). Only command-
type hooks are supported there. Everywhere else we rely on hook events to
learn the session id, so we adopt whichever id shows up on the *next* hook
that does fire (`UserPromptSubmit`, `PreToolUse`, `Stop`, `SessionEnd`, …) —
see `decideResumeUpdate` in `src/session/resume-tracker.ts`.

`oxplow__get_batch_context` returns, besides the caller's stream/thread
ids + summary, an `otherActiveBatches: Array<{ streamId, streamTitle,
threadId, batchTitle, activeBatchId }>` with one entry per peer stream —
handy when the agent suspects the "current stream" has drifted from
where it actually writes (the same phenomenon that motivated the
streamId-derivation in other MCP tools).

Each hook POSTs to the runtime's MCP server with bearer-token auth via the
env-var-interpolated `OXPLOW_HOOK_TOKEN` header, plus `X-Oxplow-Stream`,
`X-Oxplow-Thread`, `X-Oxplow-Pane`. The MCP server's `onHook` callback dispatches
to `runtime.handleHookEnvelope`, which:

1. Stores the event in `HookEventStore` (a ring buffer, also fed to the UI's
   Hook Events tool window via the `hook.recorded` EventBus event).
2. If the normalized payload carries a session id that differs from
   `thread.resume_session_id`, persists the new id so a later oxplow restart
   relaunches claude with `--resume <id>`.
3. Drives effort-anchored snapshot flushes (see "Snapshot tracking"
   below). The runtime no longer tracks per-turn rows; snapshots and
   per-effort attribution are anchored to `work_item_effort`.
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
     approve, then calls `mcp__oxplow__commit({ commit_point_id, message })`.
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
   and calls `mcp__oxplow__commit({ auto: true, threadId, message })`,
   which routes to `executeAutoCommitForThread` — runs `git commit`
   and publishes a `thread.changed`/`auto-committed` event. **No
   commit_point row is created, and no item↔sha bookkeeping happens**
   (item↔commit attribution can't be made reliable when users commit
   outside oxplow — see data-model.md for why we removed the
   `work_item_commit` junction). This is the unified-flow endpoint:
   auto and approve modes now only differ in whether the agent asks
   the user first.
   - Supplementary context tool: `mcp__oxplow__tasks_since_last_commit`
     returns work items whose efforts closed after the most recent done
     commit_point (or all closed efforts when the thread has never
     committed). The agent uses it when it's lost memory of earlier
     completed tasks; the diff is still the primary source of truth.
   - Fallback: if the agent calls `oxplow__commit` with `auto: true` but
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
4. **Writer thread with `in_progress` work items.** Block with the audit
   directive built by `buildInProgressAuditStopReason` — lists every
   `in_progress` item on the thread (id + title) and instructs the agent
   to reconcile each: still active → leave alone; acceptance criteria met
   → `complete_task` (status `human_check`, never self-mark `done`);
   stuck → `blocked`; paused → `ready`; obsolete → `canceled`. Tasks
   persist across turn boundaries; without this audit step stale
   `in_progress` rows pile up because nothing forces a settle. The audit
   takes priority over the ready-work branch — reconcile what's open
   before picking up new work.

   **No-change suppression.** The runtime keeps a per-thread fingerprint
   (`lastAuditSignatureByThread`, signature = sorted
   `id|updated_at|note_count` over the in_progress set) of the last set
   it audited. On the next Stop, if the current signature matches the
   recorded one — same items, no `update_work_item` /
   `complete_task` (which bumps `updated_at`), no `add_work_note`
   (which bumps `note_count`) — the directive is suppressed. Any
   change re-arms the audit. This stops the tight ack-loop where the
   agent answers "still in progress" → Stop fires → identical audit
   nudge → same answer, costing the user a wall of repeated lines and
   model tokens. See wi-c468e8fc093d.
5. **Writer thread with no `in_progress` and ready work.** Block with a
   terse directive built by `buildNextWorkItemStopReason` — a one-liner
   pointing at `mcp__oxplow__read_work_options` (with the embedded
   threadId) and the merged `oxplow-runtime` skill. Protocol (mark
   `in_progress` before work, `human_check` after, one-at-a-time
   attribution) lives in those skills, not the directive — keep the
   stop-hook message stable and cheap. "Queue" here means ready items
   only — `human_check` belongs to the user's review queue and is
   terminal from the pipeline's perspective.

   **Just-read suppression.** The runtime keeps a per-thread record
   (`lastReadWorkOptionsByThread`) of the ready-item ids the agent saw
   in its most recent `read_work_options` call. On the next Stop, if
   the current ready-set is identical, the directive is suppressed and
   the record is cleared — the agent already has the list; re-emitting
   it would waste a turn.

6. **Otherwise.** Allow stop.

**Subagent-in-flight carve-out.** The runtime tracks per-thread `Task`
tool calls (PreToolUse → +1, PostToolUse → -1) in
`pendingSubagentsByThread`. When the count is non-zero on a Stop, the
audit (priority 4) and ready-work (priority 5) branches are suppressed
— re-emitting them while the parent is mid-`Task` produces a visual
loop where the parent acks each Stop with "still actively being worked
by background subagent" while still waiting on the subagent. Markers
(commit/wait points) still fire: those are user-placed and represent
explicit work the agent must address. Same signal is surfaced to the
tab icon — see "Agent status" below.

### fork_thread

The runtime exposes `mcp__oxplow__fork_thread({ sourceThreadId, title,
summary, moveItemIds? })` — one transaction that:

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
`oxplow__commit` MCP handler once the user has approved in chat) is
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
   risky changes, the orchestrator calls `oxplow__read_work_options`,
   launches one `general-purpose` subagent with the brief, and
   records the summary via `add_work_note`. Subagents run in isolated
   context windows — their tokens don't count against the orchestrator,
   so main context stays flat regardless of queue depth.

The dispatch protocol (mark `in_progress` before work, `human_check`
after, never two items `in_progress` at once, blocked + note on
stuck) is identical for both modes and lives in the merged
`oxplow-runtime` skill (orchestrator side — filing + lifecycle +
dispatch combined) plus the `oxplow-subagent-work-protocol` skill
(scoped to subagents). Briefs no longer need to repeat it.

Related small fixes get batched into one task ("fix 4 test fixtures" =
one item, not four). Claude Code's built-in `TaskCreate` is a
within-turn micro-planner and never mirrors oxplow items.

`oxplow__read_work_options` (defined in `src/mcp/mcp-tools.ts`, backed by
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
`oxplow__*` tool surface:

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
- `add_followup({ threadId, note })` / `remove_followup({ threadId, id })` /
  `list_followups({ threadId })` — orchestrator-only, in-memory transient
  follow-up reminders. No DB row, lost on runtime restart. Surfaces as
  italic muted "↳ follow-up: …" lines at the top of the To Do section
  in the Work panel. Use when you defer a sub-ask mid-turn that doesn't
  warrant a full `create_work_item`. Always call `remove_followup` in
  the same turn you handle it. Never file both a follow-up and a real
  task for the same concern. NOT exposed to subagents — the dispatch
  brief deliberately omits any mention of follow-ups so subagents can't
  stash bookmarks they'll never come back to handle. See the agent
  skill at `.oxplow/runtime/claude-plugin/skills/oxplow-runtime/SKILL.md`
  for the decision rule (follow-up vs. task). Storage:
  `src/electron/followup-store.ts`; runtime publishes the bus event
  `followup.changed` so the UI re-fetches `getThreadWorkState`.
- `fork_thread({ sourceThreadId, title, summary, moveItemIds? })` — see
  "fork_thread" above. Creates a new queued
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

`buildWikiNoteMcpTools` (`src/mcp/wiki-note-mcp-tools.ts`) surfaces the
per-project wiki (`wiki_note` table + `.oxplow/notes/*.md` files — see
`data-model.md`). Tools are metadata-only: `list_notes`,
`get_note_metadata`, `resync_note`, `search_notes`, `delete_note`.
**There is intentionally no create/update tool** — the agent writes
bodies directly with its Write/Edit tools on
`.oxplow/notes/<slug>.md` (far cheaper than round-tripping full bodies
through MCP args). The notes watcher re-syncs metadata on every file
event; `resync_note` forces an immediate re-baseline when the agent
wants freshness pinned to the current HEAD without waiting for the
debounce.

## Write guard

Non-writer threads share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** `buildWriteGuardResponse`
  (`src/electron/write-guard.ts`) returns a `PreToolUse` deny for `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` from any non-`active` thread. When
  the tool's target path resolves OUTSIDE the project root AND outside
  the project's `.oxplow/`, the call is allowed (e.g. writing to
  `~/.claude/plans/foo.md`); the deny message names the specific
  absolute path. Containment checks use `isInsideWorktree` from
  `src/electron/runtime-paths.ts`, shared with the runtime's hook-path
  filter.
- **Prompt enforcement.** `NON_WRITER_PROMPT_BLOCK` (same file) is
  appended to the system prompt for non-writer threads, telling the agent
  to avoid Bash mutations too (the hook can't reliably classify shell
  commands, so the prompt is the only line of defence there).
- MCP tools (`mcp__oxplow__*`) are always allowed: they write to the state
  DB, not the worktree.

## Dev-time MCP live-reload (opt-in)

Set `OXPLOW_DEV_RELOAD=1` before launching the runtime to watch
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
every oxplow tool with full `inputSchema`; the harness picks which to
eagerly inline vs defer. There is no MCP-spec annotation and no plugin
config knob to declare a tool "always loaded". If this ever becomes
tunable, the wiring is `src/mcp/mcp-server.ts` `tools/list` response +
`src/mcp/mcp-tools.ts` tool registrations (see wi-2998dfa502da).

## Harness-injected system-reminders (not ours)

A few system-reminders come from the Claude Code harness itself, not
oxplow hooks, and are **not suppressible** from the plugin side:

- "The task tools haven't been used recently…" — harness nudge about
  `TaskCreate`/`TaskUpdate`. Noise in oxplow projects where work items
  live in `mcp__oxplow__*` tools instead. No hook, env var, or plugin
  config lets us silence it; it fires on its own schedule. If a future
  Claude Code release exposes a suppression hook, revisit wi-2a0262ae2ac2.
- The file-in-IDE reminder ("The user opened the file X in the IDE.
  This may or may not be related to the current task.") — same story,
  harness-injected on IDE focus, not a oxplow hook. Revisit if Claude
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
entirely, set `injectSessionContext: false` in `oxplow.yaml` — default is
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

## Preamble vs skill split

`buildBatchAgentPrompt` is intentionally terse — session ids, writer
flag, and a pointer to the skills. Procedural policy is consolidated in
one orchestrator-side skill (`src/session/agent-skills.ts`):
`oxplow-runtime` merges filing (when to file, how to shape items,
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

`config.agentPromptAppend` (loaded from `oxplow.yaml` via
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

**Subagent-in-flight carve-out.** The reducer counts unreturned `Task`
tool calls (PreToolUse + / PostToolUse -). When a `stop` event arrives
while the count is >0, status stays `working` instead of flipping to
`done`. Without this the tab icon would flip to "agent done" the moment
the parent paused for a subagent, even though the subagent was still
doing real work. The status flips to `done` once the final `Task`
PostToolUse returns and a subsequent `stop` lands. See wi-593a50b62e22.

## Snapshot tracking

The runtime keeps a content-addressed history of worktree files so the
UI (and future tooling) can render turn- and effort-level diffs without
relying on git. Snapshots are **time-ordered** and deduplicated by a
`version_hash` over `(path, hash, size, state)` tuples — there is no
parent chain, and two flushes of an unchanged worktree return the same
snapshot id. Mechanics:

- A per-stream in-memory **dirty set** accumulates relative paths. It
  is populated by the workspace fs-watcher (always, regardless of
  thread state) and by the PostToolUse hook's `markDirty` branch. No
  separate per-path log is kept — the dirty set is passed to
  `SnapshotStore.flushSnapshot` as an optimizer hint so only changed
  paths need restat, and every other entry carries forward from the
  previous snapshot.
- Snapshots are anchored to **efforts**, not turns. A status
  transition into `in_progress` flushes a `task-start` snapshot and
  records its id on `work_item_effort.start_snapshot_id`; closing the
  effort flushes a `task-end` snapshot recorded on
  `work_item_effort.end_snapshot_id`. Both are linked back to the
  effort via `file_snapshot.effort_id`.
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
- Effort close enforces a **5-minute minimum gap**: if the latest
  snapshot is fresher than `END_SNAPSHOT_MIN_GAP_MS`, the close path
  skips flushing a new row to avoid spamming history with
  near-identical states. The effort's `end_snapshot_id` may be left
  null in that case.
- Effort-level diffs come from
  `getSnapshotPairDiff(work_item_effort.start_snapshot_id,
  work_item_effort.end_snapshot_id, path)` and the analogous
  `getSnapshotSummary` call, exposed to the UI via
  `workItemApi.listWorkItemEfforts`.

See [data-model.md](./data-model.md) for the `file_snapshot` and
`work_item_effort` schemas, and
[ipc-and-stores.md](./ipc-and-stores.md) for the `file-snapshot.created`
EventBus event and the snapshot/effort IPC methods.

## Per-effort write log

Snapshot pair-diffs over-report when two subagents edit the same worktree
in parallel: both efforts share the same window, so each shows the
union. To attribute writes correctly the agent declares its touched
files on the status transition that closes the effort; the runtime
stores them in `work_item_effort_file` (see data-model.md).

**Agent-declared payload.** When calling `update_work_item` or
`complete_task` to close an effort, the agent passes
`touchedFiles: string[]` — the repo-relative paths it wrote or edited
during this effort. `applyStatusTransition` (in `runtime.ts`) captures
the open effort id, flushes the task-end snapshot, closes the effort,
and then inserts `work_item_effort_file` rows for each deduped path
via `INSERT OR IGNORE`. Payloads larger than `TOUCHED_FILES_CAP` (100
paths) drop all rows, so the "assume all" fallback engages in
`computeEffortFiles`. The PostToolUse hook no longer writes to
`work_item_effort_file`; the previous active-effort heuristic was
unreliable whenever ≥2 efforts were in_progress (the common case the
log is meant to cover).

Attach only fires on the `in_progress → human_check` and
`in_progress → blocked` transitions, and only when an effort is
currently open for the item. A `touchedFiles` payload on a plain
metadata update or on an already-closed item is accepted by the
schema but silently ignored — there's no effort row to attach it to.

**File-and-close shortcut.** `create_work_item` also accepts
`touchedFiles`. When the caller asks for `status: "human_check"` or
`"blocked"` AND passes `touchedFiles`, the MCP handler files the row
at `ready`, then runs `ready → in_progress → <target>` under the
covers so the normal effort-open/close path fires and attribution
lands just like a conventional close. Passing `status: "human_check"`
*without* `touchedFiles` is still legal (pure note/record row, or
agent explicitly declining attribution) — no effort is synthesized in
that case.

**Recent-human-check reminder (UserPromptSubmit).** When the agent just
closed an item to `human_check` on the thread that's submitting a new
prompt, the UserPromptSubmit hook injects a `<recent-human-check-
reminder>` block into `additionalContext` pointing at the item and
spelling out the reopen flow (`update_work_item → in_progress → redo
→ complete_task`). This fires even when the agent never touches
`create_work_item` next turn — the most reliable failure mode was the
agent investigating/reverting in-place on a correction without
recording a new effort. See `buildRecentHumanCheckReminder` in
`src/electron/runtime.ts` and the wiring in `handleHookEnvelope`'s
`UserPromptSubmit` branch. Window is 15 minutes by default.

**Redo-hint on `create_work_item`.** When the caller files a new row
on a thread that has an agent-authored `human_check` item closed
within the last 10 minutes, the response carries a `redoHint` field
pointing at that item and telling the agent to consider reopening
(`update_work_item → in_progress`) instead of filing the new task.
This is a soft nudge — the create still succeeds, because a
genuinely separate concern *should* get its own row. The heuristic
just makes the reopen path impossible to miss when the most common
trap (user rejects the last effort → agent reflexively files a
"Fix …" task) is most likely to be tripped. See
`findRecentHumanCheckItem` in `src/mcp/mcp-tools.ts`.

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

## Task lifecycle

Tasks (`work_item` rows) are the user-visible primitive. The Work
panel's in_progress bucket is driven purely by `work_item` rows —
there are no synthesized "live turn" rows, no auto-file /
auto-complete / adoption. Per-effort attribution and snapshots are
anchored to `work_item_effort`, which the runtime opens/closes on
status transitions.

Agent rules (mirrored verbatim in the project root `CLAUDE.md`):

- **Start of work** — file an `in_progress` task before editing
  project files.
- **Pivot** — before starting a different task, dispose of the
  current one: stopping for good → `canceled`; switching but coming
  back → `ready`; can't proceed → `blocked`. Then start the new task
  `in_progress`.
- **Defer/batch** — create the new task as `ready` with a short note
  capturing the ask. Flip to `in_progress` when actually picked up.
- **Merge** — update the current task's title / description /
  acceptance criteria when new info refines it. No new row.
- **Q&A** — pure conversational asks need no task. Tasks are for
  independent, completable work.
- **Persist across turns** — if a turn ends with work mid-flight
  (asked a question, Stop fired before finishing), the task stays
  `in_progress`. Only `human_check` when the work is actually shipped.

### Stop-hook directives related to tasks

The Stop-hook pipeline (see "Stop-hook pipeline" above) carries two
task-shaped branches on the writer thread:

- **Task audit (priority 4).** If any item is `in_progress`, the
  runtime emits `buildInProgressAuditStopReason` listing each
  in_progress item (id + title) and instructing the agent to
  reconcile: still active → leave alone; criteria met →
  `complete_task` (status `human_check`); stuck → `blocked`; paused
  → `ready`; obsolete → `canceled`. Audit fires *before* the
  ready-work branch — settle what's open before picking up new work.
- **Ready work (priority 5).** When nothing is `in_progress` and
  ready items exist, emit `buildNextWorkItemStopReason` pointing at
  `mcp__oxplow__read_work_options`. The just-read suppression rule
  prevents re-emitting the same ready set on consecutive Stops.

If a turn spawns real follow-up work, the agent calls
`mcp__oxplow__create_work_item` / `file_epic_with_children`.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` and the
  approved-commit-point execution loop.
