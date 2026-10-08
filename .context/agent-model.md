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
2. Hook responses returned to Claude over HTTP: `additionalContext` on
   the prompt and post-tool hooks. A Stop is never refused
   ([work-tracking.md](./work-tracking.md) "No gates"). The default no-op response is
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

### No synthesized agent terminal input (no automation)

**The agent's terminal input has exactly one source: human keystrokes /
paste via the UI.** oxplow must NEVER programmatically generate, inject,
or auto-respond with agent terminal input. The agent makes its own tool
calls and is always driven by a human — never by oxplow typing at it.
Concretely:

- The renderer's xterm (`TerminalPane.tsx`) pipes the user's own
  keystrokes / paste / scroll / resize to the PTY via the
  `forward_terminal_input` IPC/RPC command
  (→ `terminal_sessions.send` → `pty.write`). That command is the
  human-input transport, **not** an agent-messaging API — it's named
  `forward_terminal_input` (not the old `send_terminal_message`)
  precisely so it can't be mistaken for one. It must stay: in
  remote/daemon mode the PTY is server-side and browser keystrokes can
  only reach it through this client→server call, so removing it makes
  the remote terminal read-only.
- `forward_terminal_input` is **UI-only** — it is never on the MCP
  (agent) surface, so the agent cannot call it to type at itself or a
  peer. Enforced by `oxplow-surface-parity`'s
  `ui("forward_terminal_input")` row.
- The drag/drop + "add to context" path (`agent-input-bus.ts`) is also
  human-initiated: a user gesture publishes text the visible
  `TerminalPane` pastes. oxplow synthesizes nothing on its own.
- Steering (nudges, `<session-context>`)
  reaches the agent through **hook responses** that the agent's OWN
  harness injects (the invariant above), never by oxplow writing to the
  terminal.

**Why it matters.** Synthesizing `{type:"input"}` to "type at" the agent
would be automating the agent CLI, which risks violating its
(Claude Code / Codex / opencode) license/ToS.

**Guards** (fail the build if violated):
- Frontend source-scan
  (`apps/desktop/src/no-agent-input-automation.test.ts`): confines
  `forwardTerminalInput` calls and `{type:"input"}` message construction
  to the human-input files (`TerminalPane.tsx`, its ordered sender
  `terminalInput.ts` — used by the pane alone — the `api.ts` facade,
  generated bindings).
- Rust source-scan (in
  `crates/oxplow-rpc/src/commands/terminal.rs` tests): asserts the only
  production caller of `terminal_sessions.send(` is the
  `forward_terminal_input` command core.

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
fresh project, or after a `Stop`), creating a work
item does **not** kick it off. Someone — a human, or a harness typing
into the xterm — has to send the first `UserPromptSubmit`.

## Driving from automation

Everything a test harness (or another agent) needs to drive an inner
oxplow agent:

- **Where the agent runs.** Each thread's terminal agent runs directly in
  a PTY (there is no terminal multiplexer, tsk1018), rendered in the
  first center-area tab. The renderer is `TerminalPane` attached to the
  `working` pane target; UI-side, it's an xterm.js inside
  `.xterm`. Click that element to focus, type with regular keystrokes;
  xterm pipes them through the PTY to the thread's assigned agent.
- **When a turn is done.** `derive_session_status`
  (`crates/oxplow-app/src/agent_status_derive.rs`) reduces the thread's
  logged `agent.*` activity to two states:
  `working` (agent is actively burning cycles) or `waiting` (agent
  isn't doing anything; user owes the next move). Brand-new threads,
  finished turns, exited processes, and permission prompts all
  collapse to `waiting`. The UI surfaces this as the colored dot on
  each thread's glyph in the Navigator and on the agent center tab
  (`AgentStatusDot`) — yellow pulsing for `working`, neutral grey for
  `waiting` (idle, ready for the next prompt; it was pale red, which read
  as an error) (plus red `stalled` and blue `awaiting` when the thread
  waits on the person).
  Poll for the transition *out* of `working` to know a turn finished.
  Looking at terminal rows alone is fragile (scrollback, progress
  indicators, partial lines).
- **Committing from a driven session.** Commits are user-driven only.
  Either run `git commit` yourself in the terminal, click commit in
  the Files panel, or tell the agent in chat "go run `git commit -m
  …`". The runtime never invokes `git commit` and there are no
  queueable commit/wait point markers.
- **task lifecycle.** Create →
  agent marks `in_progress` → agent works → agent marks `done`
  when the work is complete. The user can reopen by flipping
  back to `in_progress`. Polling "is everything done?" treats `done`
  as terminal.

## Common pitfalls

- **Write-guard blocks Edit/Write/MultiEdit/NotebookEdit from any
  non-`active` thread.** See "Write guard" below. If the agent reports
  "permission denied" on a file write inside a non-writer thread,
  that's the hook doing its job — promote the thread to writer
  (`oxplow.thread.promote`, which a person or the agent itself may run;
  it queues the current writer) or switch to the writer thread instead.
- **Queueing work without a prompt does nothing if the agent is
  idle.** See the first-turn caveat above.
- **Runtime never commits.** The harness has no `git commit` path —
  no auto-commit at Stop, no commit-point markers, no `mcp__oxplow__commit`
  tool. Drive commits yourself via CLI / Bash / Files-panel commit.

## Launching the agent

`build_agent_command_for_session` in `crates/oxplow-app/src/agent_command.rs`
constructs a shell command for the thread's assigned `AgentKind`.

**What an agent inherits (tsk1032).** Every agent and terminal (PTY and
ACP) is spawned without `agent_path::NOT_INHERITED`: Claude Code's session
markers (`CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, …) and an oxplow agent's
identity (`OXPLOW_HOOK_TOKEN`, `OXPLOW_THREAD_ID`, …). Otherwise oxplow run
from inside an agent's terminal starts child sessions — Claude Code turns
transcript saving off, breaking resume and token counts — whose hooks point
at the outer oxplow. It's a list, not a prefix: `CLAUDE_CONFIG_DIR`,
`CLAUDE_CODE_USE_BEDROCK` and `OXPLOW_HOME` are the person's configuration
and pass. Oxplow's own `OXPLOW_*` for the agent ride its command.
Project configuration is changed through the `config.*` commands on the
command bus ([commands.md](./commands.md)) — an agent sets `zones`,
`metricRetentionDays`, `generated`, … with `oxplow.config.set`, while the keys
that run a program or pick the model (`agents`, `lsp`, `collection`,
`ai`, `acpAgents`, `agentModels`, `extensions`, `agentPromptAppend`, …)
need a person's confirmation: the agent's `oxplow.config.set` is kept as a
proposal (`proposal:N`, with the before/after) that the person approves
or declines, and `run_command` tells the agent so ([commands.md](./commands.md),
"Proposals"). `set_zones` is gone; `zones` is just a key.

`.oxplow/project.yaml` lists enabled agents as `agents: [...]`; the first entry
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
  `OXPLOW_HOOK_TOKEN`, and `OXPLOW_SESSION` (the agent session, `ses<n>`)
  so hooks can identify themselves to the runtime. The OTLP exporters
  carry the session too (`X-Oxplow-Session` beside `X-Oxplow-Thread`),
  and so do the Claude and ACP MCP connections (nothing reads it there
  yet).

The command runs as `sh -lc <command>` in a PTY
(`oxplow_rpc::commands::terminal::open_terminal_session`, keyed by the
agent session — the pane target is its id, `ses3` — so a re-attach resumes
the live process and two sessions in a thread are two processes). Switching
streams or threads doesn't kill existing agent sessions: the daemon keeps
them, and a re-attach replays their buffer. They end with the daemon; the
next open resumes the agent's own session (`--resume`). There is no tmux
mode (tsk1018): it went with the "Open in tmux" toggle and the
`oxplow-tmux` crate.

### The agent is spawned by absolute path, on purpose (tsk245)

`AgentCommandOptions::program` carries the resolved absolute path to the CLI,
from `agent_path::resolve_agent_program`. **Don't "simplify" it back to the bare
binary name** — that is a bug that only reproduces on a GUI launch:

- A **GUI-launched** app (Finder, dock, oxplow's own launcher) gets macOS's
  minimal PATH, not the user's shell PATH. A terminal-launched app inherits the
  shell's, and the PTY child inherits it in turn — which is the *only* reason
  the bare name ever worked.
- The PTY runs `sh -lc`, and `sh` is bash-in-sh-mode: even as a **login** shell
  it reads `/etc/profile` and `~/.profile`, never `~/.zshrc` / `~/.zprofile`.
  So `-l` does not recover a zsh user's PATH.

Together those meant launching from the launcher killed every agent pane with
`sh: claude: command not found` — with Claude Code's default install location
(`~/.local/bin`) sitting exactly in the gap, and the message landing right under
the stale-resume notice so it read as a session problem.

`agent_path` probes PATH first (so a deliberate PATH entry always wins), then a
fixed list of dirs agent CLIs install into. When it resolves nothing the command
keeps the bare name — resolution is a heuristic and the shell may still win —
but prefixes a `command -v` preflight that reports the real cause instead of the
shell's bare `command not found`.

`agent_path::base_pty_env` is the single source of the env every PTY spawns
with (`TERM`, `COLORTERM`, and the widened `PATH`). All five spawn sites call
it; adding a sixth that hand-rolls the env re-opens this bug for the *tools the
agent shells out to*, which the absolute path doesn't cover.

Known limit: version-manager shims (mise, nvm, volta) live under versioned
directories that no fixed list can guess, so a GUI-launched agent can still miss
`node`/`bun`. Fixing that needs a login-shell env capture (`$SHELL -ilc`), which
was deliberately not taken here — it costs a subprocess per launch and can hang
on a user's rc file.

## Plugin hook bridge

Agent-specific runtime files are materialized by `oxplow-plugin` under
`.oxplow/runtime/` on every spawn. The rest of the app consumes only the
provider output (`AgentCommandOptions`) instead of branching on plugin
details.

- Claude writes `.oxplow/runtime/claude-plugin/`, passes it with
  `--plugin-dir`, and registers HTTP hooks for `PreToolUse`,
  `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`,
  `Stop`, and `Notification`, plus events it only observes
  (`SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`,
  `PreCompact`), acked unread. `OXPLOW_HOOK_DEBUG=<file>` appends every
  hook payload as sent, one JSON line each, to learn real payload shapes
  ([work-tracking.md](./work-tracking.md) "The record").
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
  response throws inside opencode, which blocks the tool call — so the
  write guard works; opencode's lowercase tool
  names and `filePath` arg are mapped to Claude's `Edit`/`Write`/… and
  `file_path`), `tool.execute.after` → `PostToolUse`, and the
  `session.idle` event → `Stop`. Subagent sessions (`parentID` set) are filtered out of
  UserPromptSubmit/Stop so child activity doesn't flip the thread's
  turn lifecycle.
  Skills + slash commands ship too: opencode only discovers SKILL.md
  from fixed locations (no config key), so `write_opencode_runtime`
  materializes the offered skills (`capabilities::agent_text`) into
  `<project>/.opencode/skills/<name>/` — each dir carries a `*`
  .gitignore so the generated files never land in commits. The offered
  commands ride `OPENCODE_CONFIG_CONTENT`'s inline `command` key
  (`oxplow_plugin::opencode_command_definitions(&text)`, frontmatter
  description + body template) as `/oxplow-review-comments` etc. — opencode
  has no plugin namespacing, hence the `oxplow-` prefix instead of
  Claude's `/oxplow:` form. The launch model comes from
  `agentModels.opencode` in .oxplow/project.yaml (falling back to the
  `OPENCODE_MODEL` const). Known gaps vs the Claude bridge: no
  SessionStart/SessionEnd/Notification events.

