# Agent execution model


What this doc covers: how a Claude, Codex, or opencode process is
launched in a
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
   which fills the xterm with noise on Edit/Write-heavy turns. This
   covers *every* path: ingest failures and handler timeouts also ack
   `200 {}` (the agent can't act on a 4xx/5xx — it just prints the
   warning) with the cause logged server-side. The shared helper is
   `hook_ack()` in `crates/oxplow-control-plane/src/lib.rs`; never
   return a bespoke status from a hook branch.
3. MCP tool responses (when the agent calls a `oxplow__*` tool).

Auto-progression through the queue is built entirely on (2). The agent
thinks it's about to stop; the harness says "actually, do this next."

### What we can't do from oxplow hooks

Claude Code inserts its own `<system-reminder>` blocks into user
messages — for example, the periodic "The task tools haven't been used
recently; consider using TaskCreate" nudge. **Hooks can add context to
a prompt but cannot edit existing system-reminders out**, so oxplow has
no way to suppress these from the agent's view. Related asks (e.g.
"don't nag about TaskCreate while a oxplow task is in_progress")
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
  `.xterm`. Click that element to focus, type with regular keystrokes;
  xterm pipes them through the PTY to the thread's assigned agent.
- **When a turn is done.** `deriveThreadAgentStatus`
  (`crates/oxplow-domain/src/hook.rs`) reduces hook events to two states:
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
- **task lifecycle.** Create → Stop-hook picks next ready item →
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

`build_agent_command_for_session` in `crates/oxplow-app/src/agent_command.rs`
constructs a shell command for the thread's assigned `AgentKind`.
`oxplow.yaml` lists enabled agents as `agents: [...]`; the first entry
is the default for newly-created threads, and each thread persists its
own `agent` at creation time so Claude and Codex threads can run
concurrently.

- Claude runs `claude --plugin-dir <abs> --append-system-prompt <text>
  --mcp-config <json> [--resume <sid>]`.
- Codex runs `codex --cd <worktree>` or `codex resume --cd <worktree>
  <sid>`, plus CLI config overrides for oxplow MCP and lifecycle hooks.
