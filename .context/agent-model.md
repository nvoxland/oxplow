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
- **When a turn is done.** `deriveThreadAgentStatus`
  (`src/session/agent-status.ts`) reduces hook events to two states:
  `working` (agent is actively burning cycles) or `waiting` (agent
  isn't doing anything; user owes the next move). Brand-new threads,
  finished turns, exited processes, and permission prompts all
  collapse to `waiting`. The UI surfaces this as the colored dot on
  each thread tab — yellow pulsing for `working`, red for `waiting`.
  Poll for the transition *out* of `working` to know a turn finished.
  Looking at terminal rows alone is fragile (scrollback, progress
  indicators, partial lines).
- **Committing from a driven session.** Commits are user-driven only.
  Either run `git commit` yourself in the terminal, click commit in
  the Files panel, or tell the agent in chat "go run `git commit -m
  …`". The runtime never invokes `git commit` and there are no
  queueable commit/wait point markers. The Stop-hook does not emit
  any commit-related directives.
- **Work-item lifecycle.** Create → Stop-hook picks next ready item →
  agent marks `in_progress` → agent works → agent marks `done`
  when acceptance criteria are met. The user can reopen by flipping
  back to `in_progress`. Polling "is everything done?" treats `done`
  as terminal.

## Common pitfalls

- **Write-guard blocks Edit/Write/MultiEdit/NotebookEdit from any
  non-`active` thread.** See "Write guard" below. If the agent reports
  "permission denied" on a file write inside a non-writer thread,
  that's the hook doing its job — promote the thread to writer or
  switch to the writer thread instead.
- **Queueing work without a prompt does nothing if the agent is
  idle.** See the first-turn caveat above.
- **Runtime never commits.** The harness has no `git commit` path —
  no auto-commit at Stop, no commit-point markers, no `mcp__oxplow__commit`
  tool. Drive commits yourself via CLI / Bash / Files-panel commit.

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
   blocks the tool (read-only thread; see Write guard below) or if
   `buildFilingEnforcementPreToolDeny` blocks it (Edit / Write /
   MultiEdit / NotebookEdit on a writer thread without an in_progress
   item; see `filing-enforcement.ts`).
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
`computeStopDirective(threadId)` builds a `ThreadSnapshot` from the
live stores, calls the pure function, then applies any returned side
effects (currently only `record-audit-signature`). Keeping the decision
separate from the side effects lets every branch be unit-tested with a
fixture.

**Q&A short-circuit.** Before any branch runs, the pipeline checks
`snapshot.turnHadActivity`. The runtime tracks a per-thread flag
(`turnActivityByThread`) seeded `false` on UserPromptSubmit and
flipped `true` on the first qualifying PostToolUse — write-intent
tools (Edit/Write/Bash with non-readonly command), oxplow filing
tools, and dispatch tools (see `isActivityTool`). When the flag is
still `false` at Stop, the turn was pure Q&A — the agent answered or
asked the user something with no real work — and **every directive is
suppressed** so the agent stays stopped waiting for the user. Audit
and filing-enforcement are both skipped. `undefined` (no
UserPromptSubmit fired) is treated as "unknown → don't suppress" so
older tests / edge cases stay stable.

**Awaiting-user gate.** A turn that *did* have qualifying tool
activity (e.g. filed a work item) but ended with the agent asking the
user a question still needs to stop cleanly — the Q&A short-circuit
won't fire because activity ≠ false. The agent signals this
explicitly via `mcp__oxplow__await_user({ threadId, question })`.
The runtime tracks `awaitingUserByThread`; when set, the Stop pipeline's
top branch returns "allow stop" and **suppresses every directive**
(in-progress audit, filing-enforcement). The flag is cleared on the
next UserPromptSubmit.

**Filing enforcement (writer thread, PreToolUse).** Enforcement runs
in the PreToolUse hook (`buildFilingEnforcementPreToolDeny` in
`filing-enforcement.ts`), not the Stop hook. When the agent invokes
Edit / Write / MultiEdit / NotebookEdit on a writer thread and the
thread has no `in_progress` work item, the hook returns
`permissionDecision: "deny"` and the edit is rejected before it
lands. The agent files an item at `in_progress` (or flips an existing
ready row to `in_progress`) and re-issues the edit. **A `ready`-status
filing call alone does NOT satisfy the guard** — `ready` is backlog
("noticed for later"), only `in_progress` is a commitment to ship
now. Earlier versions accepted "any filing call this turn" via a
per-thread `filedThisTurn` flag; that let the agent create a ready
row and quietly edit against it without ever transitioning. The
`hasInProgressItem` predicate is now computed live from the
work-item store on each PreToolUse, so a `create_work_item` /
`update_work_item` / `transition_work_items` that lands at
`in_progress` is reflected immediately. Bash is **excluded** — shell
commands routinely mutate the worktree as a side effect (`git
merge`, `git pull`, codegen, formatters) without representing
authored change worth filing. The Stop-hook in-progress audit still
fires for any lingering items, so real edits made via Bash under an
open item are unaffected.

**Plan-mode plan file is exempt** (`isPlanModePlanFile` in
`filing-enforcement.ts`). Writes whose `tool_input.file_path` lands
under `$HOME/.claude/plans/<slug>.md` skip the filing guard — that
file is owned by the harness's plan workflow, not project work, and
plan mode denies every other tool while it's on, so blocking the
plan-file write would dead-lock the workflow. The carve-out is
narrow: only paths under `.claude/plans/` ending in `.md`.

**Mid-turn-prompt reminder (UserPromptSubmit).** When a new
`UserPromptSubmit` arrives on the writer thread and the thread
already has any `in_progress` item from a prior prompt, the runtime
injects a `<prior-prompt-in-progress-reminder>` block into
`additionalContext` via `buildPriorPromptInProgressReminder`. It
names the open item and tells the agent to either file a new row
(separate concern) or explicitly reopen the existing one (fix/redo)
— so multi-prompt turns don't quietly pile new asks into whichever
item was already open. Pairs with the recent-done reminder: that one
fires when the prior item already closed, this one fires when it's
still running. Builder lives in `runtime.ts` next to
`buildRecentDoneReminder`.

**Ready-match nudge (UserPromptSubmit).** Sibling of the prior-prompt
reminder, but for `ready` rows. `buildReadyMatchReminder(items,
promptText)` tokenizes the prompt and each ready item's title +
description into lowercase alphanumeric runs ≥ 4 chars (excluding a
small stop-word list), scores intersection size, and emits a
`<ready-item-match-reminder>` block iff exactly one ready item has
≥ 2 shared tokens AND no other ready item is within 1 of its score.
Catches the failure mode where the agent files a fresh task that
duplicates a ready row already on the board, instead of flipping the
existing row to in_progress. Conservative — silent on ambiguity, since
the safer default is "file a new row" if the agent isn't confident
the prompt is the same concern.

**Wiki-capture is a UserPromptSubmit hint, not a Stop directive.**
The wiki is for any non-trivial exploratory Q&A — codebase
walkthroughs AND general synthesis (design rationale, comparisons,
tradeoffs, recommendations, advice). Two regex families in
`buildWikiCaptureHint(prompt)` cover both: a codebase pattern (matches
"how does", "explain", "trace", "describe", "walk me through", "give
me an overview", "high-level architecture", "summarize the codebase",
etc.) and a general-synthesis pattern (matches "why does/did/should",
"what's the difference", "compare X to Y", "tradeoffs", "pros and
cons", "should I", "best way", "is it better", "advice on",
"recommend", "rationale behind"). Either match injects a
`<wiki-capture-hint>` block into `additionalContext`. The hint points
the agent at the `oxplow-wiki-capture` skill (search existing notes →
append-or-create → `mcp__oxplow__resync_note`) and notes that the
write-guard wiki carve-out applies, so capture works on read-only
threads too. Fix/feature/yes-ack prompts pay no token cost — the
builder returns `null`. The Stop hook no longer carries a
wiki-capture branch; the old directive fired post-hoc, after the
answer had already gone to chat with no durable home. The standing
WIKI CAPTURE line in `buildThreadAgentPrompt` carries the same
broadened framing — wiki ≠ codebase-only.

The pipeline runs in priority order:

1. **Writer thread with `in_progress` work items.** Block with the audit
   directive built by `buildInProgressAuditStopReason` — lists every
   `in_progress` item on the thread (id + title) and instructs the agent
   to reconcile each: still active → leave alone; acceptance criteria met
   → `complete_task` (status `done`);
   stuck → `blocked`; paused → `ready`; obsolete → `canceled`. Tasks
   persist across turn boundaries; without this audit step stale
   `in_progress` rows pile up because nothing forces a settle.
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
2. **Filed-but-didn't-ship advisory.** Fires when the turn filed at
   least one new `ready` work item, made zero project edits, and has
   nothing `in_progress` — the "user said do X, agent logged it as
   backlog and stopped" misread. Same dedup pattern as the audit
   branch: a per-thread `filedButDidntShipFiredByThread` flag is set
   by a `record-filed-but-didnt-ship-fired` side effect after the
   first fire, suppressing re-emission on subsequent Stops within the
   same prompt gap. Cleared on UserPromptSubmit alongside the other
   per-turn filing flags. Without dedup the advisory loops forever
   because its triggering condition (ready item filed, no edits) is a
   property of accumulated turn state and never changes between Stop
   acks.
3. **Otherwise.** Allow stop.

**No commit / wait-point branches.** The runtime never drives `git
commit` and there are no queueable commit / wait-point markers. Commits
are user-driven (CLI / Bash / Files-panel commit). The pipeline never
emits commit-shaped directives.

**Cross-turn queue progression is user-driven.** There is intentionally
no Stop-hook directive that pushes the agent onto the next ready work
item. When the agent finishes its current obligations and Stops, it
stops — the user resumes queue work by typing a prompt or running the
plugin-emitted `/work-next` slash command (which calls
`read_work_options` and dispatches to a `general-purpose` subagent per
the `oxplow-runtime` skill).

**Subagent-in-flight carve-out.** The runtime tracks per-thread `Task`
tool calls (PreToolUse → +1, PostToolUse → -1) in
`pendingSubagentsByThread`. When the count is non-zero on a Stop, the
audit branch is suppressed — re-emitting it while the parent is
mid-`Task` produces a visual loop where the parent acks each Stop with
"still actively being worked by background subagent" while still
waiting on the subagent.

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
   `blocked` on the source thread; `in_progress` / terminal items are
   rejected with an error listing the offenders so
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
   `done`. Snapshots still fire with correct attribution; we
   just skip the subagent round-trip.
2. **Subagent dispatch for bigger work.** For multi-file/multi-step/
   risky changes, the orchestrator calls `oxplow__read_work_options`,
   launches one `general-purpose` subagent with the brief, and
   records the summary via `add_work_note`. Subagents run in isolated
   context windows — their tokens don't count against the orchestrator,
   so main context stays flat regardless of queue depth.

The dispatch protocol (mark `in_progress` before work, `done`
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

`read_work_options` is the dispatch unit: the agent (or the user, via
`/work-next`) calls it and dispatches the returned cluster to a
`general-purpose` subagent. The grouping (epic-as-unit vs standalone
items) lives in the tool, not the caller.

`list_ready_work` remains available for inspection but is no longer the
primary tool for queue-driven dispatch.

## MCP tools

`buildWorkItemMcpTools` (`src/mcp/mcp-tools.ts`) registers the agent's
tool surface. Internally each `ToolDef.name` carries an `oxplow__`
prefix (historical), but `mcp-server.ts` strips that prefix at the
`tools/list` boundary via `exposedToolName` so the harness sees clean
names like `create_work_item`. With the harness's own `mcp__oxplow__`
namespace on top, the agent calls `mcp__oxplow__create_work_item` —
not the legacy `mcp__oxplow__oxplow__create_work_item`. The long form
still resolves on `tools/call` for back-compat.

The default `kind` for `create_work_item` is `"task"` — omit it
unless you specifically need an epic/subtask/bug/note. Forcing the
field on every call produced a guaranteed first-call failure for
trivial fixes.

`update_work_item` accepts `blocked → in_progress` directly (deliberate
unblock gesture; no separate hop through `ready` required). Only
terminal states (`done`/`canceled`/`archived`) still require an
intermediate `ready` step.

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
- `get_subsystem_doc({ threadId, name })` — returns
  `{ name, path, content, exists }` for `.context/<name>.md` in the
  thread's stream worktree. Cheap alternative to `Read` when you only
  need the doc body — saves the model from re-reading the same
  `.context/` doc 20+ times per session and never hard-errors on a
  missing doc (returns `exists: false` instead). Path-traversal
  characters in `name` are rejected.
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
  "fork_thread" above. Creates a new queued thread on the same stream,
  seeds a note item, optionally moves ready / blocked items across in
  one transaction.

`buildLspMcpTools` (`src/mcp/lsp-mcp-tools.ts`) adds language-server
queries (definition, references, hover) the agent can use without
shelling out.

`buildWikiNoteMcpTools` (`src/mcp/wiki-note-mcp-tools.ts`) surfaces the
per-project wiki (`wiki_note` table + `.oxplow/notes/*.md` files — see
`data-model.md`). Tools are metadata-only: `list_notes`,
`get_note_metadata`, `resync_note`, `search_notes` (title),
`search_note_bodies` (content), `find_notes_for_file` (backlinks),
`delete_note`. **There is intentionally no create/update tool** —
the agent writes bodies directly with its Write/Edit tools on
`.oxplow/notes/<slug>.md` (far cheaper than round-tripping full
bodies through MCP args). The notes watcher re-syncs metadata + body
on every file event; `resync_note` forces an immediate re-baseline
when the agent wants freshness pinned to the current HEAD without
waiting for the debounce.

The `oxplow-wiki-capture` skill (`src/session/wiki-capture-skill.ts`)
loads when the agent uses these tools or when the user asks an
exploration question ("how does X work", "where is X", "explain X")
or types `/note`. It carries the find-or-create flow (search by
title → body → file backlinks before creating), slug/body
conventions, and the "fold in `oxplow__get_thread_notes` from any
query subagents this turn dispatched" guidance. The
`<wiki-capture-hint>` block injected on exploration UserPromptSubmits
(see "Wiki-capture is a UserPromptSubmit hint" above) auto-loads the
skill; the `/note` slash command at `.claude/commands/note.md`
triggers the same flow on demand.

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
- **Wiki-notes carve-out.** Writes to `.oxplow/notes/<slug>.md` are
  allowed even on non-writer threads — the per-project wiki is not
  committed to git and doesn't collide with the writer's in-progress
  code, so capture is safe from any thread. Other `.oxplow/` paths
  (`state.sqlite`, `snapshots/`, `runtime/`) stay blocked.
- **Prompt enforcement.** `NON_WRITER_PROMPT_BLOCK` (same file) is
  appended to the system prompt for non-writer threads, telling the agent
  to avoid Bash mutations too (the hook can't reliably classify shell
  commands, so the prompt is the only line of defence there). The
  block also documents the wiki-notes carve-out so the agent knows it
  CAN capture exploration findings via Write.
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

`deriveThreadAgentStatus` (`src/session/agent-status.ts`) reduces a stream
of hook events into one of two states: `working` or `waiting`. The
runtime recomputes on every hook arrival and emits
`agent-status.changed`. The UI shows it as a colored dot on each thread
tab — yellow pulsing for `working`, red for `waiting`. The two states
encode the only signal a tab indicator actually needs: is the agent
burning cycles, or does the user owe the next move? Brand-new threads,
finished turns (`stop`), exited processes (`session-end`), permission
prompts (`notification`), and user interrupts all collapse to `waiting`.

**Subagent-in-flight carve-out.** The reducer counts unreturned `Task`
tool calls (PreToolUse + / PostToolUse -). When a `stop` event arrives
while the count is >0, status stays `working` instead of flipping to
`waiting`. Without this the tab icon would flip the moment the parent
paused for a subagent, even though the subagent was still doing real
work. The status flips to `waiting` once the final `Task` PostToolUse
returns and a subsequent `stop` lands. See wi-593a50b62e22.

**User-interrupt synthetic event.** Claude Code does not reliably fire
the `Stop` hook when the user cancels a turn with Escape (or `Ctrl-C`):
the in-flight tool's `PostToolUse` is dropped and no `Stop` lands, so
the reducer would otherwise stay `working` until the next prompt. The
runtime's `sendTerminalMessage` watches the websocket input stream and,
when it sees a bare `\x1b` or `\x03` byte (interrupt heuristic in
`terminalInputIsInterrupt`, runtime.ts), ingests a synthetic
`Interrupt` meta hook event for the thread that owns the terminal
session. The reducer's `meta` branch treats `hookEventName ===
"Interrupt"` as a forced reset: status drops back to `done` and
`pendingTasks` is cleared. The synthesis only fires when the thread is
currently `working` so a user idly tapping Escape at a prompt is a
no-op. Multi-byte ESC sequences (arrow keys, etc.) are explicitly
filtered out — only the bare interrupt byte counts. See wi-53c5f6e407fc.

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
  records its id on `work_item_effort.start_snapshot_id`. Any move
  *out* of `in_progress` (done / blocked / ready / canceled /
  archived) flushes a `task-end` snapshot recorded on
  `work_item_effort.end_snapshot_id`, subject to the 5-minute gap
  rule: when the stream's most recent snapshot is younger than
  `END_SNAPSHOT_MIN_GAP_MS`, the flush is skipped and
  `end_snapshot_id` is left null. Both task-start and task-end are
  linked back to the effort via `file_snapshot.effort_id`. The flush
  is automatic inside `applyStatusTransition` (which the MCP work-
  item tools all delegate to) — agents never need to flush
  explicitly.
- On project open, `takeStartupSnapshot` runs once per stream — a full
  worktree walk that emits `source: "startup"`. If nothing changed
  while the app was down, `version_hash` dedup returns the existing
  snapshot and no new row is written; otherwise a fresh one is
  recorded so the "changes during downtime" are visible.
- On work-item status transitions, `handleStatusTransition` (and the
  pure `applyStatusTransition` helper it delegates to) runs. A
  transition *into* `in_progress` flushes `source: "task-start"` and
  opens a new `work_item_effort` row pointing at it; a transition
  *out of* `in_progress` (to `done`, `canceled`, `blocked`, etc.)
  flushes `source: "task-end"` and closes the effort.
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

Attach only fires on the `in_progress → done` and
`in_progress → blocked` transitions, and only when an effort is
currently open for the item. A `touchedFiles` payload on a plain
metadata update or on an already-closed item is accepted by the
schema but silently ignored — there's no effort row to attach it to.

**File-and-close shortcut.** `create_work_item` also accepts
`touchedFiles`. When the caller asks for `status: "done"` or
`"blocked"` AND passes `touchedFiles`, the MCP handler files the row
at `ready`, then runs `ready → in_progress → <target>` under the
covers so the normal effort-open/close path fires and attribution
lands just like a conventional close. Passing `status: "done"`
*without* `touchedFiles` is still legal (pure note/record row, or
agent explicitly declining attribution) — no effort is synthesized in
that case.

**Recent-done reminder (UserPromptSubmit).** When the agent just
closed an item to `done` on the thread that's submitting a new
prompt, the UserPromptSubmit hook injects a `<recent-done-reminder>`
block into `additionalContext` pointing at the item and spelling out
the reopen flow (`update_work_item → in_progress → redo →
complete_task`). This fires even when the agent never touches
`create_work_item` next turn — the most reliable failure mode was the
agent investigating/reverting in-place on a correction without
recording a new effort. See `buildRecentDoneReminder` in
`src/electron/runtime.ts` and the wiring in `handleHookEnvelope`'s
`UserPromptSubmit` branch. Window is 15 minutes by default.

**Redo-hint on `create_work_item`.** When the caller files a new row
on a thread that has an agent-authored `done` item closed within the
last 10 minutes, the response carries a `redoHint` field pointing at
that item and telling the agent to consider reopening
(`update_work_item → in_progress`) instead of filing the new task.
This is a soft nudge — the create still succeeds, because a
genuinely separate concern *should* get its own row. The heuristic
just makes the reopen path impossible to miss when the most common
trap (user rejects the last effort → agent reflexively files a
"Fix …" task) is most likely to be tripped. See
`findRecentDoneItem` in `src/mcp/mcp-tools.ts`.

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
  `in_progress`. Only `done` when the work is actually shipped.

### Stop-hook directives related to tasks

The Stop-hook pipeline (see "Stop-hook pipeline" above) carries one
task-shaped branch on the writer thread:

- **Task audit (priority 4).** If any item is `in_progress`, the
  runtime emits `buildInProgressAuditStopReason` listing each
  in_progress item (id + title) and instructing the agent to
  reconcile: still active → leave alone; criteria met →
  `complete_task` (status `done`); stuck → `blocked`; paused
  → `ready`; obsolete → `canceled`.

There is intentionally no ready-work branch — cross-turn queue
progression is user-driven (a plain prompt, or `/work-next` shipped
via the plugin). If a turn spawns real follow-up work, the agent
calls `mcp__oxplow__create_work_item` /
`file_epic_with_children`.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` for the
  Files-panel commit dialog (user-driven).