Gotcha: Claude Code silently drops HTTP hooks for `SessionStart` ("HTTP hooks
are not supported for SessionStart" in `claude --debug-file`). Only command-
type hooks are supported there. Everywhere else we rely on hook events to
learn the session id, so we adopt whichever id shows up on the *next* hook
that does fire (`UserPromptSubmit`, `PreToolUse`, `Stop`, `SessionEnd`, …) —
see the resume handling in `crates/oxplow-app/src/hook_ingest.rs`.

`oxplow__get_batch_context` returns, besides the caller's stream/thread
ids + summary, an `otherActiveBatches: Array<{ streamId, streamTitle,
threadId, batchTitle, activeBatchId }>` with one entry per peer stream —
handy when the agent suspects the "current stream" has drifted from
where it actually writes (the same phenomenon that motivated the
streamId-derivation in other MCP tools).

**Token usage — OTEL, not the Stop hook (tsk22).** The control plane hosts a
sibling `POST /v1/metrics` OTLP receiver beside `/hook` and `/mcp`
(`handle_otlp_metrics`, same bearer auth). Claude Code's launch env
(`terminal.rs::claude_otel_env`) points its OTEL metrics exporter at it and
attaches `X-Oxplow-Thread` as an OTLP header (one process per thread →
constant), so the receiver attributes the `claude_code.token.usage` counter
without a session→thread lookup. Each export is logged as one
`agent.tokens.reported` event anchored to the turn it measured (P10.M2,
`otlp_ingest.rs`); the `token_usage.otlp` consumer turns it into
`oxplow.tokens` facts. Cache
kinds (`cacheRead`/`cacheCreation`) are tracked too (tsk73) — on the separate
`oxplow.cache_tokens`/`oxplow.cache_usage` measures, feeding
`agent.tokens.cache_read`/`cache_creation`/`cache_hit_pct` and the per-close
`task.tokens`; token-denominated only, never dollars. This
replaced the transcript-parse token capture, which overcounted ~2–3× (Claude
repeats a message's cumulative `usage` on every content-block line). The
`token_usage.turns` reactor on `agent.turn.ended` (`TokenUsageService::on_stop`)
records the per-turn `agent_token_usage` prompt rows + `oxplow.turn` facts. Details: `.context/metrics.md` → "OTEL token tracking".

Each hook POSTs to the runtime's MCP server with bearer-token auth via the
env-var-interpolated `OXPLOW_HOOK_TOKEN` header, plus `X-Oxplow-Stream`,
`X-Oxplow-Thread`, `X-Oxplow-Session`. The MCP server's `onHook` callback dispatches
to `runtime.handleHookEnvelope`, which:

**Which agent session a hook came from** (`agent_session_store::resolve_tx`,
shared by hooks and OTLP exports), in order:
1. the session the sender named (`X-Oxplow-Session`, the ACP host, the UI's
   interrupt) — refused, with a warning, when it is another thread's;
2. the session whose resume id is the hook's harness session id (an open
   one first, then the newest);
3. the thread's newest open session, one with a turn running first —
   logged as a warning, since every process oxplow starts names its
   session, so this firing means a sender that doesn't;
4. none: an agent oxplow didn't start. It records with the thread's
   anchors only, its turn and events carry no session (adopting it onto
   one is later work).

Turns are per session: a prompt opens a turn in its session, a Stop or
Interrupt closes that session's turns, a process start interrupts only
its own session's, and the resume id is the session's
(`agent_session.resume_session_id`). A PTY's exit ingests a `SessionEnd`
naming only its session (`TerminalSessionRegistry::ingest_exits_into`):
that session's open turn closes and its harness session ends — once, so
a harness that already posted its own SessionEnd (Claude) logs no second
one, and one that posts none (Codex) still ends.

1. (There is no in-memory hook ring any more — P3.9. What a hook did is its
   `agent.*` events in the log; the Hook events page lists them through
   `list_agent_events` and refetches on `ModelsChanged` naming `v_event`;
   the Work panel's live turn rows on `v_agent_turn` — P8.A10.)
2. Runs `HookIngestService::ingest` (`crates/oxplow-app/src/hook_ingest.rs`,
   P3.3): **one transaction per envelope** writes the state the hook changes
   and the `agent.*` events that record it, anchored to the thread's stream,
   its agent session, that session's open turn and the thread's single
   open effort (`activity_anchors_tx`).
   **Attribution is a best effort, by design (tsk511, decided with
   Nathan 2026-09-30).** Several agents, the person and outside processes
   can all change a worktree at once, so no rule ties every change to
   the right work exactly. The anchor stays **thread-scoped** — the
   thread's own open effort, tied to its active task — because that says
   more than the stream would, even when it's sometimes wrong. A writer
   thread editing under a sibling thread's effort (the guard allows it,
   tsk133) records no effort on its events; that's an accepted edge
   case, not a bug to close by tightening the guard or widening the
   anchor:
   - every prompt → `agent.prompt.submitted` (`reprompt: true` inside an
     open turn), its text in `event_content` (`prompt: ContentRef`, read
     with `read_event_content` body `prompt`) — so a re-prompt's text is
     kept, not only the turn's first; a prompt with no turn open also opens
     one (`agent.turn.started`);
   - PreToolUse → `agent.tool.requested` with the policy's `decision`
     (`HookEnvelope.decision`, set by the control plane's `pre_tool_check`
     and by `AcpHost::check_tool`); PostToolUse → `agent.tool.finished`.
     `path` / `detail` / `ok` / `exit_code` are computed here
     (`tool_calls::parse_tool_call` against the thread's worktree), and the
     tool's input and output go to `event_content` by hash (the payload
     carries `{hash, size}`). The harness's `tool_use_id` is the dedupe key,
     so a re-posted hook logs once;
   - Stop / Interrupt close the open turns (`agent.turn.ended@2`, with
     Claude's `transcript_path`);
   - a status the hook sets (Running, Idle / AwaitingUser, Stopped,
     session-start Idle) is logged as `agent.status.changed` when it differs from the
     thread's newest logged one, read in the same transaction
     (`last_status_tx`). **The log is the status** (tsk499): there is no
     in-memory copy, so a restarted daemon still knows a thread waits on
     the person. `SqliteAgentStatusStore` is the read side (`get`,
     `list_all` = every thread that has logged one; `v_agent_status` is
     each thread's latest); `HookIngestService::set_status` (ACP
     permission cards) is the only other writer and logs through the same compare. One lock
     spans each status-deciding transaction and its `AgentStatusChanged`
     emit, so announcements reach the UI in commit order. What the rail
     *shows* is still derived from activity (`list_agent_statuses`), so a
     dead agent reads as stalled rather than its last announced status.
   **Session tracking is part of it.** A session id seen for the first time
   on a thread (on any hook — Claude posts no HTTP SessionStart) logs
   `agent.session.started` once (dedupe key `session:<id>:started`) and
   becomes its agent session's `resume_session_id`, so a later restart relaunches with
   `--resume <id>`. **A `SessionStart` is a process start** (tsk500):
   every one except `source: "compact"` (a compaction inside a running
   turn) closes the turns the previous process left open as interrupted,
   logs `agent.session.started` again (`resumed: true` for the resume id)
   and sets the status Idle — so a turn that died without a Stop reads idle
   after the restart instead of running, then stalled. The ACP client posts
   the same `SessionStart`; there is no separate boot kind.
   `SessionEnd` first closes the turns **that session** opened and left
   open — an exit mid-turn sends no Stop — as interrupted ("session
   ended"), status Stopped (tsk449), so no turn holds the quiet-period
   snapshot trigger open after its agent is gone; another session's turns
   are left alone. (A crash that sends no `SessionEnd` is closed by the
   next `SessionStart`, above.) **Each close keeps the turn's
   transcript** (tsk924): `SessionEnd` passes its body's
   `transcript_path`, as `Stop` does, and a `SessionStart` passes its own
   only to the turns of the session it resumes (another session's file
   isn't theirs), so `TurnTokensConsumer` records an interrupted turn's
   tokens under it. A hook carries no process identity, so a `SessionEnd`
   delivered after its session was resumed would close the resumed turn;
   Claude posts it before exiting, before oxplow can resume. It logs `agent.session.ended` every time, and when
   `reason` is `clear` and the id is the resume id it blanks it: `/clear`
   starts a fresh session with no HTTP hook, so until its first prompt the
   token would still point at the cleared one. Other end reasons keep it so
   normal restarts still resume, and a clear of a stale session never wipes
   a newer token. (The in-memory resume cache is gone: the ingest reads
   the thread row in its transaction anyway.)
   A second cleanup runs at **launch** for a token that's stale for any
   other reason (transcript pruned, machine moved, id rotted). Before
   passing `--resume`, `open_terminal_session`'s direct branch probes the
   session file via `resume_check::claude_resume_state`
   (`crates/oxplow-app/src/resume_check.rs`): it maps the cwd to Claude's
   `$HOME/.claude/projects/<cwd-with-non-alnum→'-'>/<id>.jsonl` and, if
   the project dir exists but the `.jsonl` is gone (`Missing`), blanks the
   thread's pointer and launches fresh — so `claude --resume <stale>`
   never runs and its raw "No conversation found" error never reaches the
   terminal. Conservative by design: an absent project dir reads as
   `Unknown` (never clears), so an encoding drift can't wrongly wipe a
   valid pointer, and the shell `||` net in `agent_command.rs` still
   covers the file-vanishes-between-check-and-exec race. Claude-only;
   codex/opencode keep just the shell net.
3. Opens and closes `agent_turn` rows (UserPromptSubmit / Stop /
   interrupt, logging `agent.turn.*`), and a closed turn ends at a
   `turn_end` snapshot (see "Snapshot tracking" below). Per-effort
   attribution stays anchored to `effort`.
4. For `PreToolUse`: asks the shared **`AgentPolicy`** (see "Agent
   policy" below): the write guard (see Write guard below). It never
   asks for tracked work. A deny is rendered as Claude's
   `hookSpecificOutput`.
   `claude_intent(body)` returns `None` for any tool outside the four
   worktree-mutating edits, so `pre_tool_check` short-circuits
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
   launch-time system prompt no longer win. The same `additionalContext`
   also carries any **prompt advisories** that fire for the thread's open
   effort (`oxplow_app::advisories::for_thread`, see
   `.context/extensions.md` → "Advisories"): with `oxplow-bundled`
   enabled, the metric deltas block and one-shot threshold crossings. The
   pieces are joined with a blank line; any may be absent.
6. For `Stop`: closes the turn, keeping the payload's
   `last_assistant_message` as its `answer`, and acks `{}`. A stop is
   never refused.

**Side-band hook steps are best-effort by design.** The PostToolUse
extras (collection observations, wiki-page attribution) are individually
try/warn — a coverage parse failure must
never fail the hook or block the agent. This is deliberate policy,
not an oversight; the durable lifecycle writes they decorate are
covered by the transactional invariants in `data-model.md` instead.

**Hook handling is time-bounded.** Claude Code blocks on the hook
response, so the control plane races the whole post-auth pipeline
against a 5s timeout (`HOOK_HANDLING_TIMEOUT` /
`bounded_hook_response` in `crates/oxplow-control-plane/src/lib.rs`).
On expiry it logs a warning and returns the generic ack — tool call
allowed — so a wedged DB (e.g. the writer lock held by
a snapshot flush) can never stall the agent. Availability over
enforcement: the MCP tools re-check write-guard + filing at the call
site, so a timed-out PreToolUse deny is still caught there.

## ACP agents: configuration (tsk335)

**Sessions.** `AgentKind::Acp` sessions name an ACP agent in
`agent_session.acp_agent`. `AgentKind::is_terminal()` is false for them:
`open_terminal_session` refuses them, and `write_agent_runtime` returns
`PluginError::NotTerminal`.

**Configuration.** Agents come from `oxplow_config::acp_presets()`:
- claude: `claude-agent-acp`;
- gemini: `gemini --acp`;
- codex: `codex-acp`.

These are layered with the project's `acpAgents: [{name, command, args,
env}]` by `resolve_acp_agents`; a project entry replaces a preset of the
same name.

**Listing.** `oxplow_app::acp::agents::list` (IPC `list_acp_agents`,
UI-only) reports each agent's source, whether it may start here, and its
resolved path (`agent_path::resolve_program`). Presets may always start;
a project entry needs a person's approval in Settings → Data → Programs
(`exec_consent`, `ProgramKind::AcpAgent`).

**Creating threads.** `oxplow.thread.create` takes `acp_agent` (stored on the
thread's agent session). It's required for
`agent: acp`, refused otherwise, and must name a known agent. The
new-thread picker lists "ACP · <name>" per agent when ACP is enabled in
`agents:`, flagged "not installed" or "needs approval" (`agentChoices` in
`agentKinds.ts`).

**Not built yet:** a personal (user-global) `acpAgents` file; presets and
project entries only for now.

## ACP agents: protocol mapping and transcript (tsk336)

**The SDK is pinned (`agent-client-protocol = "=2.2.0"`) and fenced.**
`acp/wire.rs` is the only module that names its schema types. It
converts them to oxplow's `acp/model.rs` types (`ToolCall`,
`ToolCallPatch`, `AcpUpdate`, `PermissionAsk` / `PermissionAnswer`), so
an SDK bump touches one file. Content text is capped at 64 KiB per
block.

**Gotcha: the SDK turns on `serde_json/preserve_order` workspace-wide**
(Cargo features unify). JSON objects serialize in insertion order, not
sorted order. Anything that needs a stable text form must build it from
a `BTreeMap` explicitly: `metric_cube::dims_key` does, and approval hashes
never hash JSON. Hook response bodies changed key order only, which is
the same JSON.

**`acp/mapping.rs` (pure):**
- `intent_for` makes the policy intent:
  - edit, delete and move are `WorktreeWrite`, anything else is `Other`;
  - paths come from `locations`, then the diffs, then path-like `rawInput` keys (`file_path`, `path`, `absolute_path`, `notebook_path`, `source`/`destination`, `old_path`/`new_path`), deduplicated.
- `canonical_events` makes the Claude-shaped events the ingest records:
  - read → `Read`;
  - edit → `Edit`, or `Write` when every diff is a new file;
  - delete and move → one `Edit` per path, so effort claims see them;
  - search → `Grep`, execute → `Bash`, fetch → `WebFetch`;
  - `mcp__…` titles or names pass through, and think / switch-mode record nothing.
- The `tool_response` is `{is_error, …rawOutput}` once the call finishes, so a Bash `exit_code` survives.

**`acp/transcript.rs`: the in-memory conversation (no table).**
- **Items:**
  - user, agent, thought;
  - tool (merged with its updates by tool-call id);
  - plan (replaced within a turn);
  - permission;
  - policy-denied, bypass;
  - error.
- **Ids and seqs:** each item has a stable `id`, plus a `seq` that is bumped from one counter every time the item changes. `since(seq)` returns new and changed items, and clients upsert them by `id`.
- **Chunks** of the same kind coalesce into the trailing item.
- **`usage_update`** is transcript state (the context meter), not an item.
- **Size:** the ring is capped (2000 items by default).

## ACP agents: sessions (tsk337)

**Shape.**
- `Services.acp` (`acp/manager.rs`) holds each **agent session**'s command sender and a shared `SessionView` (status, transcript, stderr tail), keyed by `AgentSessionId`: two ACP sessions in one thread are two agents with their own transcripts.
- One actor task per session (`acp/session.rs`) owns the connection.
- `wire::run` feeds every agent message into ONE channel, and the prompt's result is an ordered barrier, so the actor sees updates, requests and turn end in wire order.
- Events for every session go out on one broadcast channel (`AcpEvent`).
- **Open and close are race-free (tsk359).**
  - `open` reserves the session's slot under one lock before spawning anything, so concurrent opens start one agent.
  - `close` marks the handle closed at once.
  - A closed session still winding down is replaced and marked not current; its actor then records no Interrupt over the new session's status.
- **Thread lifecycle.** Closing a thread (`oxplow.thread.close`) closes its open effort in the same transaction, and once the close commits it stops every open session's process — ACP agent or PTY — through `agent_sessions::SessionProcesses::kill`. A fork (`oxplow.thread.create { from }`) keeps the source session's harness and `acp_agent`.
- The agent runs via `tokio::process` with `kill_on_drop` and an augmented `PATH`; its stderr's last lines are kept for a failed start.

**Host.** `acp/host.rs` `AcpHost` is the seam (tests use a recording double). `ServicesAcpHost::new(svc, stream, session)` holds `Weak<Services>` (sessions live in Services) and the agent session it hosts — every envelope and status it records names it — and records exactly what a hooked turn records:
- `SessionStart` plus the resume id on start (a start closes turns a previous process left open and resets the thread to idle);
- `UserPromptSubmit` on the person's prompt;
- `PreToolUse` on every policy check;
- `PostToolUse` plus `AgentContext::post_tool_context` on each finished tool call (per canonical event);
- `Stop` with the turn's reported counts and its last agent message on its body (counted by the `token_usage.turns` reactor; the message is the turn's answer);
- `Interrupt` when the agent goes away.

**Starting.**
- `initialize` offers fs read/write and no terminal.
- **MCP:** oxplow's MCP rides `session/new|load` as an HTTP MCP entry. An agent without HTTP MCP support is refused with a clear error.
- **Resume:** the agent session's `resume_session_id` is `session/load`ed when the agent supports it. The replay rebuilds the transcript and records nothing: no hooks, no tool rows, no bypass checks, and fs writes are refused.
- **System prompt:** it goes in `_meta.systemPrompt.append` when `system_prompt_via_meta` (the Claude adapter). Otherwise it is a block ahead of the first prompt of a new session.
- **Skills (tsk376):** an ACP agent discovers no skill files, so its system prompt ends with an `# oxplow skills` index (`AgentText::skill_index` of what's offered now, `capabilities::agent_text`: name + frontmatter description) and it reads a body with the read-only, agent-only MCP tool `get_skill(name)`. Terminal runtimes still ship the files; boot (`boot.rs`, once services are up), an extension change and a `capability.switched` call `capabilities::refresh_agent_text` to rewrite the skills and commands of runtimes already on disk (never creating one), so an agent outliving an upgrade or a switch reads the current ones.

**Prompts (the no-automation rule).**
- `acp/human_prompt.rs` `compose` is the only way to make a `HumanPrompt`, and `wire::AgentConn::prompt` accepts nothing else.
- The session calls it only for `Command::Prompt`, which only `AcpManager::submit_human_prompt` sends.
- A second prompt while a turn runs is `TurnInFlight`, never queued.
- Session context, advisories, decisions and post-tool nudges ride the person's prompt as a visible leading block (shown behind a disclosure on the user item).
- `acp/guard_tests.rs` scans the Rust source so that:
  - `PromptRequest` appears only in `wire.rs`;
  - no raw `"session/prompt"` appears outside the fake;
  - `compose(` is called only from the session;
  - `submit_human_prompt(` has no other callers.

**Permissions.**
- Each `session/request_permission` is checked by `AgentPolicy`:
  - **Deny:** answered `reject_once` (or `cancelled` when no reject option exists) with a policy-denied item. The model sees only the rejection; a reason can't be carried.
  - **Allow:** a permission card waits for the person, with no timeout. "Always allow" is dropped for writes, the status is AwaitingPermission, and the thread status is `AwaitingUser`.
- Cancel answers every open card `cancelled`, as the protocol requires.
- The cards' "awaiting" is cleared BEFORE the Stop is ingested, so the Stop decides the status from the turn's final message.
- **Status after the cards.** When the cards are answered or cancelled, the host restores what the first card interrupted: a question the thread was already waiting on survives, and otherwise the status is Running if a turn is open, else Idle. The view likewise goes to Running inside a turn and Idle outside one.
- **Teardown is one path (tsk361).** `session::Teardown` cancels unanswered cards, adds the error item, records the Interrupt (only while the session is current) and emits Stopped/Closed. The actor runs it on a clean exit. The task driving the connection runs it again afterwards, which is a no-op once stopped, so a transport error that drops the actor mid-loop still ends the session.

**fs and bypass.**
- `fs/write_text_file` is policy-checked. A deny returns a JSON-RPC error whose message is the reason, which does reach the model.
- **Confinement (tsk351):** oxplow reads and writes files for an agent only inside the session's worktree. Paths are normalized first (`..`, symlinks); anything outside is refused, with the reason going back to the agent. Relative paths resolve against the cwd.
- **Bypass detection (tsk362):**
  - A write-kind tool call that completes after the policy or a person **rejected** (or cancelled) it gets a bypass banner: the agent ignored the answer.
  - One that completes without anyone **allowing** it, and without every path written through `fs/write_text_file`, is checked afterward; if the policy would deny it, it gets a banner too.
  - This is the only backstop for adapter modes that skip asking.

**Turn end.**
- Open cards are cancelled.
- Stop, tokens and directive are recorded.
- The directive is stored in `SessionView.directive` and shown as an item and event. **It is never sent.** `dismiss_directive` clears it.

**RPC and events (tsk338).** `oxplow-rpc/src/commands/acp.rs`; every command is a `ui(...)` parity row, so none can become an MCP tool.
- **`acp_open_session`** (a ctx row with a hand-written Tauri adapter, because it needs `plugin_runtime` for oxplow's MCP URL and token). It:
  - takes the agent session's id, and checks it is open and an `acp` session (its thread and stream come from the row);
  - resolves the agent (`find`, `may_start`, `resolve_command`);
  - assembles the system prompt (`system_prompt_via_meta` is true when the command is `claude-agent-acp`);
  - passes the session's resume id;
  - opens the session and returns the `AcpSnapshot`.
- **The rest:**
  - `acp_prompt`, the prompt box's Enter, and the only caller of `submit_human_prompt`, which the guard pins;
  - `acp_cancel`;
  - `acp_respond_permission` (no option means cancel);
  - `acp_transcript(sinceSeq)`, which returns `null` when there's no session;
  - `acp_dismiss_directive`;
  - `acp_close_session`.
- **Every RPC takes the session id** (`sessionId`), and `acp_close_session` stops the process only — the session's slot closes with its command.
- **Events:** the `acp:event` channel (frame key `acp`) carries `AcpEvent { agentSessionId, threadId, generation, type: item|status|directive|usage|closed, … }` over the daemon's `/events`. It is in `event_channels::FRAMES` and in TS `EVENT_CHANNELS` / `CHANNEL_ROUTING` (multiplexed).
- **Bindings:** raw agent JSON (`rawInput` / `rawOutput`) is TS `unknown`, via `specta_typescript::Unknown`, because specta's own `serde_json::Value` rendering doesn't typecheck.

**UI (tsk339).** `AgentPage` renders `components/acp/AcpAgentView.tsx` (by `sessionId`, filtering events by `agentSessionId`) for an `acp` session instead of the terminal.
- **Transcript state:** `acpTranscript.ts` is a pure reducer over the `acpTranscript` snapshot and live `acp:event`s. Items upsert by id and the newer seq wins. A seq gap, or a remote reconnect (`onRemoteReconnect`), refetches `since(headSeq)`. Past a gap, `headSeq` stays at the last contiguous seq, so that refetch includes the missed items (tsk356). Each open of a thread's session is a new **generation** (`AcpEvent.generation`, `AcpSnapshot.generation`) whose ids and seqs restart. The reducer starts over on a new generation, so a Restart never mixes old and new transcripts, and late events from a closed session can't overwrite the new one (tsk357). The view subscribes to events before fetching, so nothing between the two is lost.
- **Opening:** it opens the session on mount when none exists. A failure shows the error with Retry, plus "Open settings" when the agent needs approval.
- **Items:**
  - the user message, with an "oxplow context" disclosure;
  - agent markdown;
  - collapsible thoughts;
  - tool cards (status, locations, "View diff", which opens `DiffPane` with the literal old/new text, and collapsible output);
  - the plan;
  - permission cards whose buttons call `acpRespondPermission`;
  - the policy-blocked notice, the bypass banner, and errors.
- **Header:** the status and the context meter (`used/size`, with cost in the tooltip).
- **The directive banner** offers "Put in input", which ONLY fills the draft, and "Dismiss".
- **`AcpPromptBox.tsx`:**
  - it is the only renderer caller of `acpPrompt`; `no-agent-input-automation.test.ts` pins that;
  - Enter sends, Shift+Enter adds a newline, Escape or Stop cancels;
  - nothing sends while a turn runs;
  - "Add to agent context" (`agent-input-bus`) appends to the draft while the box is visible.
- **Component test:** `AcpAgentView.test.tsx` asserts that "Put in input" never sends and Enter sends once.
- **Bindings:** `ItemBody` / `PermissionAnswer` / `AcpEventBody` use `rename_all_fields = "camelCase"` so TS sees `requestId` / `optionId`. `AcpEvent` is exported to the bindings (`.typ::<AcpEvent>()`).

**Limits (know these before trusting the gate).**
- **A permission reject carries no reason.** The model sees only "rejected". Only an `fs/write_text_file` denial (an error message) tells it why. The Claude adapter writes to disk itself, so its only gate is the permission request.
- **Adapter modes that skip asking** (e.g. an "accept edits" / bypass mode set inside the agent) leave bypass detection as the only backstop. That is after the fact: the write already happened, and oxplow flags it with a banner.
- **"UI-only" is enforced twice.** The `ui(...)` rows keep `acp_*` out of the MCP tool surface, and the daemon's `/ipc` requires the per-launch UI token that only the renderer holds (tsk345, [remote-daemon.md](./remote-daemon.md) → "Auth"). So no agent can prompt an agent by calling the daemon directly either.
- **Usage:**
  - per-turn tokens come only from the prompt response's `usage` (an unstable ACP field, enabled via the SDK feature `unstable_end_turn_token_usage`) and are recorded via `record_turn`;
  - `usage_update` (context occupancy plus cumulative cost) drives only the context meter;
  - ACP agents get no OTEL env, so nothing double-counts.
- **System prompt:** `assemble_acp_system_prompt` omits the `<session-context>` block. The first human prompt always carries a fresh one via `prompt_context`, so including it here would send it twice.

**Verification.**
- **Headless (tsk340):** the daemon plus Playwright, with `oxplow-acp-fake` as a project `acpAgents` entry, confirmed:
  - the approval gate (with "Open settings");
  - a prompt → reply with plan, tool and context meter;
  - a read-only thread's edit blocked with no card;
  - a writer's card without "always allow", which round-trips;
  - the Stop audit shown as a banner, with "Put in input" filling the draft and nothing sent;
  - the MCP entry the agent received (URL plus Bearer / `X-Oxplow-*`) initializing against oxplow's MCP, where a wrong token gets 401.
- **Live smoke:** `tests/acp_live.rs` is `#[ignore]`d. Run it with `OXPLOW_ACP_LIVE_CMD="<adapter command>" cargo test -p oxplow-app --test acp_live -- --ignored` (e.g. `bunx @zed-industries/claude-code-acp`).
- **Not done:** recorded traces from real adapters, because this machine has no Node.

**Fake agent.** `crates/oxplow-acp-fake` is a scripted fake speaking raw JSON-RPC, deliberately not the SDK, so the tests exercise real wire JSON. `fake:<step>` lines in a prompt drive it: say, think, edit, bypass, fswrite, fsread, bash, plan, usage, tokens, wait, crash.
- `acp/session_tests.rs` runs it in-process over a duplex pipe.
- `tests/acp_services.rs` runs it against real `Services`, and once as its binary.

## Agent policy (shared by every transport, tsk333)

The write guard is one policy that every agent transport asks, not logic
in the hook route. It is isolation only; nothing waits on tracked work
([work-tracking.md](./work-tracking.md)).

**The one rule (confirmed 2026-10-08):** the guard keeps agents off
worktree files, nothing more. A non-writer thread may not change its
own stream's worktree, and no thread may change another stream's.
Everything else is open to every thread, writer or not: files outside
every stream, the shared wiki (through `oxplow.knowledge.write_page`),
and oxplow's own objects — filing, editing, starting and finishing work
items, opening and closing efforts, notes, comments. A task's state and
an effort are records, not claims on the worktree
(`commands/work_item.rs`, `commands/effort.rs`), so a research or review
thread can say what it's working on and get its own effort bucket. The
bus's `WriteGate` applies to `Write` commands only; `Record` commands
skip it ([commands.md](./commands.md) "Agent policy").

**An agent may open a sibling agent session** in its own stream
(`oxplow.agent_session.open`, decided 2026-10-08): the row makes the UI
attach and start its process, and that is all — nothing types into it.
The no-automation guards (`forward_terminal_input` UI-only,
`submit_human_prompt`'s single caller, `no-agent-input-automation.test.ts`)
are what keep a session an agent opened from being driven. Its close is
destructive and waits for a person as a proposal.

- **Pure rules** live in `crates/oxplow-runtime/src/policy.rs`.
  `decide_tool(ToolIntent{label, kind, paths}, PolicyFacts)` returns
  `Allow`, or `Deny { layer: WriteGuard, reason }`.
  - **Scope:** the thread's own stream worktree (`PolicyFacts.worktree_root`). For a worktree stream that's its sibling directory, not the daemon's project dir (tsk350; before that, worktree streams were unguarded).
  - **Normalization:** paths are normalized first (`normalize_path`: resolve `..`, canonicalize the deepest existing ancestor), so `..` or a symlink can't spell a guarded path as an outside one.
  - **Other streams:** a path in another stream's worktree (the primary checkout included) is denied for every thread, writer or not (workspace isolation). The primary project's `.oxplow/wiki` is shared and exempt.
  - **Outside every stream:** any other absolute path is allowed.
  - With several paths, the first refused path wins.
  - The reason text comes from the same cores the Claude builders use
    (`write_guard::read_only_reason`), so the wording can't drift.
- **I/O and state** live in `crates/oxplow-app/src/agent_policy.rs`,
  exposed as `Services.agent_policy`:
  - `check_tool(svc, thread, intent)` gathers the thread, the stream's
    `in_progress` claim and git state.
  - `check_command(thread, spec)` is the command bus's agent gate
    ([commands.md](./commands.md)): an agent may run a command only when
    its spec admits agents. `DenyLayer::Command` names the refusal.
  - `claude_intent(body)` maps a Claude-shaped payload to an intent.
- **Recording is the ingest's and the pump's; context is shared.** Every
  transport hands its envelopes to `HookIngestService::ingest`, which logs
  `agent.*` events; pump reactors record from them (tool-call rows, effort
  claims, wiki attribution, collection, advisories, tokens — P3.5–P3.7).
  What the agent is told comes from `Services.agent_context`
  (`crates/oxplow-app/src/agent_context.rs`, `AgentContext`, P3.8 — it was
  `AgentActivity`, whose hand-wired fan-out is gone):
  - `post_tool_context` returns the ROLE CHANGE banner, else the thread's
    undelivered nudges after settling the reactors that write them.
  - `prompt_context` builds the session-context block, advisories and
    decisions, deduped per session.
  - `reset_session` clears the per-session baselines. Session and resume
    tracking live in the hook ingest (P3.3).
  - Transports that don't speak Claude's tool vocabulary build a
    `CanonicalToolEvent` (`crates/oxplow-app/src/acp/mapping.rs`) and ingest
    its `to_payload()`. That is the one place the canonical shape is built;
    the ingest and every reactor key on Claude's tool names.
- **Transports only render the answer.**
  - The hook route renders `hookSpecificOutput` for a deny.
  - An ACP agent gets an automatic permission reject or an fs error.
- **Byte-for-byte pins.** `crates/oxplow-control-plane/tests/hook_goldens.rs`
  pins the Claude responses byte for byte (`UPDATE_GOLDENS=1` rewrites
  them; a changed golden is a changed agent contract).

## Stop hook

A Stop is never refused. oxplow used to block stops (an audit of open
efforts, an effort review, a filed-but-didn't-ship check) to make the
agent declare and close its work; that is gone
([work-tracking.md](./work-tracking.md)). The Stop ingest closes the
turn, keeps `last_assistant_message` (Claude's Stop payload; ACP sends
its transcript's last agent message under the same key,
`hook_ingest::LAST_ASSISTANT_MESSAGE`) as the turn's `answer`, takes the
turn-end snapshot, sets the status (keeping `awaiting_user` when the
turn set it) and acks `{}`. ACP's `turn_ended` and opencode's
`session.idle` do the same and nothing else.

## Subagents and the work queue

Subagent tool calls reach the hooks like any other, so a subagent's
edits and test runs land in the thread's effort with nothing from it:
there is no dispatch protocol, brief or subagent skill. An Explore
subagent can still write its finding to a thread note allocated for it
(`oxplow.knowledge.add_note`, then `oxplow.knowledge.update_note`), read back with
`list_thread_notes`.

`next_work_item` (`crates/oxplow-mcp/src/lib.rs`, backed by
`oxplow_app::work_item_reads::next`) suggests what to work on next on a
thread's list, whichever list is active, in one of three shapes:
- `{ mode: "epic", epic, children }` — the first ready (`todo`) item, in
  list order, is an epic (an item with children); its ready descendants
  no open `blocks` link holds are its children.
- `{ mode: "standalone", items }` — the head is not an epic; every ready
  non-epic item no open blocker holds, to pick one or a cluster.
- `{ mode: "empty" }` — nothing ready (always, with none).

`/work-next` (the bundled extension's command) calls it and works the item it picks, moving it to
`in_progress` so the effort links to it. Claude Code's built-in
`TaskCreate` is a within-turn micro-planner and never mirrors oxplow
items.

## MCP tools

**Caller identity and commands.** Every MCP request carries the acting
thread (`X-Oxplow-Thread` / `X-Oxplow-Stream` headers, or `?thread=` on
the endpoint URL for Codex); `oxplow_mcp::caller_of` turns it into the
`Actor::Agent` the command bus audits to. Writes that are commands go
through `run_command` (`list_commands` shows what the agent may run);
an anonymous connection may read but not run commands. Any thread in
the stream may file, edit, start and finish tasks and open efforts
(`work_item.*` and `effort.*` are `Record` commands: a record, not a
claim on the worktree — see "Agent policy" below). Per-harness plumbing
and the rule live in [commands.md](./commands.md).

`buildTaskMcpTools` (`crates/oxplow-mcp/src/lib.rs`) registers the agent's
tool surface. Internally each `ToolDef.name` carries an `oxplow__`
prefix (historical), but `crates/oxplow-mcp/src/lib.rs` strips that prefix at the
`tools/list` boundary via `exposedToolName` so the harness sees clean
names like `run_command`. With the harness's own `mcp__oxplow__`
namespace on top, the agent calls `mcp__oxplow__run_command` —
not the legacy `mcp__oxplow__oxplow__run_command`. The long form
still resolves on `tools/call` for back-compat.

**Tools read the interface.** No MCP tool belongs to one implementation:
the work-item tools (`list_work_items`, `get_work_item`,
`next_work_item`) read whichever list is active (empty with none), so
the tool list never changes with a switch. What an implementation owns
is commands — a provider instance's namespace — offered only while it's
active (`capabilities::Active::refusal`).

**The server's instructions** (`get_info`) are the one text every
harness — Claude Code, Codex, opencode, ACP agents — shows its agent. They
say what oxplow knows (`query_sql` over `v_*`), that writes are commands,
and how to answer the person: when asked to see, list, compare or track
something, show it with `show_lens` rather than printing it (tsk1033: the
first acceptance walk's "Show me" got terminal text). They ship to users'
projects, so they never name this repo's docs.

### The ServerHandler is hand-rolled — re-diff it on every rmcp bump

`impl ServerHandler for OxplowMcp` writes out `list_tools` / `call_tool`
/ `get_tool` instead of using `#[tool_handler]`, only so `list_tools`
can stamp `read_only_hint` (tsk203). That means rmcp can change what
the macro generates without our copy following: rmcp 3 added
`result_type` / `ttl_ms` / `cache_scope` to `ListToolsResult` and made
`call_tool` return `CallToolResponse`. On an rmcp upgrade, diff the
three methods against `rmcp-macros/src/tool_handler.rs`.
`crates/oxplow-control-plane/tests/mcp_wire.rs` runs a real
Streamable-HTTP session (initialize → tools/list → tools/call) against
`/mcp` — the only test that crosses rmcp's protocol layer.

### Param casing is lenient (camelCase aliases tolerated)

Param structs are snake_case (`thread_id`, `work_item`) and that
stays the **canonical/advertised** form — the JSON schema is derived
unchanged from each struct. But tool *outputs* are camelCase
(`itemId`, …) and weak models (opencode / GPT-5-mini) carry camelCase
priors from training, so they'd send `workItem` / `threadId` and
hit an opaque `-32602 missing field "work_item"` they can't act on.

`lenient_params::Parameters` (in `crates/oxplow-mcp/src/lib.rs`) is a
drop-in replacement for rmcp's `Parameters<T>` — same name, so the
`#[tool]` macro still recognizes it in handler signatures and derives
the schema from `T`. Before deserializing it **additively** inserts a
snake_case copy of any camelCase/kebab key (never removing the
original), recursing into nested objects/arrays, so snake_case input is
untouched, a camelCase call just works, and a genuinely missing field
yields a clear self-describing `McpError` (not a raw transport error).
This is global — every tool and nested param struct gets it for free,
including future ones. See `lenient_from_object` + `to_snake_case`.

Complementary hardening: required-but-inferable fields are made
optional and inferred in-handler where safe (e.g. `list_comments`
infers `scope` from `id`'s prefix). The goal is the same — a reasonable
call shouldn't 32602.

### Surface parity with the IPC adapter

The MCP tool surface (agent) and the Tauri IPC command surface (UI,
`crates/oxplow-tauri-ipc/`) are two thin adapters over the same
`oxplow_app::Services`. They drifted silently — many user-meaningful
ops lived on IPC but not MCP. The `oxplow-surface-parity` crate
(`crates/oxplow-surface-parity/`) now guards this: a checked-in
`MANIFEST` classifies every op as `Both`, `AgentOnly`, `UiOnly { why }`
(the reason an agent has no tool: a person's consent or setting, their
selection or input, live status pushed to the UI, runtime infra, an
ancestry walk the VCS answers, file I/O the agent does with its own
tools) or `Model { models }` (a UI read an agent makes through those
published models with `query_sql`). Every row is decided — there is no
"tool not built yet" exposure (P11, tsk944) — and `tests/parity.rs`
enumerates the *actual* registered names on each
surface (MCP via `oxplow_mcp::registered_tool_names()`, IPC via a
capturing `tauri_specta::LanguageExt` over `specta_builder()`) and
fails if anything is unclassified, dangling, or a `Both` row is missing
a side. Names may diverge per surface (e.g. IPC `list_comments_for_stream`
↔ MCP `list_comments`), so each row carries both names.

**Consequence for new work:** adding a `#[tool]` (or a
`#[tauri::command]`) requires a `MANIFEST` row or the parity test
fails. A UI-only row must say why (`every_ui_only_row_says_why`), and a
model row must name published core models
(`every_model_counterpart_is_published`).

Domains mirrored onto MCP so far (beyond the original task/wiki/comment
surface): **VCS reads** (`git_status`, `vcs_log`, `vcs_blame`, `diff`
(two revisions; `since_fork` for a branch's own changes), `read_at` —
revisions are `working`, `snap:<id>` or `git:<rev>` — and
`vcs_branches`; `stream_id` optional;
mutations stay on Bash); **snapshots / local history**
(`list_snapshots_for_stream`, `list_files_for_snapshot`,
`get_snapshot_stats`, `list_snapshot_change_entries` take a `snapshot_id`
— a whole capture; `get_file_snapshot` and `read_file_snapshot` take a
`file_snapshot_id` — one captured file row; `read_file_at_snapshot
{ snapshot_id, path }` reads a path as of a capture. Restoring is the
destructive `oxplow.snapshot.restore_file` command (an agent's is a proposal —
P8.A9). Reads and restore share `oxplow_app::snapshot_files::SnapshotFiles`, and a
restore writes into the row's stream's worktree, not the primary
checkout — P2.9, tsk433);
**code quality** (`run_code_quality_scan`, `list_code_quality_scans`,
`list_code_quality_findings` — the scan orchestration is shared via
`Services::run_code_quality_scan`); **selection** (`select_thread`,
`switch_stream`; a thread's and a stream's lifecycle are the `thread.*`
and `stream.*` commands, comments and notes the `knowledge.*` ones, all
through `run_command`, P8.A3–A6); and **site-wide search** (`search` —
BM25 over tasks/comments/notes/wiki/file-contents via the unified FTS index,
fed by the `search:<kind>` assets for tasks, comments, notes and wiki and the
`search.index` consumer for files; optional `stream_id` scopes file hits).
A file's captured history is the model `v_snapshot_file`, and an
effort's files (claimed or observed) `v_effort_file`; git mutations
stay on the agent's own git.

The `kind` discriminator (`epic`/`task`/`subtask`/`bug`/`note`) was
removed end-to-end — `oxplow.work_item.create` takes none and a task row no
longer carries one. An "epic" is just any task that has children
(create the epic, then each child with `parent_ref`); the bucketing is
computed on read.

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
`crates/oxplow-mcp/src/lib.rs`. A work item is named by its ref, or a
loose id of the active list (resolved by the `work_item.*` commands).

`oxplow.work_item.transition { ref, to, native_state? }` moves an item
between canonical states directly — `blocked → in_progress` (unblock)
and `done → in_progress` (reopen) need no hop through `todo`; archive
is `{ to: done|canceled, native_state: archived }`.

- Work-item reads are MCP tools (`list_work_items`, `get_work_item`,
  `next_work_item`, `get_open_effort`, …) over the work-item interface;
  every write is `run_command` (P8.A10 deleted the MCP task-write tools —
  `create_task`, `update_task`, `complete_task`, `upsert_task`,
  `transition_tasks`, `reorder_tasks`, `file_epic_with_children`):
  `oxplow.work_item.create { title, body?, parent_ref?, state?, native?,
  thread? }` — always on the active tracker; filed on the agent's own
  thread unless `thread` names another; `native` is the tracker's own
  fields (oxplow: `{ priority? }`); `state` defaults to `todo` = oxplow
  `ready`; the result carries `ref` — `oxplow.work_item.update`,
  `oxplow.work_item.transition`, `oxplow.work_item.reorder { ref, before?, after? }`,
  `oxplow.work_item.link` / `oxplow.work_item.comment`. There is no agent delete —
  `oxplow.work_item.delete` is destructive, and an agent never confirms one:
  cancel or archive instead. Closing is `oxplow.work_item.transition → done`;
  the effort policy closes the thread's effort.
- `oxplow.effort.report { thread?, summary?, impacts? }` is optional and returns
  `{ effort, link_warnings }`. It records a summary and the impacts
  beyond the edits on the thread's open effort, else its latest
  (`SqliteEffortStore::latest_for_thread`) — never opening, closing or
  creating one. A person names the thread; a thread with no effort is
  refused at `/thread`. It takes no files or runs — those are observed
  (`.context/work-tracking.md`) — and without a report `v_effort.summary`
  is the final message of the effort's last turn.
- **An effort's files are claimed or observed, never declared.**
  `effort_file.source` is `claimed` when an edit tool named the path
  (the `effort.claim` reactor, "Per-effort write log" below) and
  `observed` when it changed during one of the thread's turns: the
  `effort.observe` consumer (`crates/oxplow-app/src/effort_observation.rs`,
  after `effort.policy`, on `thread.checkpoint`) diffs the turn's start →
  end snapshots and records each changed path on the effort holding the
  turn, unless another thread's overlapping effort claimed it
  (`effort_store::observe_files_tx`). A file two threads changed at once
  and neither claimed is observed by both. Changes between turns (the
  person's own) belong to no effort. The diff is a **content diff** —
  `SqliteSnapshotStore::diff_snapshots(start, end)` reconstructs each
  path's content as-of each boundary and reports a path only when its
  `blob_hash` differs (the shared `oxplow_domain::diff_trees`), so an
  edit-then-revert doesn't count. Two pieces make the capture work:
  1. `SnapshotCaptureService::request_snapshot` sleeps for
     `DEFAULT_PREDRAIN_DELAY` (300 ms) before draining the dirty set so
     the fs-watch debouncer (250 ms in `workspace_watch`) has time to
     deliver in-flight events.
  2. There is **one `SnapshotCaptureService` per stream**
     (`SnapshotCaptureRegistry`), each watching its own worktree, so an
     effort on a worktree stream captures against THAT worktree.
- **Paths the project never snapshots belong to no effort.**
  `EffortService::claimable_paths(thread, paths)` runs every claim and
  observation through the stream's `WorkspaceFilter` (the project's
  `generated.exclude` list + `.gitignore`) and drops what capture
  excludes. With no capture service reachable for the thread (bare
  `EffortService`, unregistered stream) paths pass through unfiltered.
- **A run is the effort its causing tool call was in.** Producers
  observe-always: tests, analysis and coverage are recorded whether or
  not an effort is open (coverage stores absolute line-sets and derives
  its effort diff at read). The capture is stamped with
  `CollectionService::resolve_owner(thread, anchored)` — the tool event's
  effort anchor, else the thread's open effort — as
  `metric_capture.effort_id`, and an effort that adopts a turn restamps
  its captures. Subagent tool calls reach the hooks like any other, so a
  subagent's runs need nothing from it; `oxplow.test.record_run` is only for
  counts oxplow couldn't parse from a run's output.
- `add_followup({ threadId, note })` / `remove_followup({ threadId, id })` /
  `list_followups({ threadId })` — orchestrator-only, in-memory transient
  follow-up reminders. No DB row, lost on runtime restart. Surfaces as
  italic muted "↳ follow-up: …" lines at the top of the To Do section
  in the Work panel. Use when you defer a sub-ask mid-turn that doesn't
  warrant a full `oxplow.work_item.create`. Always call `remove_followup` in
  the same turn you handle it. Never file both a follow-up and a real
  task for the same concern. NOT exposed to subagents — the dispatch
  brief deliberately omits any mention of follow-ups so subagents can't
  stash bookmarks they'll never come back to handle. See the agent
  skill at `.oxplow/runtime/claude-plugin/skills/oxplow-runtime/SKILL.md`
  for the decision rule (follow-up vs. task). Storage:
  `crates/oxplow-app/src/followup.rs`; runtime publishes the bus event
  `followup.changed` so the UI re-reads that thread's work
  (`workItems.readWorkList`).
- Forking a thread is `run_command oxplow.thread.create { from }` — see
  "Forking a thread" above.
- `list_comments({ id, scope?, status? })`, then `run_command
  oxplow.knowledge.reply_comment { comment, body }` / `oxplow.knowledge.update_comment
  { comment, status: "resolved" }` (P8.A6) — the user's
  threaded annotations anchored to text in pages (wiki / file / task).
  `id` is a thread id (`thr…`) or stream id (`str…`, the whole
  workspace). `scope` (`"thread"` / `"stream"`) is **optional** — when
  omitted it's inferred from `id`'s prefix; pass it explicitly only to
  assert the kind (a mismatch then returns an agent-readable error
  rather than silently inferring). A missing/blank `id`, an
  uninferable id, or a bogus `scope` string all return a clear
  in-handler `McpError` naming the fix, not a raw transport -32602 —
  see `resolve_comment_scope` in `crates/oxplow-mcp/src/lib.rs`.
  `status` filters `"all"` / `"open"` / `"needs_response"`. A reply is
  authored `"agent"` — the actor, never an input — which clears
  `needs_response` until the user replies again; an agent comments only
  in its own stream and thread. **The runtime
  never force-triggers any of this — there is no Stop-hook branch and
  no synthesized work item for comments.** The agent only touches
  comments when the user prompts it (typically via the
  `/review-comments` plugin command, which just wraps these calls).
  `comment_id` is an integer (comments use autoincrement ids). Store:
  `crates/oxplow-db/src/comment_store.rs`; an agent's mutations are
  `knowledge.*` comment commands through `run_command`, and views
  re-read the comment models on `ModelsChanged`.
  - **`list_comments` returns hydrated typed context, not just the
    quote.** Each row is an `EnrichedCommentThread { thread, primary,
    context_chain, referenced }`. `thread` is the raw comment + message
    history; `primary` is the comment's anchor target resolved to a
    `RefSummary { kind, id, title?, detail?, body_excerpt? }`;
    `context_chain` is the nesting of page regions the selection sat
    inside (innermost→outermost — e.g. a file row highlighted under a
    commit yields `[commit …]`); `referenced` are the canonical refs
    found inside the selection itself (links + inline mentions). So a
    follow-up on a commit row arrives with the commit subject + diffstat
    (primary), the dashboard it lives in (chain), and any file the quote
    linked to (referenced) — the agent gets *what the highlighted thing
    is* in one call. Hydration runs through
    `oxplow_app::ref_resolver::{resolve_ref, resolve_refs}`, which resolves
    every canonical kind: `work_item` → title+status, `commit` →
    subject+diffstat, `file` → size + head excerpt, `dir` → entry
    count + names, `wiki` → title + lead, `finding` → kind + location;
    unknown kinds return a bare `{kind,id}`. The IPC
    `list_comments_for_target` stays raw — the renderer already has the
    page, so only the MCP surface pays the resolution cost. The same
    resolver is the single source of truth for backlink labels:
    `list_backlinks`/`list_outbound`'s `source_label` is just
    `resolve_ref(...).title`.

**Code tools** (`crates/oxplow-mcp/src/lib.rs`): `code_definition`,
`code_references`, `code_hover`, `code_symbols`,
`code_workspace_symbols`, `code_call_hierarchy`, `code_diagnostics` —
typed answers from the code-intelligence capability, over the same
shared language-server sessions the editor uses (`.context/lsp.md`). When no
server is configured for a language, the error is self-describing — it
names the suggested Mason package and both fix paths. The agent can fix
it: `run_command oxplow.lsp.install_server { package }` installs from the
Mason registry (picked up immediately by editor + tools) once a person
approves the proposal — what binaries oxplow downloads and runs is their
call (P8.A9) — and
`lsp_list_servers` shows what's configured/installed/running. Adding an
`lsp.servers` entry to `.oxplow/project.yaml` is the manual alternative for
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

Canonical id shapes (`.context/refs.md`): `wiki` the slug;
`work_item` `oxplow:tsk<n>`; `file` the bare repo-relative path; `dir`
the bare path with no trailing slash; `commit` the full sha; `finding`
the rowid as a string.
Each row carries `ref_type` so you can tell e.g. a commit's
`touched_file` edge from a wiki body's `wikilink`.

The wiki is read with SQL (P5.C4, [knowledge.md](./knowledge.md)):
`query_sql` over `v_knowledge_page` (a page's title, excerpt,
`outbound_refs` and `stale_ref_count`) and `v_knowledge_ref` (each
file ref's pin against the file's latest snapshot, `stale`), and the
site `search` tool (`kinds: ["wiki"]`) over bodies. The six MCP read
tools (`list_wiki_pages`, `search_wiki_pages`, `search_wiki_page_bodies`,
`get_wiki_page_metadata`, `list_stale_wiki_pages`,
`find_wiki_pages_for_wiki_page`) went with it. `wiki_ref_drift({ slug, path })` closes the loop: for one stale ref it
returns the unified diff between the snapshot the ref was pinned to and
the file's current on-disk content (`compute_wiki_ref_drift` in
`crates/oxplow-app/src/wiki_drift.rs`, via `similar`), so the agent reads
only the changed hunks instead of re-opening the file. `status` is
drifted | unchanged | not_a_ref | no_pin | binary; the diff is capped
(`truncated` flags it). The wiki-only
`find_wiki_pages_for_file` was removed in favour of `list_backlinks`
(below) — every cross-kind backlinks question goes through one tool
now. **Writes are commands** — `run_command knowledge.write_page` /
`delete_page` / `link` / `resync` ([knowledge.md](./knowledge.md)); the
write guard refuses an agent's Write/Edit into `.oxplow/wiki/`.

The watcher restates the page's row after each successful resync; the
UI re-reads on the `modelsChanged` that produces (there is no wiki event
of its own). The slug is the file stem of the touched
`.oxplow/wiki/<slug>.md`, and the `FsWatcher` debounce is 250 ms so
bursts (editor swap-saves, batched writes) coalesce into one resync.
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

**Wikilinks for file + commit + task references.** The skill instructs
the agent to write repo file references as `[[path/to/file.ts]]`
wikilinks, with optional `:line` suffix and `|display` override. Git
commits are written as `[[abc1234]]` (bare 7-40 char hex) or
`[[git:abc1234]]` — both resolve to the GitCommitPage. **Tasks are
`[[tsk42]]`** (always the `tsk` prefix — the GitHub `[[#42]]`/`#42`
form is *not* a ref: the extractor drops it and the renderer shows it
as a broken, non-clickable link). Backticks remain for code-ish
identifiers. The wiki renderer
(`apps/desktop/src/components/Wiki/MarkdownView.tsx`, `preprocessWikilinks`)
rewrites `[[ ]]` into clickable links — SHA-shaped targets become
`gitcommit:` links that dispatch through `onOpenCommit`; file-shaped
targets become `file:` links that open in an editor tab via
`onOpenFile`; `tsk<digits>` targets become `task:` links; bare slugs
route to wiki navigation; and any interior that matches no known ref
shape — plus any recognized ref whose *object doesn't exist* (deleted
wiki page / task) — renders as a broken, non-clickable link
(`BrokenLink`, `data-testid="broken-wikilink"`). Existence for wiki/task
is read from the client-side title caches (`useWikiRef`/`useTaskRef`,
which now surface a `missing` status). The reference
parser (in `crates/oxplow-db/src/wiki_page_store.rs`) already picks
paths out of `[[ ]]` because the bracket characters fall outside its
lookbehind,
so backlinks/freshness work without parser changes. The

**Link checker (write-command feedback).** `oxplow.effort.report` (over its
`summary`) and the `oxplow.knowledge.add_note` / `update_note` commands (through
`check_links_in`) run `oxplow_app::link_check` over the text they just
persisted and return a `link_warnings` array naming each invalid `[[…]]`
— unrecognized syntax or a dangling target — so the authoring agent
self-corrects in the same turn; `oxplow.work_item.create` / `oxplow.work_item.update`
check an oxplow item's body the same way (tsk775).
`knowledge.write_page` refuses instead, through the same synchronous core
(`check_links_in`). **Every kind the vocabulary knows is a link**
(tsk894): a typed `Reference` (task, wiki, file, dir, commit, finding) is
probed for existence; another known kind (an effort, another provider's
work item, a run) is valid as it stands; a plugin kind with a `resolve`
model (`v_ref_kind.resolve`) is valid when that model has its `ref`
(`[[pr:12]]` → `github_pr:12`). Only an interior matching no kind is
"not a recognized reference". A file is looked for in the **thread's
worktree** (the note's or work item's thread; the primary checkout when it
has none), and a file pinned to a revision (`[[src/a.rs@HEAD]]`) at that
revision (`RevisionGraph::has_file`), not on disk (tsk895). The shared classifier is `oxplow_domain::refs::classify_wikilinks`
(the single source of truth for "is this interior a real ref"), and
existence probes reuse the `ref_resolver` store/git/fs surfaces. The
`<wiki-capture-hint>` block injected on exploration UserPromptSubmits
(see "Wiki-capture is a UserPromptSubmit hint" above) auto-loads the
skill; the `/note` slash command at `.claude/commands/note.md`
triggers the same flow on demand.

## Collection command & skill

The `/oxplow:configure` command (asset `crates/oxplow-plugin/assets/configure.md`)
sets up the **collection** subsystem (see `.context/collection.md`): it has
the agent instrument the project's test tooling to emit standard-format
reports at stable paths, then records the `testing:` block and one report
collector per report (`collectors:` with `records:`) in
`.oxplow/project.yaml`. The standing `oxplow-collection` skill
(`crates/oxplow-plugin/assets/oxplow-collection.SKILL.md`) loads when a task
closes and on `/oxplow:configure`; it tells the agent to run the tests
before completing (so a report exists) and — critically — to **never parse
or report coverage numbers itself**, because oxplow parses the report
deterministically (`observed`). Both are wired in `write_plugin`
(`crates/oxplow-plugin/src/lib.rs`). The ingestion side (PostToolUse test
detector, the report collectors a detected run reads, `oxplow.collector.sync` for
one run by hand, `oxplow.test.record_run`, and the `list_effort_observations` /
`get_open_effort` MCP reads) is documented in `.context/collection.md`.
`get_open_effort({ thread_id })` answers "what is this thread's
currently-open effort?" — returns `{ open, effortId, workItem,
startedAt, hasStartSnapshot }` (`workItem` its linked work-item ref;
`open:false` with null ids when none): whether an
effort is open before a run is recorded, and whether its diff coverage has
a baseline (`hasStartSnapshot`).
Report parsing is **pluggable**: a report collector names a bundled parser
(`entry: oxplow:<junit|lcov|cobertura|jacoco|clippy|eslint>`, jq programs in
`crates/oxplow-collect-plugin`) or its own jaq / Starlark / exec script, no
recompile (tsk863).
When the PostToolUse hook detects a test run but no configured report was
refreshed, it returns a one-shot nudge via `hookSpecificOutput.additionalContext`
steering the agent to the report-emitting command. See the "Report-less-run
nudge" section in `.context/collection.md`.

### Nudge persistence

The PostToolUse nudges — the report-less-run nudge above and any
post-tool-use **advisory** that fires (kind `<extension>/<advisory id>`)
— are written by
the pump reactors (`collection`, `advisories.post_tool` —
`crates/oxplow-app/src/post_tool_reactors.rs`, P3.6) to the `agent_nudge`
table (`crates/oxplow-db/src/agent_nudge_store.rs`, see
`.context/data-model.md`) tagged with kind, the message, the trigger (bash
command), the turn and the **cause** (the `agent.tool.finished` event — a
redelivered event can't fire the same kind twice). **The persisted nudge is
the delivery:** `AgentContext::post_tool_context` settles the two reactors (≤2.5
s) and returns the thread's nudges with no `delivered_at`
(`take_undelivered`, which stamps them) — oxplow's own kinds first, then
advisories — so one that finishes after its hook answered reaches the agent
on the thread's next tool call. The ExitPlanMode ROLE CHANGE banner still
wins its call; nudges wait for the next. **One-shot marks are durable**
(`once_mark`, per effort or per thread): the report-less-run nudge fires
once per effort, and an advisory once per its `once_per` scope, across
restarts. **Every advisory hit is a nudge**, whatever its trigger: prompt
and turn-end hits are persisted too, and `AgentContext::prompt_context`
takes the thread's undelivered nudges like a tool call does — so a hint a
turn's end raised reaches the agent at the next prompt (or tool call),
stamped `delivered_at` when it does.

These are surfaced UI-side only (the agent never reads them back): an
"Agent Nudges" H2 section on the task page (after each effort's Metrics)
lists them, re-running when `ModelsChanged` names `v_agent_nudge`.
The point is a reviewer/human-facing record of "what oxplow told the agent
this effort" — previously the nudges were fully ephemeral. IPC + event wiring
is in `.context/ipc-and-stores.md` (Agent nudges).

## Token usage capture (from the hook transcript_path)

The PTY is opaque, but the hook payload oxplow already receives carries
`transcript_path` (the agent's session JSONL). oxplow parses it to give
per-effort + per-thread token visibility — the one place agent token
counts are observable. (tsk104; `crates/oxplow-app/src/token_usage.rs`.)

**Hook payload fields used.** From the raw Stop `payload_json`:
`transcript_path` (the JSONL file) and `session_id` (the cursor key).
For Claude, each `type=="assistant"` line carries
`message.usage.{input_tokens, output_tokens,
cache_creation_input_tokens, cache_read_input_tokens}` and
`message.model`.

**Flow (the `token_usage.turns` pump reactor, P3.7).** Nothing is parsed in
the Stop hook. The ingest logs `agent.turn.ended@2` with Claude's
`transcript_path`; `TurnTokensConsumer` (`crates/oxplow-app/src/token_usage.rs`)
reacts to it with `TokenUsageService::on_stop`, carrying a `TurnRecord`
(the turn, the effort it ran in, the event id). An ACP agent reports its
counts with the turn instead: the host puts them on the Stop body
(`TURN_USAGE_KEY`), they ride the event's `usage`, and the reactor records
one row keyed by the event (`agent_token_usage.cause`), so a redelivery
counts it once. Transcript rows carry no `cause` — one chunk can hold
several turns, so the cursor (step 5) is their redelivery guard (tsk498).
The transcript path:
1. Pull `transcript_path` from the payload; resolve the thread's
   `AgentKind` + stream.
2. Read the persisted per-session cursor (`agent_token_cursor`), seek to
   it, and read only the COMPLETE lines of the tail (everything up to the
   last newline — a half-written final line is left for next time).
3. `parse_turns(kind, tail)` splits the new tail into one `Turn` per agent
   turn — each carrying the human-authored **prompt** that opened it plus
   the summed usage + `model` of the assistant messages that answered it
   (tsk143). A turn begins at a genuine user prompt and runs until the next
   one; tool-result user messages (the harness's continuation lines) fold
   into the current turn rather than opening a new one. **Pluggable per
   agent kind:** Claude implemented; Codex/Opencode return `[]` (their
   transcript formats differ — opencode surfaces its own `$cost` — and are
   wired later). (`parse_usage_delta` still exists as the whole-chunk sum,
   but `on_stop` records per-turn.)
4. Attribute each turn to the effort the oxplow turn ran in (the event's
   effort anchor; without one, the thread's open effort — nullable: a Stop
   can land with no open effort) and persist one
   `agent_token_usage` row per turn (provenance `observed`, with the actual
   per-turn `model` and `prompt`). A chunk spanning several prompts (a brief
   plus follow-up nudges, or an interrupt-and-re-prompt) yields one row per
   prompt — so an effort review shows *every* thing that was asked, not just
   the first.
5. Advance the cursor to the new offset **in the same transaction as the
   rows** (`SqliteTokenUsageStore::record_batch`), so a redelivered turn
   never reads the same bytes twice; rows carry `turn_id` (a view of
   `v_token_usage` re-runs on `ModelsChanged`). The `oxplow.turn`
   facts capture is keyed by the event (`turn-tokens:<event id>`).

**Prompt capture is pure OBSERVATION (tsk143).** The prompt text is read
out of the same transcript walk oxplow already does — it is the exact thing
the human typed into the real `claude`/`codex`/`opencode` CLI. There is NO
second input box, NO agent-input path, and NO MCP "send prompt" tool; we
only RECORD the user-authored prompt, never generate or send one (the same
boundary as `forward_terminal_input`, which stays human-keystrokes-only and
non-MCP). The prompt is stored locally in the effort DB (`agent_token_usage.
prompt`) like every other effort artifact — same privacy posture as the
token counts, file lists, and coverage already kept there.

**Live capture (optional follow-up).** v1 sources prompts from the at-Stop
transcript walk — a single capture path with the exact stored text, but the
prompt only appears once the turn finishes. A `UserPromptSubmit` hook could
surface the prompt LIVE as the user types, at the cost of a second capture
path to reconcile against the transcript walk. Left out of v1 unless it
turns out trivial.

**Bootstrap (first capture for a session).** When `cursor()` returns
`None` — a fresh daemon, or the first Stop after attaching to an
already-long transcript — `on_stop` does NOT ingest from offset 0.
Reading from 0 would lump the entire prior transcript into one
`turns:1` row attributed to whatever effort happened to be open
(tsk142). Instead it **seeds** the cursor to `complete_offset()` (just
past the last complete line currently in the file) and records nothing,
returning `Ok(None)`. Only turns appended *after* oxplow started
watching are attributed — the deliberate tradeoff is that the single
turn in flight at the very first Stop is not counted (we have no record
of the pre-turn offset to isolate it). A genuine `Some(0)` cursor (a
session oxplow tracked from byte 0) still takes the normal ingest path,
so this is purely a `None`-vs-`Some(0)` distinction.

The cursor is **persisted** (not in-memory) so a daemon restart never
re-sums already-recorded usage. Display is **tokens-only** for now; the
stored `model` lets cost be layered on later. Everything reads it through
`v_token_usage` (which carries each turn's `prompt`, V82): the
oxplow-bundled `task-tokens` lens in the task page's `work_item.detail.body` slot
(a total plus a per-turn log of prompt, model and tokens across the task's
efforts), the `thread-tokens` strip in the Work panel's `thread.plan.header` slot, and
the `usage` lenses. Tables: see `.context/data-model.md`
(`agent_token_usage` / `agent_token_cursor`).

## Write guard

Non-writer threads share the writer's worktree (same checkout, separate
agent panes). Letting their agents write would corrupt the writer's
in-progress changes.

- **Hook enforcement.** The shared agent policy (above) denies `Write`,
  `Edit`, `MultiEdit`, `NotebookEdit` (and, for ACP agents, delete/move)
  from any non-`active` thread; the reason comes from
  `write_guard::read_only_reason` (`crates/oxplow-runtime/src/write_guard.rs`).
  "The worktree" is the thread's own stream's (see "Agent policy"
  above). A path in another stream's worktree is denied for every
  thread. When the tool's target path resolves OUTSIDE every stream's
  worktree AND outside the project's `.oxplow/`, the call is allowed (e.g. writing to
  `~/.claude/plans/foo.md`); the deny message names the specific
  absolute path. Containment checks live alongside the write guard
  in `crates/oxplow-runtime/` and reuse `AppLayout` from
  `crates/oxplow-app/src/lib.rs`.
- **Wiki pages are written by command.** A direct write to
  `.oxplow/wiki/` is denied for every thread (`write_guard::wiki_page_reason`);
  pages go through `knowledge.write_page`, which any thread may run.
  Other `.oxplow/` paths (`local.sqlite`, `snapshots/`, `runtime/`) stay
  blocked.
- **Bash isn't classified.** A non-writer's shell commands aren't
  checked; the session context's Access line says the thread is
  read-only.
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
spawn path (`open_terminal_session`) calls
`ThreadService::selected_or_active(&stream_id)`, which falls back from
the user's explicit selection → the writer (active) thread → the first
queued thread. This guarantees `OXPLOW_THREAD_ID` and the
visible `<session-context>` note's thread line are populated for any
stream that has at least one thread (boot seeds a thread, running the project's default agent, for
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
in `.oxplow/project.yaml` — default is `true`.

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

## Decisions fed back to the agent (tsk298)

Decisions the agent records (`oxplow.effort.record_decision` → `v_decision`) are fed
back to it in two places. Only `provenance = 'recorded'` rows: decisions
oxplow *inferred* after the fact ([ai-providers.md](./ai-providers.md))
are guesses for the reviewer, never presented to the agent as its own.

**On `UserPromptSubmit`**

- The open effort's decisions, capped at 15 and most recent last, ride
  the same `additionalContext` as the session-context block. They're
  built by `oxplow_app::reasoning::effort_decisions_block`.
- They're deduped with the same per-session state (the key is
  `<session_id>#decisions`). So they're sent on the first prompt of a
  session and again only when they change.
- `SessionStart` (startup / resume / clear / compact) clears that key, so
  **after a compaction the agent gets its earlier decisions back**. That
  is the point: agents otherwise forget their own choices across
  compaction.

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

`config.agentPromptAppend` (loaded from `.oxplow/project.yaml` via
`loadProjectConfig` in `crates/oxplow-config/src/lib.rs`) is concatenated into every
agent's system prompt by `buildBatchAgentPrompt` (in `crates/oxplow-runtime/src/lib.rs`). The
Settings modal (`apps/desktop/src/components/SettingsModal.tsx`) reads/writes this
via `runtime.setAgentPromptAppend` which calls `writeProjectConfig` to
persist back to YAML.

A new value applies to **agent sessions started after Save** — existing
sessions keep the prompt they launched with.

After `agentPromptAppend`, `buildBatchAgentPrompt` also appends:
- `# Stream instructions` + `stream.custom_prompt` if the stream has a
  non-empty custom prompt (set on `StreamSettingsPage`,
  persisted to `streams.custom_prompt` — see data-model.md v18).
- `# Thread instructions` + `thread.custom_prompt` if the thread has a
  non-empty custom prompt (set on `ThreadSettingsPage`,
  persisted to `threads.custom_prompt` — see data-model.md v18).

These are the last sections before the prompt is finalized, so they can
provide finer-grained overrides without displacing earlier context.

## Agent status

**Status is per agent session.** Each session has its own: its newest
`agent.status.changed` (anchored to the session) and the status derived
from its own activity; a thread's activity no session claims has one
too. A thread's status is its sessions' **roll-up**, and a stream's is its
threads': `oxplow_domain::agent::roll_up_status`, the desktop's
`rollUpAgentStatus` and the `v_agent_status` model share one rule and one
truth table (`crates/oxplow-domain/fixtures/agent_status_rollup.json`;
the model is tested against the Rust rule over every pair). The ranking
is what the person owes first: **awaiting > stalled > working > waiting**
(`awaiting_user > stalled > running > error > stopped > idle`). A dead
turn (`stalled`) means the person owes the next move, so it outranks work
in flight — a stream whose one thread died beside a busy one must still
say so; the old stream dot that ranked working first hid it.
`v_agent_session_status` holds each open session's logged status;
`AgentStatusChanged` carries the session, the UI keeps statuses by session
(`sessionStatusKey`) and rolls them up per thread (`threadStatuses`) and
per stream.

`derive_session_status` (`crates/oxplow-app/src/agent_status_derive.rs`)
reduces a session's recent activity into one of two states: `working` or
`waiting`. **The input is the event log** (P3.9): the session's newest 200
`agent.*` events (`recent_activity`, via `SqliteEventLogStore::recent_in_session`),
each read as an `Activity` (`activity_of`) — `prompt.submitted` (every
prompt, re-prompts too), `tool.requested{allowed}` (a refused request never
runs, so it opens no tool), `tool.finished`, `turn.ended` (completed vs
interrupted/restart), `session.started`, and `status.changed`
(`awaiting_user` parks the thread until a prompt or another status moves
it). Hook ingest logs it from what it sees: a Stop whose final message
ends in a question (its last line is the detail), a pending
`AskUserQuestion` / `ExitPlanMode` (until its PostToolUse), and Claude's
`Notification` hook with `notification_type` `permission_prompt` or
`elicitation_dialog` (until the tool's PostToolUse); ACP permission
cards log it through `HookIngestService::set_status`. It
survives a restart, unlike the in-memory ring it replaced. The stall watch
and `list_agent_statuses` derive from the same reads. The runtime
recomputes on every tool hook and emits `agent-status.changed`. The UI shows it as a colored dot on each thread
tab — yellow pulsing for `working`, red for `waiting`. The two states
encode the only signal a tab indicator actually needs: is the agent
burning cycles, or does the user owe the next move? Brand-new threads,
completed turns, a session (re)start and user interrupts all collapse to
`waiting`.

**Subagent-in-flight carve-out.** The reducer counts unreturned subagent
tool calls (`SUBAGENT_TOOLS`: `Task`, `Agent`; requested + / finished -). When a `stop` event arrives
while the count is >0, status stays `working` instead of flipping to
`waiting`. Without this the tab icon would flip the moment the parent
paused for a subagent, even though the subagent was still doing real
work. The status flips to `waiting` once the final `Task` PostToolUse
returns and a subsequent `stop` lands. See the original ticket history.

**User-input-pending carve-out.** Two Claude Code built-in tools block
the turn waiting on a human answer: `ExitPlanMode` (the plan-approval
prompt — "should I implement this plan?") and `AskUserQuestion` (the
clarifying-question prompt). Each fires `PreToolUse` when the agent
invokes it, but the matching `PostToolUse` only arrives once the user
answers. Until then no `Stop` hook fires either — the agent is
genuinely waiting on the user. `derive_session_status` counts unreturned
calls to either tool (`is_user_input_tool` in
`crates/oxplow-app/src/agent_status_derive.rs`) and, if the count is >0
at the end of replay, overrides the derived state to `AwaitingUser` so
the dot shows "Waiting for input" instead of staying yellow — and the
stall threshold is exempted (waiting indefinitely is legitimate).
**tsk128:** `AskUserQuestion` was previously *not* tracked, so an agent
parked on a clarifying question stayed `Running` and degraded to
`Stalled` ("agent stopped responding mid-turn") — read as a death and
mis-triggering a re-dispatch. Both tools are now handled identically.

**User-interrupt synthetic event.** Claude Code does not reliably fire
the `Stop` hook when the user cancels a turn with Escape (or `Ctrl-C`):
the in-flight tool's `PostToolUse` is dropped and no `Stop` lands, so
the reducer would otherwise stay `working` until the next prompt. The
runtime's `sendTerminalMessage` watches the websocket input stream and,
when it sees a bare `\x1b` or `\x03` byte (interrupt heuristic in
`terminalInputIsInterrupt`, `crates/oxplow-runtime/src/lib.rs`), ingests a synthetic
`Interrupt` hook for the thread that owns the terminal session. The
ingest closes the open turn as interrupted (`agent.turn.ended{interrupted}`),
which the reducer treats as a reset: the open tools and pending subagents
are cleared and the thread reads as not working. The synthesis only fires when the thread is
currently `working` so a user idly tapping Escape at a prompt is a
no-op. Multi-byte ESC sequences (arrow keys, etc.) are explicitly
filtered out — only the bare interrupt byte counts. See the original ticket history.

**Stall / death detection (API-error deaths).** Claude Code emits *no*
hook at all when a turn dies on a transient API error (socket closed
mid-stream) or a model-unavailable error ("Claude Fable 5 is currently
unavailable") and the process drops back to its prompt — observed live
as a dot stuck on `working` for ~1h while the queue silently stalled.
Nothing event-driven can catch that, so the derivation is time-aware:
`derive_session_status(events, now)` degrades a derived `Running` whose
newest hook event is older than its silence threshold to a derived-only
`AgentStatusState::Stalled` (never persisted to the agent_status
table). **Two thresholds (tsk130),** chosen by whether a tool call is
still open (any `PreToolUse` without its matching `PostToolUse`):

- `AGENT_STALL_AFTER_MS` (15 min) when a tool is open — a single Bash
  can legitimately run silently up to its 10-min max, so wait it out.
- `AGENT_DEAD_AFTER_MS` (5 min) when nothing is open — silence right
  after a prompt or between tool calls means the next model call died,
  caught promptly instead of the old uniform 15 min.

**PTY output is a second liveness signal (tsk141).** Hooks are sparse
*within* a turn: a single long turn (observed live at ~1h5m) streams
tokens to the terminal for many minutes while emitting **no**
Pre/PostToolUse between tool calls, so a frozen hook log alone reads as
death even though the agent is plainly working — the inverse of the
tsk130 death case. `derive_session_status_with_activity(events,
last_output_at, now)` therefore measures silence from the *later* of
the newest hook event and `last_output_at` (the thread's most recent
PTY output). An agent still writing to its PTY stays `Running`
regardless of how stale its last hook is; only when **both** signals go
quiet past the threshold does the turn degrade to `Stalled` — so tsk130
death detection is intact (a dead turn stops emitting output too, and
output older than the threshold can't revive it). `derive_session_status`
is the hook-only wrapper (`last_output_at = None`), used where a hook
just arrived (so the log is fresh by construction); the watchdog uses
the activity-aware form. Liveness is tracked by
`output_activity::OutputActivity` (a per-agent-session last-output
timestamp, never persisted): the terminal forwarder
(`terminal_sessions.rs`) stamps it on every output burst for a pane
spawned for an agent session (`attach_or_create_for_agent` with an
`AgentPane`; shell panes contribute none), the ACP host on its agent's
activity, and `AgentStallWatch` reads it. The single shared
instance lives on `Services::output_activity`.

The `AwaitingUser` override (ExitPlanMode / AskUserQuestion — see the
user-input-pending carve-out) is exempt from both: waiting on the user
indefinitely is legitimate. Because no hook will ever arrive to trigger
a re-derive, `AgentStallWatch`
(`crates/oxplow-app/src/agent_stall_watch.rs`, spawned from `boot.rs`)
re-derives every open session once a minute and pushes
`AgentStatusChanged { agent_session_id, state: Stalled }` so the renderer's dot recovers
on its own. It raises nothing about in_progress tasks: a task left in
progress while its agent is idle is normal (nothing marks work done, and
the agent may be waiting on the person — `.context/work-tracking.md`).
The renderer collapses status as running → `working`, stalled →
`stalled` (red pulsing dot, labeled "agent exited or errored mid-turn —
re-run"), everything else → `waiting`.

## Snapshot tracking

The runtime keeps a content-addressed history of each worktree so the
UI and the agent can see what changed per turn and per effort without
relying on git. The store and its guarantees (content identity,
`tree_hash`, the `snapshot_op` operation log, one-transaction takes) are
in [data-model.md](./data-model.md) "snapshot + file_snapshot"; this is
when takes happen.

- **Dirty set.** A per-stream in-memory set of changed paths, fed by the
  workspace fs-watcher and the PostToolUse hook. A take drains it; only
  those paths are re-stat'ed, everything else carries forward.
- **Turn end (P2.3).** When a turn closes — Stop, or an interrupt; every
  harness (Claude hooks, ACP, opencode, codex) reaches this through
  `HookIngestService::ingest` — `turn_snapshots::CaptureTurnSnapshots`
  runs a `turn_end` take anchored to the turn, its thread and the
  thread's single open effort, and `agent_turn.snapshot_id` records the
  snapshot the turn ended at. **What the turn changed is
  `agent_turn.start_snapshot_id → snapshot_id`** — the start is recorded
  in the turn's open transaction (the stream's current snapshot). Not the
  take's op parent: other takes during the turn (the close's
  `effort_end`, a commit's `git_refs`, another thread's turn end) move
  that, and the usual flow — edit, close the task, Stop — would read as
  a turn that changed nothing (V99, tsk438).
- **Order and cost (tsk441).** The Stop/interrupt handler closes the
  turn, sets the agent status (so the UI isn't held on "running"), then
  takes the snapshot. One take per Stop, for the newest turn this call
  closed — `AgentTurnStore::close` reports whether it closed anything,
  so a repeated Stop takes nothing. The 300 ms pre-drain runs before the
  take lock (queued takes wait in parallel), and a take with no rows
  skips the git branch/status/HEAD probes.
- **The budget.** The Stop hook waits at most `snapshotTurnBudgetMs`
  (default 2000, min 100; a `oxplow.config.set` key, not human-only). A slower
  take is never aborted: it finishes in the background and its op and
  `snapshot.taken` record `over_budget` (plus a warn log). The clock
  starts before the per-stream take lock, so waiting behind another take
  counts.
- **Turn events.** `agent_turn` open/close log `agent.turn.started@1` /
  `agent.turn.ended@1 { outcome: completed | interrupted | restart }` in
  the same transaction, anchored to turn, thread and stream.
- **Quiet period (P2.4).** Human edits between turns get a snapshot of
  their own: each fs-watch change arms a deadline (`DEFAULT_QUIET_PERIOD`,
  3 s, debounced); when it fires with paths still dirty and no turn open
  on any thread of the stream (`SqliteAgentTurnStore::
  stream_has_open_turn`, injected as the registry's `OpenTurnProbe`), a
  `quiet` take records them. While a turn is open it yields — that
  turn's `turn_end` take captures the same edits. Entries the settle gate
  deferred re-arm it. Only the watcher arms it: the boot sweep and
  explicit `mark_dirty` callers are followed by their own take. The
  trigger runs beside the watcher (`spawn_watcher` starts both) and ends
  on `shutdown()`.
- **Effort brackets.** Entering `in_progress` takes an `effort_start`
  snapshot recorded on `effort.start_snapshot_id`; leaving it takes
  an `effort_end` one on `end_snapshot_id` (both anchored to the effort).
  An unchanged tree records an op on the current snapshot rather than a
  new row, so `end_snapshot_id` is set whenever the stream has any
  snapshot (null ⇔ effort in progress); the effort-lifecycle pump
  consumer falls back to the start snapshot on a capture failure. The
  take is automatic: an open or close logs `effort.opened` /
  `effort.closed`, which the pump consumer pins — agents never flush
  explicitly.
- **Boot.** The primary stream's startup sweep (`enqueue_startup_diff`,
  then a `startup` take) records what changed while oxplow was down.
- **HEAD moves.** The git-refs listener runs a `git_refs` take, then —
  on a clean tree — a `head_moved` re-stamp (`vcs.head.moved@1`).
- **Effort-level diffs** are `diff_snapshots(start, end)` (content
  identity); the task page lists a task's efforts from `v_effort`
  (`workItems.readTaskEfforts`).

## Per-effort write log

An effort's files (`effort_file`, see data-model.md) are recorded as the
work happens, never declared by the agent.

**Claimed: structured writes (a pump reactor).** Every structured write
tool — `Edit` / `Write` / `MultiEdit` / `NotebookEdit` — claims the file
it wrote for the effort it was written in. The `effort.claim` async
consumer (`crates/oxplow-app/src/tool_call_reactors.rs`) reacts to
`agent.tool.finished`; the ingest already made `path` relative to the
thread's own tree (its stream's worktree), and an absolute path (outside
it) is never claimed. It calls `EffortService::claim_effort_file` with the
event's **effort anchor**, so a claim that lands after that effort closed
(the reactor ran late) still goes to it; with no anchor the thread's open
effort takes it. The claim is idempotent (`record_file` is `INSERT OR
REPLACE` keyed on `(effort_id, path)`, `source = 'claimed'`), and replaces
an observation of the same path. The same event feeds two sync consumers:
`tool_call.project` (the `agent_tool_call` row, one per event by
`event_id`) and `wiki.attribution` (an edit of an indexed
`.oxplow/wiki/<slug>.md` marks the page touched by the thread; a page the
watcher hasn't indexed yet is skipped rather than dead-lettered).

**Observed: everything else a turn changed.** `Bash`, codegen and
formatter writes name no file. The `effort.observe` consumer records
each path the turn's snapshot bracket changed as `observed` on the
effort holding the turn (see "MCP tools" above for the rules: another
thread's claim wins, a shared unclaimed change goes to both, changes
between turns go to none). Nothing is reconciled at close.

**Redo nudges are gone.** The UserPromptSubmit `<recent-done-reminder>`
and the MCP `create_task` `redoHint` (a soft warning when a new row was
filed on a thread with an agent-authored `done` item closed in the last
10 minutes) no longer exist — the latter went with the MCP task tools
(P8.A10), and `oxplow.work_item.create` carries no such check. What remains is
the PreToolUse filing directive, whose second door is "fix/redo of a
recently-closed done item → `oxplow.work_item.transition` it to `in_progress`".

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
  the effort has ≥1 `effort_file` row (claimed or observed) the pair-diff
  is filtered to those paths; if it has 0 rows we fall back to the raw
  pair-diff — better to over-report than silently show empty.

`get_effort_files` is implemented in
`crates/oxplow-tauri-ipc/src/commands/effort.rs` over the
`EffortStore` and `SnapshotStore` and wired to IPC like the other
snapshot reads.

## Task lifecycle

Tasks (`task` rows) are the user-visible primitive. The Work
panel's in_progress bucket is driven purely by `task` rows —
there are no synthesized "live turn" rows, no auto-file /
auto-complete / adoption. Per-effort attribution and snapshots are
anchored to `effort`, which opens and closes in the same transaction as
a task's status change (`oxplow.work_item.create` / `.update` / `.transition`,
all audited to the actor); work tracked outside oxplow brackets itself
with `oxplow.effort.open` / `oxplow.effort.close` (any thread; [commands.md](./commands.md)). No edit waits on an open effort.

Agent rules (mirrored verbatim in the project root `CLAUDE.md`):

- **Start of work** — file an `in_progress` task before editing
  project files.
- **Pivot** — before starting a different task, dispose of the
  current one: stopping for good → `canceled`; switching but coming
  back → `ready`; can't proceed → `blocked`. Then start the new task
  `in_progress`.
- **Defer/batch** — create the new task as `ready` with a short note
  capturing the ask. Flip to `in_progress` when actually picked up.
- **Merge** — update the current task's title / description
  when new info refines it. No new row.
- **Q&A** — pure conversational asks need no task. Tasks are for
  independent, completable work.
- **Persist across turns** — if a turn ends with work mid-flight
  (asked a question, Stop fired before finishing), the task stays
  `in_progress`. Only `done` when the work is actually shipped.

## Related

- [data-model.md](./data-model.md) — the queue the agent operates on.
- [ipc-and-stores.md](./ipc-and-stores.md) — how to add new MCP tools
  and the underlying storage.
- [git-integration.md](./git-integration.md) — `gitCommitAll` for the
  Files-panel commit dialog (user-driven).