- opencode runs `opencode -m <model> [-s <sid>]` (with a fresh-session
  fallback when the saved resume id is stale). The model is currently
  hardcoded to `github-copilot/gpt-5-mini` (`OPENCODE_MODEL` in
  `crates/oxplow-app/src/agent_command.rs`); per-project configurability
  is a filed follow-up. Hooks, MCP, and the per-thread system prompt
  all ride the `OPENCODE_CONFIG_CONTENT` env var — inline opencode
  config (merged last by opencode) wiring the oxplow MCP server
  (bearer via opencode's own `{env:OXPLOW_HOOK_TOKEN}` interpolation),
  the hook-bridge plugin, and an `instructions` entry pointing at the
  per-thread prompt file (opencode has no `--append-system-prompt`).
- All agents export `OXPLOW_STREAM_ID`, `OXPLOW_THREAD_ID`,
  `OXPLOW_HOOK_TOKEN`, and `OXPLOW_PANE` so hooks can identify
  themselves to the runtime.

The command is launched in a tmux pane via `ensureAgentPane`
(`crates/oxplow-app/src/agent_pane.rs`). Switching streams or threads doesn't kill
existing agent sessions; tmux keeps them alive in the background.

## Plugin hook bridge

Agent-specific runtime files are materialized by `oxplow-plugin` under
`.oxplow/runtime/` on every spawn. The rest of the app consumes only the
provider output (`AgentCommandOptions`) instead of branching on plugin
details.

- Claude writes `.oxplow/runtime/claude-plugin/`, passes it with
  `--plugin-dir`, and registers HTTP hooks for `PreToolUse`,
  `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`,
  `Stop`, and `Notification`.
- Codex writes `.oxplow/runtime/codex-plugin/`, packages the same
  oxplow skills in Codex plugin layout, and registers command hooks
  that POST Codex hook stdin to the same oxplow hook endpoint. Codex MCP
  is configured with CLI `--config` overrides pointing at the
  streamable-HTTP oxplow MCP endpoint.
- opencode writes `.oxplow/runtime/opencode-plugin/` —
  `plugin/oxplow-hooks.js` (an opencode JS plugin loaded via the
  `plugin` array in `OPENCODE_CONFIG_CONTENT`) plus a `prompts/` dir
  the spawn path fills with the per-thread system prompt. The JS
  bridge translates opencode plugin hooks into the same Claude-shaped
  payloads the control plane parses: `chat.message` →
  `UserPromptSubmit`, `tool.execute.before` → `PreToolUse` (a deny
  response throws inside opencode, which blocks the tool call — so
  write-guard + filing enforcement work; opencode's lowercase tool
  names and `filePath` arg are mapped to Claude's `Edit`/`Write`/… and
  `file_path`), `tool.execute.after` → `PostToolUse`, and the
  `session.idle` event → `Stop`. A blocked Stop (`{decision:"block",
  reason}`) is relayed best-effort as a fresh prompt via
  `client.session.prompt` — Stop-hook steering parity, pending live
  verification. Subagent sessions (`parentID` set) are filtered out of
  UserPromptSubmit/Stop so child activity doesn't flip the thread's
  turn lifecycle (the Claude analogue is SubagentStop handling).
  Skills + slash commands ship too: opencode only discovers SKILL.md
  from fixed locations (no config key), so `write_opencode_runtime`
  materializes the five oxplow skills into `<project>/.opencode/skills/
  <name>/` — each dir carries a `*` .gitignore so the generated files
  never land in commits. The work-next / review-comments / configure
  commands ride `OPENCODE_CONFIG_CONTENT`'s inline `command` key
  (`oxplow_plugin::opencode_command_definitions()`, frontmatter
  description + body template) as `/oxplow-work-next` etc. — opencode
  has no plugin namespacing, hence the `oxplow-` prefix instead of
  Claude's `/oxplow:` form. The launch model comes from
  `agentModels.opencode` in oxplow.yaml (falling back to the
  `OPENCODE_MODEL` const). Known gaps vs the Claude bridge: no
  SessionStart/SessionEnd/Notification events.

Gotcha: Claude Code silently drops HTTP hooks for `SessionStart` ("HTTP hooks
are not supported for SessionStart" in `claude --debug-file`). Only command-
type hooks are supported there. Everywhere else we rely on hook events to
learn the session id, so we adopt whichever id shows up on the *next* hook
that does fire (`UserPromptSubmit`, `PreToolUse`, `Stop`, `SessionEnd`, …) —
see `decideResumeUpdate` in `crates/oxplow-app/src/agent_pane.rs`.

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
   relaunches claude with `--resume <id>`. The inverse runs on
   `SessionEnd(reason=clear)`: `/clear` starts a fresh session with NO
   HTTP hook for it (SessionStart is command-type only), so until the
   new session's first prompt the token still points at the cleared
   one — a restart in that window would resurrect it. The SessionEnd
   branch (`resume_should_clear` in
   `crates/oxplow-control-plane/src/lib.rs`) blanks the token when an
   explicit clear ends exactly the session it points at; other end
   reasons (`other`, `prompt_input_exit`, …) keep it so normal
   restarts still resume.
3. Drives effort-anchored snapshot flushes (see "Snapshot tracking"
   below). The runtime no longer tracks per-turn rows; snapshots and
   per-effort attribution are anchored to `task_effort`.
4. For `PreToolUse`: returns a deny response if `buildWriteGuardResponse`
   blocks the tool (read-only thread; see Write guard below) or if
   `buildFilingEnforcementPreToolDeny` blocks it (Edit / Write /
   MultiEdit / NotebookEdit on a writer thread without an in_progress
   item; see `crates/oxplow-runtime/src/filing.rs`). Both guards bail to
   `None` for any tool outside the four worktree-mutating edits, so
   `pre_tool_check` short-circuits via `pre_tool_check_applies(tool_name)`
   *before* any DB read or git-state stat — the common case (Read / Grep
   / Bash / mcp / Task / …) does zero work here. Persistence is
   unaffected: the event is still ingested in `handle_hook_inner`
   regardless. (The HTTP round-trip itself still fires for every tool —
   the plugin's `PreToolUse` matcher is `"*"`; narrowing that matcher to
   the edit tools is a separate, sign-off-gated win.)
5. For `UserPromptSubmit`: returns `additionalContext` made up of a
   live `<session-context>` block (stream + thread + writer, rebuilt
   from the stores — see `buildSessionContextBlock` in `crates/oxplow-runtime/src/lib.rs`)
   followed by the editor-focus summary from
   `(removed under Tauri)`. The session-context block refreshes
   on every turn so the agent notices when the user promoted a
   different thread to writer mid-session; the frozen ids in the
   launch-time system prompt no longer win.
6. For `Stop`: runs `computeStopDirective` (below).

**Side-band hook steps are best-effort by design.** The PostToolUse
extras (collection observations, wiki-page attribution, the resume
tracker) are individually try/warn — a coverage parse failure must
never fail the hook or block the agent. This is deliberate policy,
not an oversight; the durable lifecycle writes they decorate are
covered by the transactional invariants in `data-model.md` instead.

**Hook handling is time-bounded.** Claude Code blocks on the hook
response, so the control plane races the whole post-auth pipeline
against a 5s timeout (`HOOK_HANDLING_TIMEOUT` /
`bounded_hook_response` in `crates/oxplow-control-plane/src/lib.rs`).
On expiry it logs a warning and returns the generic ack — tool call
allowed, no directive — so a wedged DB (e.g. the writer lock held by
a snapshot flush) can never stall the agent. Availability over
enforcement: the MCP tools re-check write-guard + filing at the call
site, so a timed-out PreToolUse deny is still caught there.

## Stop-hook pipeline

The decision logic lives in `decideStopDirective` (a pure function in
`crates/oxplow-runtime/src/stop_hook.rs`). The runtime's
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
activity (e.g. filed a task) but ended with the agent asking the
user a question still needs to stop cleanly — the Q&A short-circuit
won't fire because activity ≠ false. The agent signals this
explicitly via `mcp__oxplow__await_user({ threadId, question })`.
The runtime tracks `awaitingUserByThread`; when set, the Stop pipeline's
top branch returns "allow stop" and **suppresses every directive**
(in-progress audit, filing-enforcement). The flag is cleared on the
next UserPromptSubmit.

**Filing enforcement (writer thread, PreToolUse).** Enforcement runs
in the PreToolUse hook (`buildFilingEnforcementPreToolDeny` in
`crates/oxplow-runtime/src/filing.rs`), not the Stop hook. When the agent invokes
Edit / Write / MultiEdit / NotebookEdit on a writer thread and the
thread has no `in_progress` task, the hook returns
`permissionDecision: "deny"` and the edit is rejected before it
lands. The agent files an item at `in_progress` (or flips an existing
ready row to `in_progress`) and re-issues the edit. **A `ready`-status
filing call alone does NOT satisfy the guard** — `ready` is backlog
("noticed for later"), only `in_progress` is a commitment to ship
now. Earlier versions accepted "any filing call this turn" via a
per-thread `filedThisTurn` flag; that let the agent create a ready
row and quietly edit against it without ever transitioning. The
`hasInProgressItem` predicate is now computed live from the
task store on each PreToolUse, so a `create_task` /
`update_task` / `transition_tasks` that lands at
`in_progress` is reflected immediately. Bash is **excluded** — shell
commands routinely mutate the worktree as a side effect (`git
merge`, `git pull`, codegen, formatters) without representing
authored change worth filing. The Stop-hook in-progress audit still
fires for any lingering items, so real edits made via Bash under an
open item are unaffected.

**Plan-mode plan file is exempt** (`isPlanModePlanFile` in
`crates/oxplow-runtime/src/filing.rs`). Writes whose `tool_input.file_path` lands
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
still running. Builder lives in `crates/oxplow-runtime/src/lib.rs` next to
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
append-or-create → `mcp__oxplow__resync_wiki_page`) and notes that the
write-guard wiki carve-out applies, so capture works on read-only
threads too. Fix/feature/yes-ack prompts pay no token cost — the
builder returns `null`. The Stop hook no longer carries a
wiki-capture branch; the old directive fired post-hoc, after the
answer had already gone to chat with no durable home. The standing
WIKI CAPTURE line in `buildThreadAgentPrompt` carries the same
broadened framing — wiki ≠ codebase-only.

The pipeline runs in priority order:

1. **Writer thread with `in_progress` tasks.** Block with the audit
   directive built by `buildInProgressAuditStopReason` — lists every
   `in_progress` item on the thread (id + title) and instructs the agent
   to reconcile each: still active → leave alone; acceptance criteria met
   → `complete_task` (status `done`);
   stuck → `blocked`; paused → `ready`; obsolete → `canceled`. Tasks
   persist across turn boundaries; without this audit step stale
   `in_progress` rows pile up because nothing forces a settle.
   **No-change suppression.** The runtime keeps a per-thread fingerprint
   (`lastAuditSignatureByThread`, signature = sorted
   `id|updated_at` over the in_progress set) of the last set
   it audited. On the next Stop, if the current signature matches the
   recorded one — same items, no `update_task` /
   `complete_task` (which bumps `updated_at`) — the directive is
   suppressed. Any
   change re-arms the audit. This stops the tight ack-loop where the
   agent answers "still in progress" → Stop fires → identical audit
   nudge → same answer, costing the user a wall of repeated lines and
   model tokens. See the original ticket history.
2. **Filed-but-didn't-ship advisory.** Fires when the turn filed at
   least one new `ready` task, made zero project edits, and has
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
`read_task_options` and dispatches to a `general-purpose` subagent per
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
2. Seeds the new thread with a single `note`-kind task titled
   "Context from fork" whose description is the caller-supplied
   `summary` (no schema change — the `note` kind already exists on
   `tasks`).
3. Optionally moves each `moveItemIds` entry over via
   `taskstore.moveItemToThread`. Items must currently be `ready` or
   `blocked` on the source thread; `in_progress` / terminal items are
   rejected with an error listing the offenders so
   the caller can settle them first.
4. For each moved item, copies its last 3 notes (by `created_at DESC`,
   re-inserted in chronological order) as fresh rows on the same item
   id via `taskstore.copyLastItemNotes`. Source rows are untouched.
   Items with fewer than 3 notes copy all; items with none are no-ops.
   The user landing in the forked thread sees decisions/rationale
   carried over rather than a bare title.

Returns `{ newThreadId }`. Implementation lives on
`Services.forkThread` (`crates/oxplow-app/src/lib.rs`); the MCP tool
is just a thin surface.

## Orchestrator pattern

The thread agent is a long-lived process that must stay context-lean
across a work queue that could span dozens of items. Every file change
is filed as a task first (traceability IS the point — local
history attributes snapshots back to the sole in-progress item). Past
that, the orchestrator has two modes:

1. **Inline small-fix shortcut.** For mechanical, low-risk changes (≤
   ~20 lines across ≤ 2 files — test fixtures, import cleanup, label
   renames), the orchestrator does the Read/Edit/Bash directly under
   the task. Mark `in_progress`, edit, run tests, mark
   `done`. Snapshots still fire with correct attribution; we
   just skip the subagent round-trip.
2. **Subagent dispatch for bigger work.** For multi-file/multi-step/
   risky changes, the orchestrator calls `oxplow__read_work_options`,
   launches one `general-purpose` subagent with the brief, and
   closes the item via `complete_task` (whose `summary` lands on the
   matching `task_effort.summary` row). Subagents run in isolated
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

`oxplow__read_work_options` (defined in `crates/oxplow-mcp/src/lib.rs`, backed by
`taskstore.readWorkOptions`) returns one of three shapes:
- `{ mode: "epic", epic, children }` — the highest-priority ready item is
  an epic; all ready descendants (filtered for blocks links, transitively)
  are included as children. Dispatch the entire epic as one unit.
- `{ mode: "standalone", items }` — the head is not an epic; all ready
  non-epic items are returned with link edges inline so the agent can
  pick one or a link-related cluster. Epics are excluded from this list.
- `{ mode: "empty" }` — nothing ready; allow stop.

`read_task_options` is the dispatch unit: the agent (or the user, via
`/work-next`) calls it and dispatches the returned cluster to a
`general-purpose` subagent. The grouping (epic-as-unit vs standalone
items) lives in the tool, not the caller.

`list_ready_work` remains available for inspection but is no longer the
primary tool for queue-driven dispatch.

## MCP tools

`buildTaskMcpTools` (`crates/oxplow-mcp/src/lib.rs`) registers the agent's
tool surface. Internally each `ToolDef.name` carries an `oxplow__`
prefix (historical), but `crates/oxplow-mcp/src/lib.rs` strips that prefix at the
`tools/list` boundary via `exposedToolName` so the harness sees clean
names like `create_task`. With the harness's own `mcp__oxplow__`
namespace on top, the agent calls `mcp__oxplow__create_task` —
not the legacy `mcp__oxplow__oxplow__create_task`. The long form
still resolves on `tools/call` for back-compat.

### Surface parity with the IPC adapter

The MCP tool surface (agent) and the Tauri IPC command surface (UI,
`crates/oxplow-tauri-ipc/`) are two thin adapters over the same
`oxplow_app::Services`. They drifted silently — many user-meaningful
ops lived on IPC but not MCP. The `oxplow-surface-parity` crate
(`crates/oxplow-surface-parity/`) now guards this: a checked-in
`MANIFEST` classifies every op as `Both` / `UiOnly` / `AgentOnly` /
`AgentTodo` (the last = "should be on both, MCP tool not built yet"),
and `tests/parity.rs` enumerates the *actual* registered names on each
surface (MCP via `oxplow_mcp::registered_tool_names()`, IPC via a
capturing `tauri_specta::LanguageExt` over `specta_builder()`) and
fails if anything is unclassified, dangling, or a `Both` row is missing
a side. Names may diverge per surface (e.g. IPC `add_comment_message` ↔
MCP `respond_to_comment`), so each row carries both names.

**Consequence for new work:** adding a `#[tool]` (or a
`#[tauri::command]`) requires a `MANIFEST` row or the parity test
fails. To close an `AgentTodo` gap, build the tool and flip the row to
`Both` with `mcp: Some("…")` — the test's "every tool is classified"
check catches you if you forget. Run `cargo test -p
oxplow-surface-parity -- --nocapture` to see the current gap backlog.

Domains mirrored onto MCP so far (beyond the original task/wiki/comment
surface): **git reads** (`git_status`, `git_log`, `git_blame`,
`git_diff`, `read_file_at_ref`, `list_branches` — `stream_id` optional;
mutations stay on Bash); **snapshots / local history**
(`list_snapshots_for_stream`, `list_files_for_snapshot`, `get_snapshot`,
`get_snapshot_stats`, `list_snapshot_change_entries`,
`read_snapshot_file_content`, `restore_file_from_snapshot`);
**code quality** (`run_code_quality_scan`, `list_code_quality_scans`,
`list_code_quality_findings` — the scan orchestration is shared via
`Services::run_code_quality_scan`); **comments + lifecycle**
(`create_comment`, `set_comment_intent`, `rename_thread`,
`close_thread`, `reopen_thread`, `select_thread`, `promote_thread`,
`switch_stream`, `rename_stream`); and **site-wide search** (`search` —
BM25 over tasks/comments/notes/wiki/file-contents via the unified FTS index,
fed by the `Indexer` service; optional `stream_id` scopes file hits).
Still `AgentTodo` (see the backlog): composed snapshot DTOs, git
mutations/extra reads, `checkout_stream_branch`.

The default `kind` for `create_task` is `"task"` — omit it
The `kind` discriminator (`epic`/`task`/`subtask`/`bug`/`note`) was
removed end-to-end — `create_task` no longer accepts one and a task
row no longer carries one. An "epic" is now just any task that has
children; the bucketing is computed on read.

**Id-prefix validation at the boundary.** Every tool that takes a
string id (`thread_id`, `stream_id`, `note_id`, `followup_id`, …) calls
`expect_id_kind(tool, param, value, expected_prefix)` before
constructing the typed id. Ids are `<3-letter-prefix><int>` strings
(`str…` → stream, `thr…` → thread, `not…` → note, `fup…` → follow-up,
`eff…` → effort, `cmt…` → comment, …; see
[data-model.md](./data-model.md#entity-ids)). The check parses the value
via `oxplow_domain::AnyId` and confirms its kind matches the expected
prefix. Task and comment ids additionally accept the bare-integer form
via `parse_task_id` / `parse_comment_id` (`42` as well as `tsk42`). When
a caller passes the wrong kind — e.g. a stream id where a thread id was
expected — the tool returns an `invalid_params` error that names the
tool, the parameter, the value passed, what it looks like, and what was
expected. This converts what
would otherwise surface as an opaque downstream `FOREIGN KEY
constraint failed` into something actionable. Add the same call at
the top of any new tool handler — see `IdPrefix` and the
`ID_STREAM` / `ID_THREAD` / `ID_NOTE` / `ID_FOLLOWUP` constants in
`crates/oxplow-mcp/src/lib.rs`. Task ids are integers, so the task
parameter validator is the separate `parse_task_id` helper (digits
only, returns `Some(TaskId)` or an `invalid_params` error).

`update_task` accepts `blocked → in_progress` directly (deliberate
unblock gesture; no separate hop through `ready` required). Only
terminal states (`done`/`canceled`/`archived`) still require an
intermediate `ready` step.

- `get_batch_context`, `list_batch_work`,
  `list_ready_work`, `read_task_options`, `create_task`, `update_task`,
  `get_task`, `delete_task`, `reorder_tasks`,
  `link_tasks`, `list_recent_file_changes`,
  `dispatch_task`, `file_epic_with_children`, `complete_task`,
  `amend_effort`, `transition_tasks`
- `complete_task` returns `{ task, file_review }`. The "changed"
  set is a **content diff between the effort's start and end
  snapshots** — `SqliteSnapshotStore::diff_snapshots(start, end)`,
  which reconstructs each path's content as-of each boundary
  (latest `file_snapshot` row ≤ that snapshot) and reports a path
  only when its `blob_hash` differs. This is the shared
  `oxplow_domain::diff_trees` comparison (the same one
  `oxplow_git::diff_commits` uses), **not** raw membership of rows
  in `(start, end]` — so a no-op rewrite / edit-then-revert (equal
  hash) doesn't count as changed, and the comparison is the source
  of truth, not snapshot row timing. (`list_changed_paths_for_effort`
  still exists for an IPC view but no longer drives the review.)
  Two pieces make the underlying capture work end-to-end:
  1. `SnapshotCaptureService::request_snapshot` sleeps for
     `DEFAULT_PREDRAIN_DELAY` (300 ms) before draining the dirty
     set so the fs-watch debouncer (250 ms in `workspace_watch`)
     has time to deliver in-flight events; without that wait, an
     edit followed immediately by `complete_task` collapses the
     bracket to zero-width.
  2. There is **one `SnapshotCaptureService` per stream**
     (`SnapshotCaptureRegistry`), each watching its own worktree.
     `TaskService` resolves the right service via the task's
     thread → stream, so an effort on a worktree stream captures
     against THAT worktree's fs-watch — not the primary's.
     Without per-stream capture, edits in any non-primary
     worktree are invisible to `file_snapshot` and the bracket
     diff is always empty there. When `file_review` is non-null the bracket diff
  disagreed with the agent's declared `touched_files`:
  `claimed_but_not_changed` lists files the agent said it edited
  but the worktree didn't change;
  `changed_but_not_claimed` lists files that did change but the
  agent didn't declare — minus any path another effort already
  claimed whose snapshot window *overlaps* this effort's window
  (`paths_claimed_by_intervening_efforts`: `other.start < self.end
  AND (other.end IS NULL OR other.end > self.start)`), since that
  concurrent/sibling effort already owns it. Overlap (not "ends
  inside") is deliberate: when sibling efforts are filed in one turn
  and completed in sequence, the earliest-completed one would
  otherwise be nagged for files a later-completed sibling claims
  (whose end lands *after* this window). The latter is capped at 10
  entries (`unclaimed_overflow` carries the original count when
  truncated) so the agent isn't asked to triage a wall of paths
  from parallel efforts or formatters. `amend_effort(effort_id, add_files,
  remove_files)` is the corrective tool — adds/removes
  `task_effort_file` rows AND, for every path in `remove_files`,
  records an acknowledgement row in `effort_acknowledged_path` so
  the Stop hook's recompute treats the discrepancy as resolved.
  Re-adding a previously-disclaimed path via `add_files` clears
  its acknowledgement. Persisted authorship is always the agent's
  declared list (after any amend), never the raw diff.
- The Stop hook also surfaces unresolved file reviews as a
  one-shot directive (priority: between stale-epic-children and
  in-progress audit). MCP `complete_task` stashes the effort id in
  `ThreadRuntimeRegistry::pending_effort_reviews`; the Stop hook
  drains it via `take_pending_effort_reviews`, recomputes the diff
  fresh against the current `task_effort_file` rows, subtracts the
  paths the agent has acknowledged via `effort_acknowledged_path`,
  and fires the directive only if a discrepancy still remains. So
  a successful `amend_effort` reconciles in a single round-trip —
  the Stop hook won't re-flag the same disclaimed path on the next
  recompute. Drained = one-shot regardless.
- `dispatch_task({ threadId, itemId, extraContext?, autoStart? })` composes
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
  warrant a full `create_task`. Always call `remove_followup` in
  the same turn you handle it. Never file both a follow-up and a real
  task for the same concern. NOT exposed to subagents — the dispatch
  brief deliberately omits any mention of follow-ups so subagents can't
  stash bookmarks they'll never come back to handle. See the agent
  skill at `.oxplow/runtime/claude-plugin/skills/oxplow-runtime/SKILL.md`
  for the decision rule (follow-up vs. task). Storage:
  `crates/oxplow-app/src/followup.rs`; runtime publishes the bus event
  `followup.changed` so the UI re-fetches `getThreadWorkState`.
- `fork_thread({ sourceThreadId, title, summary, moveItemIds? })` — see
  "fork_thread" above. Creates a new queued thread on the same stream,
  seeds a note item, optionally moves ready / blocked items across in
  one transaction.
- `list_comments({ scope, id, status? })` / `respond_to_comment({
  comment_id, body })` / `resolve_comment({ comment_id })` — the user's
  threaded annotations anchored to text in pages (wiki / file / task).
  `scope` is `"thread"` (id = `b-…`) or `"stream"` (id = `s-…`, the
  whole workspace); `status` filters `"all"` / `"open"` /
  `"needs_response"`. `respond_to_comment` appends a message authored
  `"agent"` (which clears `needs_response` until the user replies
  again); `resolve_comment` marks the thread resolved. **The runtime
  never force-triggers any of this — there is no Stop-hook branch and
  no synthesized work item for comments.** The agent only touches
  comments when the user prompts it (typically via the
  `/review-comments` plugin command, which just wraps these tools).
  `comment_id` is an integer (comments use autoincrement ids). Store:
  `crates/oxplow-db/src/comment_store.rs`; mutations emit
  `CommentsChanged` on the bus.
  - **`list_comments` returns hydrated typed context, not just the
    quote.** Each row is an `EnrichedCommentThread { thread, primary,
    context_chain, referenced }`. `thread` is the raw comment + message
    history; `primary` is the comment's anchor target resolved to a
    `RefSummary { kind, id, title?, detail?, body_excerpt? }`;
    `context_chain` is the nesting of page regions the selection sat
    inside (innermost→outermost — e.g. a file row highlighted under a
    commit yields `[git-commit …]`); `referenced` are the canonical refs
    found inside the selection itself (links + inline mentions). So a
    follow-up on a commit row arrives with the commit subject + diffstat
    (primary), the dashboard it lives in (chain), and any file the quote
    linked to (referenced) — the agent gets *what the highlighted thing
    is* in one call. Hydration runs through
    `oxplow_app::ref_resolver::{resolve_ref, resolve_refs}`, which resolves
    every canonical kind: `task` → title+status, `git-commit` →
    subject+diffstat, `file` → size + head excerpt, `directory` → entry
    count + names, `wiki` → title + lead, `finding` → kind + location;
    unknown kinds return a bare `{kind,id}`. The IPC
    `list_comments_for_target` stays raw — the renderer already has the
    page, so only the MCP surface pays the resolution cost. The same
    resolver is the single source of truth for backlink labels:
    `list_backlinks`/`list_outbound`'s `source_label` is just
    `resolve_ref(...).title`.

**LSP tools** (`crates/oxplow-mcp/src/lib.rs`): `lsp_definition`,
`lsp_hover`, `lsp_references`, `lsp_diagnostics` run against the same
shared backend sessions the editor uses (`.context/lsp.md`). When no
server is configured for a language, the error is self-describing — it
names the suggested Mason package and both fix paths. The agent can fix
it itself: `lsp_install_server({ package_name })` installs from the
Mason registry (picked up immediately by editor + tools), and
`lsp_list_servers` shows what's configured/installed/running. Adding an
`lsp.servers` entry to `oxplow.yaml` is the manual alternative for
servers not in Mason.

**Unified backlinks graph (`list_backlinks` / `list_outbound`).** Every
page kind — wiki, task, file, commit, finding, directory — lives
in one persisted edge table (`page_ref`; see
[data-model.md](./data-model.md)). The two MCP tools query both
directions of any edge:

- `list_backlinks({ kind, id, limit? })` — pages pointing AT
  `(kind, id)`. Use this for cross-kind backlinks of any sort:
  "what tasks / commits / wiki pages reference src/foo.rs?",
  "who links to task:42?", "what mentions finding:fnd-1?".
- `list_outbound({ kind, id, limit? })` — what `(kind, id)` itself
  points at.

Canonical id shapes: `wiki:<slug>` uses just the slug; `task` uses
the integer rowid as a string (e.g. `"42"`); `file` uses the bare
repo-relative path; `directory` the bare path with no trailing
slash; `git-commit` the full sha; `finding` the rowid as a string.
Each row carries `ref_type` so you can tell e.g. a commit's
`touched_file` edge from a wiki body's `wikilink`.

`buildWikiPageMcpTools` (`crates/oxplow-mcp/src/lib.rs`) surfaces the
per-project wiki (`wiki_page` table + `.oxplow/wiki/*.md` files — see
`data-model.md`). Tools are metadata-only: `list_wiki_pages`,
`get_wiki_page_metadata`, `resync_wiki_page`, `search_wiki_pages` (title),
`search_wiki_page_bodies` (content), `delete_wiki_page`, and
`list_stale_wiki_pages` (pages with ≥1 file ref whose pinned snapshot is
older than the file's latest snapshot — same staleness rule as the UI
`list_wiki_freshness` reader, surfaced so the agent can find drifted
pages without reading bodies; returns `{ slug, title, stale_refs }` per
page). `list_wiki_pages` already returns the full per-page bulk fields
(title, refs, excerpt, timestamps), so the only reason to call
`get_wiki_page_metadata` after a `list` is its added `stale_refs` field
— don't fan out per-page `get` calls for data `list` already gave you.
`wiki_ref_drift({ slug, path })` closes the loop: for one stale ref it
returns the unified diff between the snapshot the ref was pinned to and
the file's current on-disk content (`compute_wiki_ref_drift` in
`crates/oxplow-app/src/wiki_drift.rs`, via `similar`), so the agent reads
only the changed hunks instead of re-opening the file. `status` is
drifted | unchanged | not_a_ref | no_pin | binary; the diff is capped
(`truncated` flags it). The wiki-only
`find_wiki_pages_for_file` was removed in favour of `list_backlinks`
(below) — every cross-kind backlinks question goes through one tool
now. **There is intentionally no create/update tool** —
the agent writes bodies directly with its Write/Edit tools on
`.oxplow/wiki/<slug>.md` (far cheaper than round-tripping full
bodies through MCP args). The notes watcher re-syncs metadata + body
on every file event; `resync_wiki_page` forces an immediate re-baseline
when the agent wants freshness pinned to the current HEAD without
waiting for the debounce.

The watcher emits `OxplowEvent::WikiPagesChanged { slug }` after
each successful resync. The slug is the file stem of the touched
`.oxplow/wiki/<slug>.md`, and the `FsWatcher` debounce is 250 ms so
bursts (editor swap-saves, batched writes) coalesce into one event.
Renderer subscribers — `WikiPageTab` in particular — filter by their
own slug and skip refreshes for unrelated wiki edits; coarse
consumers (rail HUD, title cache) ignore the slug and refetch as
before.

The `oxplow-wiki-capture` skill (the orchestrator-side skill manifest;
not yet ported into `crates/oxplow-session/`) loads when the agent
uses these tools or when the user asks an
exploration question ("how does X work", "where is X", "explain X")
or types `/note`. It carries the find-or-create flow (search by
title → body → file backlinks before creating), slug/body
conventions, and the "fold in `oxplow__get_thread_notes` from any
query subagents this turn dispatched" guidance.

**Wikilinks for file + commit references.** The skill instructs the
agent to write repo file references as `[[path/to/file.ts]]` wikilinks,
with optional `:line` suffix and `|display` override. Git commits are
written as `[[abc1234]]` (bare 7-40 char hex) or `[[git:abc1234]]` —
both resolve to the GitCommitPage. Backticks remain for code-ish
identifiers. The wiki renderer
(`apps/desktop/src/components/Notes/MarkdownView.tsx`, `preprocessWikilinks`)
rewrites `[[ ]]` into clickable links — SHA-shaped targets become
`gitcommit:` links that dispatch through `onOpenCommit`; file-shaped
targets become `file:` links that open in an editor tab via
`onOpenFile`; bare slugs route to wiki navigation. The reference
parser (in `crates/oxplow-db/src/wiki_page_store.rs`) already picks
paths out of `[[ ]]` because the bracket characters fall outside its
lookbehind,
so backlinks/freshness work without parser changes. The
`<wiki-capture-hint>` block injected on exploration UserPromptSubmits
(see "Wiki-capture is a UserPromptSubmit hint" above) auto-loads the
skill; the `/note` slash command at `.claude/commands/note.md`
triggers the same flow on demand.

## Collection command & skill

The `/oxplow:configure` command (asset `crates/oxplow-plugin/assets/configure.md`)
sets up the **collection** subsystem (see `.context/collection.md`): it has
the agent instrument the project's test tooling to emit a standard-format
coverage report at a stable path, then records the `collection:` profile in
`oxplow.yaml`. The standing `oxplow-collection` skill
(`crates/oxplow-plugin/assets/oxplow-collection.SKILL.md`) loads when a task
closes and on `/oxplow:configure`; it tells the agent to run the tests
before completing (so a report exists) and — critically — to **never parse
or report coverage numbers itself**, because oxplow parses the report
deterministically (`observed`). Both are wired in `write_plugin`
(`crates/oxplow-plugin/src/lib.rs`). The ingestion side (PostToolUse test
detector, coverage + static-analysis ride-alongs, the `ingest_coverage` /
`ingest_analysis` / `record_test_run` / `list_effort_observations` MCP tools)
is documented in `.context/collection.md`. `ingest_analysis` is the on-demand
counterpart to `ingest_coverage` for static-analysis reports (e.g.
`eslint-json`, `clippy-json`) — analysis previously had only the passive
PostToolUse path.
Report parsing is **pluggable**: those tools resolve a report's `format`
against a `CollectorRegistry` (`crates/oxplow-collect-plugin`) — the four
first-party parsers ship as bundled jaq plugins and a project can add its own
via `collection.plugins` in `oxplow.yaml`, no recompile. No new MCP tool was
added; the existing tools are now registry-backed.
When the PostToolUse hook detects a test run but no configured report was
refreshed, it returns a one-shot nudge via `hookSpecificOutput.additionalContext`
steering the agent to the report-emitting command. See the "Report-less-run
nudge" section in `.context/collection.md`.

### Nudge persistence

The PostToolUse nudges — the report-less-run nudge above and the
commit-hygiene nudge (a `git commit` that swept in files outside the open
effort's changed set) — are **persisted** as well as returned. The service
(`CollectionService::on_post_tool_use`) writes each fired nudge to the
`agent_nudge` table (`crates/oxplow-db/src/agent_nudge_store.rs`, see
`.context/data-model.md`) tagged with kind (`report-less-run` /
`commit-hygiene`), the message it surfaced, and the trigger (bash command).
This is best-effort — a persistence error is logged and swallowed, never
failing the hook. Persistence happens **after** the existing in-memory
one-shot dedup gates (per-effort for the report nudge, per-commit sha for
hygiene), so a deduped/non-fired nudge is never stored.

These are surfaced UI-side only (the agent never reads them back): a
collapsed "Agent nudges" debug sub-view on the task page (near the effort
observations) lists them, live-updating on the `agentNudgesChanged` event.
The point is a reviewer/human-facing record of "what oxplow told the agent
this effort" — previously the nudges were fully ephemeral. IPC + event wiring
is in `.context/ipc-and-stores.md` (Agent nudges).

## Write guard

Non-writer threads share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** `buildWriteGuardResponse`
  (`crates/oxplow-runtime/src/write_guard.rs`) returns a `PreToolUse` deny for `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` from any non-`active` thread. When
  the tool's target path resolves OUTSIDE the project root AND outside
  the project's `.oxplow/`, the call is allowed (e.g. writing to
  `~/.claude/plans/foo.md`); the deny message names the specific
  absolute path. Containment checks live alongside the write guard
  in `crates/oxplow-runtime/` and reuse `AppLayout` from
  `crates/oxplow-app/src/lib.rs`.
- **Wiki-notes carve-out.** Writes to `.oxplow/wiki/<slug>.md` are
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
`crates/oxplow-mcp/src/` and `crates/oxplow-db/src/` recursively. On any `.ts`/`.tsx`
change, a debounced (250ms) restart stops the current MCP server and
calls `startMcpServer` again so the rebuilt tool registrations and a
fresh TCP port + lockfile are live.

**Known limitation.** ESM caches imported modules by URL, so
re-invoking `buildTaskMcpTools` returns the *same* in-memory
module graph — an edit to handler source still needs a full runtime
restart to actually pick up new logic. The watcher still has value: it
logs the triggering file loudly so the dev knows a restart is due,
and it rebinds the port + lockfile (useful after a stale lockfile
survives a crash). Full hot-reload would require either a child-
process MCP model or a `bun --hot`-style process reload, both bigger
changes than this dev convenience warrants. Tracked on
the original ticket.

Zero runtime cost when the env var is unset; the source-root probe
doesn't run at all in that case.

## MCP tool deferral is a harness decision

Claude Code defers MCP tool schemas (surfacing them as names only until
`ToolSearch` fetches the schema) based on its own heuristics — it is
**not** a signal the MCP server sends. `tools/list` already reports
every oxplow tool with full `inputSchema`; the harness picks which to
eagerly inline vs defer. There is no MCP-spec annotation and no plugin
config knob to declare a tool "always loaded". If this ever becomes
tunable, the wiring is `crates/oxplow-mcp/src/lib.rs` `tools/list` response +
`crates/oxplow-mcp/src/lib.rs` tool registrations (see the historical task ledger).

## Harness-injected system-reminders (not ours)

A few system-reminders come from the Claude Code harness itself, not
oxplow hooks, and are **not suppressible** from the plugin side:

- "The task tools haven't been used recently…" — harness nudge about
  `TaskCreate`/`TaskUpdate`. Noise in oxplow projects where tasks
  live in `mcp__oxplow__*` tools instead. No hook, env var, or plugin
  config lets us silence it; it fires on its own schedule. If a future
  Claude Code release exposes a suppression hook, revisit the original ticket.
- The file-in-IDE reminder ("The user opened the file X in the IDE.
  This may or may not be related to the current task.") — same story,
  harness-injected on IDE focus, not a oxplow hook. Revisit if Claude
  Code adds a customization hook.

## Session-context injection

The thread id always resolves to *something* at agent-spawn time: the
Tauri commands (`open_terminal_session`, `ensure_agent_pane`) call
`ThreadService::selected_or_active(&stream_id)`, which falls back from
the user's explicit selection → the writer (active) thread → the first
queued thread. This guarantees `OXPLOW_THREAD_ID` and the
visible `<session-context>` note's thread line are populated for any
stream that has at least one thread (boot seeds a "Default" thread for
every primary stream, so this is always true in practice).

On every `UserPromptSubmit`, the runtime builds a fresh
`<session-context>` note (a short Markdown status card explaining the
current stream, worktree, branch, thread, and access role) and returns
it as `hookSpecificOutput.additionalContext` so the agent stays pointed
at the right ids mid-session. The runtime caches the last-emitted block per
agent session id (`last_context_by_session_id`) and **skips emission
when the candidate block is byte-identical to what was already sent** —
re-sending the same string is pure overhead since the agent's prompt
cache still holds the prior value. The first turn on a session, and any
turn after the block's contents change (thread flip, writer promotion,
title edit), emits normally. `SessionStart` clears the baseline so
startup, resume, clear, and compact receive one fresh note. If a project
wants to disable injection entirely, set `injectSessionContext: false`
in `oxplow.yaml` — default is `true`.

### ROLE CHANGE banner

The initial system prompt's `NON_WRITER_PROMPT_BLOCK` is frozen at
launch and replayed via cache-read on every turn, so a mid-session
writer promotion used to leave the agent acting read-only long after
the UI flipped it. To supersede the stale block in-place,
`build_session_context_block_with_role` (in
`crates/oxplow-app/src/agent_prompt.rs`) accepts an `initial_role`
input and appends a prominent `**Access changed:**` note before
`</session-context>` when the current role differs from it. The
control plane (`crates/oxplow-control-plane/src/lib.rs::RoleState`)
captures the role once per agent session id in
`initial_role_by_session_id` on the first hook it sees for that
session — UserPromptSubmit OR an ExitPlanMode PostToolUse, whichever
fires first — so the comparison baseline is stable across subsequent
turns. Both directions are covered:

- **read-only → writer.** Explains that the earlier read-only instruction
  no longer applies and task filing is still required before edits.
- **writer → read-only.** Explains that project edits are now blocked
  while wiki capture remains allowed.

No banner is emitted when the role has not changed, so steady-state
turns don't grow.

The banner reaches the agent via two complementary injection points:

1. **UserPromptSubmit.** `refreshed_session_context` builds a fresh
   `<session-context>` block (with the banner appended when the role
   has flipped) and returns it as
   `hookSpecificOutput.additionalContext`. Fires on every prompt
   when `inject_session_context: true` (default).
2. **PostToolUse(ExitPlanMode).** When the user promotes the thread
   while it's sitting on the plan-mode approval prompt, no
   UserPromptSubmit fires between "Leave plan mode" and the agent
   resuming. `role_change_banner_for` injects the banner via the
   PostToolUse `additionalContext` channel so the agent learns about
   the role flip before its next tool call.

## Preamble vs skill split

`buildBatchAgentPrompt` is intentionally terse — session ids, writer
flag, and a pointer to the skills. Procedural policy is consolidated in
one orchestrator-side skill (manifest registered alongside other
skills in the agent prompt builder; not yet a dedicated Rust module):
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
`loadProjectConfig` in `crates/oxplow-config/src/lib.rs`) is concatenated into every
agent's system prompt by `buildBatchAgentPrompt` (in `crates/oxplow-runtime/src/lib.rs`). The
Settings modal (`apps/desktop/src/components/SettingsModal.tsx`) reads/writes this
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

`deriveThreadAgentStatus` (`crates/oxplow-domain/src/hook.rs`) reduces a stream
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
returns and a subsequent `stop` lands. See the original ticket history.

**ExitPlanMode-pending carve-out.** Claude Code's built-in
`ExitPlanMode` tool fires `PreToolUse` when the agent asks the user
"should I implement this plan?", but the matching `PostToolUse` only
arrives once the user approves or rejects. Until then no `Stop` hook
fires either — the agent is genuinely waiting on the user. The
reducer counts unreturned `ExitPlanMode` calls and, if the count is
>0 at the end of replay, overrides the derived state to
`AwaitingUser` so the dot turns red instead of staying yellow. See
`agent_status_derive::derive_thread_status` in
`crates/oxplow-app/src/agent_status_derive.rs`.

**User-interrupt synthetic event.** Claude Code does not reliably fire
the `Stop` hook when the user cancels a turn with Escape (or `Ctrl-C`):
the in-flight tool's `PostToolUse` is dropped and no `Stop` lands, so
the reducer would otherwise stay `working` until the next prompt. The
runtime's `sendTerminalMessage` watches the websocket input stream and,
when it sees a bare `\x1b` or `\x03` byte (interrupt heuristic in
`terminalInputIsInterrupt`, `crates/oxplow-runtime/src/lib.rs`), ingests a synthetic
`Interrupt` meta hook event for the thread that owns the terminal
session. The reducer's `meta` branch treats `hookEventName ===
"Interrupt"` as a forced reset: status drops back to `done` and
`pendingTasks` is cleared. The synthesis only fires when the thread is
currently `working` so a user idly tapping Escape at a prompt is a
no-op. Multi-byte ESC sequences (arrow keys, etc.) are explicitly
filtered out — only the bare interrupt byte counts. See the original ticket history.

**Stall detection (API-error deaths).** Claude Code emits *no* hook at
all when a turn dies on a transient API error (socket closed
mid-stream) and the process drops back to its prompt — observed live as
a dot stuck on `working` for hours while the queue silently stalled.
Nothing event-driven can catch that, so the derivation is time-aware:
`derive_thread_status(events, now)` degrades a derived `Running` whose
newest hook event is older than `AGENT_STALL_AFTER_MS` (15 min — clears
a max-timeout 10-min Bash call with margin) to a derived-only
`AgentStatusState::Stalled` (never persisted to the agent_status
table). The ExitPlanMode `AwaitingUser` override is exempt — waiting on
the user indefinitely is legitimate. Because no hook will ever arrive
to trigger a re-derive, `AgentStallWatch`
(`crates/oxplow-app/src/agent_stall_watch.rs`, spawned from `boot.rs`)
re-derives every thread once a minute and pushes
`AgentStatusChanged { state: Stalled }` so the renderer's dot recovers
on its own. The same watchdog emits `AgentStallAlert { thread_id,
in_progress_count, waiting_ms }` — once per stall episode, re-armed
when the agent runs again or the in_progress bucket empties — whenever
a thread holds in_progress tasks but its agent has not been running
past the threshold (covers both the died-mid-turn case and "stopped
cleanly, never resumed"). The renderer collapses status as running →
`working`, stalled → `stalled` (red pulsing dot), everything else →
`waiting`, and surfaces the alert as a toast
(`useBackendSubscriptions.ts` → `formatAgentStallAlert`).

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
  records its id on `task_effort.start_snapshot_id`. Any move
  *out* of `in_progress` (done / blocked / ready / canceled /
  archived) flushes a `task-end` snapshot recorded on
  `task_effort.end_snapshot_id`, subject to the 5-minute gap
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
- On task status transitions, `handleStatusTransition` (and the
  pure `applyStatusTransition` helper it delegates to) runs. A
  transition *into* `in_progress` flushes `source: "task-start"` and
  opens a new `task_effort` row pointing at it; a transition
  *out of* `in_progress` (to `done`, `canceled`, `blocked`, etc.)
  flushes `source: "task-end"` and closes the effort.
  Re-entering `in_progress` creates a second effort — efforts are a
  per-cycle record, not a single lifetime span. A DB-level UNIQUE
  partial index on `task_effort(task_id) WHERE ended_at IS
  NULL` enforces "at most one open effort per item."
- Effort close enforces a **5-minute minimum gap**: if the latest
  snapshot is fresher than `END_SNAPSHOT_MIN_GAP_MS`, the close path
  skips flushing a new row to avoid spamming history with
  near-identical states. The effort's `end_snapshot_id` may be left
  null in that case.
- Effort-level diffs come from
  `getSnapshotPairDiff(task_effort.start_snapshot_id,
  task_effort.end_snapshot_id, path)` and the analogous
  `getSnapshotSummary` call, exposed to the UI via
  `taskApi.listTaskEfforts`.

See [data-model.md](./data-model.md) for the `file_snapshot` and
`task_effort` schemas, and
[ipc-and-stores.md](./ipc-and-stores.md) for the `file-snapshot.created`
EventBus event and the snapshot/effort IPC methods.

## Per-effort write log

Snapshot pair-diffs over-report when two subagents edit the same worktree
in parallel: both efforts share the same window, so each shows the
union. To attribute writes correctly the agent declares its touched
files on the status transition that closes the effort; the runtime
stores them in `task_effort_file` (see data-model.md).

**Claim-first auto-attribution (PostToolUse).** Every structured write
tool — `Edit` / `Write` / `MultiEdit` / `NotebookEdit` — auto-claims the
file it just wrote onto the thread's OPEN effort in real time, from the
same PostToolUse path that attributes wiki edits
(`attribute_effort_file_edit` → `effort_claim_path_from_edit` in
`crates/oxplow-control-plane/src/lib.rs`, delegating to
`TaskService::claim_open_effort_file` in `crates/oxplow-app/src/task_service.rs`).
The claim is best-effort (never fails the hook), idempotent (`record_file`
is `INSERT OR REPLACE` keyed on `(effort_id, path)`), and resolves the
open effort **per thread** via `find_open_for_thread` — so it's reliable
under the single-open-effort-per-thread rule (migration V31), unlike the
old global active-effort heuristic that was removed for over-reporting
when ≥2 efforts were in_progress. `Bash` / codegen / formatter writes are
intentionally NOT auto-claimed — they stay for snapshot reconciliation.

**Agent-declared payload (now confirm/amend).** When calling
`update_task` or `complete_task` to close an effort, the agent passes
`touchedFiles: string[]` — the repo-relative paths it wrote or edited
during this effort. Because structured edits already auto-claimed in
real time, this payload now merely confirms/amends rather than
enumerating from scratch. `applyStatusTransition` (in `crates/oxplow-runtime/src/lib.rs`) captures
the open effort id, flushes the task-end snapshot, closes the effort,
and then inserts `task_effort_file` rows for each deduped path
via `INSERT OR IGNORE`. Payloads larger than `TOUCHED_FILES_CAP` (100
paths) drop all rows, so the "assume all" fallback engages in
`computeEffortFiles`.

**Close-time reconciliation (unattributed residue).** On every
snapshot-bracketed effort close — `TaskService::update` out of
`in_progress`, so IPC `update_task`, MCP `update_task`, and the close
half of `complete_task` — `reconcile_unattributed_on_close`
(`crates/oxplow-app/src/task_service.rs`) diffs the effort's snapshot
bracket against its claims and records the `changed_but_not_claimed`
delta into `effort_unattributed_file` (see data-model.md). This is the
AUDIT layer of claim-first attribution: an out-of-band close (UI, weaker
agent) can't leave a parallel/external write looking like the agent's
authored work. Best-effort, never blocks the close; the existing
`complete_task` nudge (`compute_effort_file_review`) is unaffected
because it reads claims, not the residue table. Claiming a path later
(`record_file`) clears its residue, so the two sets never overlap.
Restart-recovery orphan closes are reconciled too: `RecoveryService`
(wired via `with_snapshot_reconcile` in `Services::new`, after the capture
registry is built) brackets each orphaned effort that has a start snapshot
— it drains the worktree (`enqueue_startup_diff`) and requests an
`EffortEnd` snapshot, since the boot worktree still reflects the dead
effort's final state, stamps it via `finish(Some(end_id), …)`, then runs
the same `reconcile_unattributed_on_close`. So a process that died
mid-effort records its unclaimed residue as unattributed rather than
silently attributing it. Best-effort and never blocks recovery: an effort
with no start snapshot (or any capture failure) keeps the legacy
`finish(None, None)` close.

Attach only fires on the `in_progress → done` and
`in_progress → blocked` transitions, and only when an effort is
currently open for the item. A `touchedFiles` payload on a plain
metadata update or on an already-closed item is accepted by the
schema but silently ignored — there's no effort row to attach it to.

**File-and-close shortcut.** `create_task` also accepts
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
the reopen flow (`update_task → in_progress → redo →
complete_task`). This fires even when the agent never touches
`create_task` next turn — the most reliable failure mode was the
agent investigating/reverting in-place on a correction without
recording a new effort. See `buildRecentDoneReminder` in
`crates/oxplow-app/src/lib.rs` and the wiring in `handleHookEnvelope`'s
`UserPromptSubmit` branch. Window is 15 minutes by default.

**Redo-hint on `create_task`.** When the caller files a new row
on a thread that has an agent-authored `done` item closed within the
last 10 minutes, the response carries a `redoHint` field pointing at
that item and telling the agent to consider reopening
(`update_task → in_progress`) instead of filing the new task.
This is a soft nudge — the create still succeeds, because a
genuinely separate concern *should* get its own row. The heuristic
just makes the reopen path impossible to miss when the most common
trap (user rejects the last effort → agent reflexively files a
"Fix …" task) is most likely to be tripped. See
`findRecentDoneItem` in `crates/oxplow-mcp/src/lib.rs`.

**1-vs-many rendering rule.** The Local History panel renders one row
per effort ending at a snapshot, *not* one row per snapshot. For a
snapshot `S`:

- 0 efforts end at S → single "External Change" / source-labelled row
  (unchanged from pre-write-log behaviour).
- 1 effort ends at S → one row labelled with the task title;
  detail pane uses `getEffortFiles(effortId)`, which short-circuits to
  the raw pair-diff.
- ≥2 efforts end at S → one row per effort, each labelled with its
  task title; detail panes call `getEffortFiles(effortId)`. If
  the effort has ≥1 `task_effort_file` row the pair-diff is
  filtered to those paths; if it has 0 rows (agent skipped the
  `touchedFiles` payload, or list exceeded the cap) we fall back to
  the raw pair-diff — better to over-report than silently show empty.

`get_effort_files` is implemented in
`crates/oxplow-tauri-ipc/src/commands/effort.rs` over the
`EffortStore` and `SnapshotStore` and wired to IPC via the same
pattern as `get_snapshot_summary`.

## Task lifecycle

Tasks (`task` rows) are the user-visible primitive. The Work
panel's in_progress bucket is driven purely by `task` rows —
there are no synthesized "live turn" rows, no auto-file /
auto-complete / adoption. Per-effort attribution and snapshots are
anchored to `task_effort`, which the runtime opens/closes on
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
calls `mcp__oxplow__create_task` /
`file_epic_with_children`.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` for the
  Files-panel commit dialog (user-driven).
