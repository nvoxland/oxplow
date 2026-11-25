# Agent execution model

How oxplow and a coding agent (Claude Code, Codex, opencode) meet: how a session is launched, what every agent is told, the
tools it calls, what oxplow observes of it, and the rules that keep one
thread from clobbering another's worktree. How oxplow turns that record
into requests, files and hints is [work-tracking.md](./work-tracking.md); the commands an
agent runs are [commands.md](./commands.md).

## Key invariant

**The runtime never prompts the agent.** An agent is steered only by:

1. The system prompt oxplow adds at launch ("What every agent is told").
2. What oxplow answers a hook: context on a prompt or after a tool call,
   or a write-guard deny before one. A Stop is never refused
   ([work-tracking.md](./work-tracking.md) "No gates").
3. What an MCP tool answers when the agent calls one.

Every hook answer goes through `respond()` in
`crates/oxplow-control-plane/src/lib.rs`, which has the hook's harness
render a `HookAnswer` and always answers `200` (an ack is `{}` for every
built-in). Claude Code prints a "non-blocking status code" warning into
the person's terminal for any other status, an empty `202` included, so
ingest failures and timeouts ack `200 {}` too, with the cause logged
server-side. Never return a bespoke status from a hook branch.

**oxplow never drives an agent.** Nothing prompts one on its own: a PTY,
or an agent signed in with a plan, is licensed for interactive use. The
unbuilt `Drive` interface, the `action` session kind and the
`programmatic` harness feature were removed on 2026-11-20 (V53 dropped
`agent_session.kind`); prompting an agent from oxplow, if it ever comes,
is a new decision.

### No synthesized agent terminal input (no automation)

**The agent's input has one source: the person.** oxplow never
generates, injects or auto-answers agent input. The agent makes its own
tool calls and is driven by a human, never by oxplow typing at it.
Synthesizing input would be automating the agent CLI, which risks its
(Claude Code / Codex / opencode) license terms.

- The renderer's terminal (`terminal/TerminalSurface.tsx` for the keys,
  `terminal/useTerminalSession.ts` for the session) pipes the person's
  keystrokes, paste, scroll and resize to the PTY as frames on the
  daemon's `/events` socket (`terminalInput` → `sendSocketFrame`), which
  the daemon hands to `terminal_input` (→ `terminal_sessions.enqueue_input`
  → the terminal's queue → `send` → `pty.write`; `.context/terminal.md`
  "Paste path and input ordering"). It is the human-input transport, not
  an agent-messaging API, and it must stay: the PTY is on the daemon, and
  keystrokes reach it only this way.
- Terminal input is UI-only: it isn't a command at all (no `/ipc` route,
  no Tauri command, nothing on the MCP surface), so an agent can't type
  at itself or a peer.
- The Codex observer sends no prompt: the person types in Codex's own
  TUI, and oxplow only injects context between turns ("The observer
  machinery").
- The agent input bus (`agent-input-bus.ts`: a page action's draft, Repair
  with the Agent) carries a person's gesture: the visible pane pastes the
  text. Nothing drags into an agent; what it should read goes in the
  context basket.
- Steering (session context, nudges) reaches the agent through hook
  answers its own harness injects, never through the terminal.

**Guards** (each fails the build):
- `apps/desktop/src/no-agent-input-automation.test.ts` confines
  `terminalInput` and `{type:"input"}` construction to
  `terminal/useTerminalSession.ts` and the `api.ts` facade, and
  `sendSocketFrame` to the facade and the transport.
- A Rust source scan in `crates/oxplow-rpc/src/commands/terminal.rs`
  asserts the only production caller of `terminal_sessions.enqueue_input(`
  is the `terminal_input` core (the queue's drain task is the only caller
  of `send`).
- `agent_observer/guard_tests.rs`: no production code starts a Codex turn, and
  context reaches a Codex thread only from `agent_observer/codex.rs`.

### What we can't do from oxplow hooks

- **Hooks add context; they can't remove it.** A harness's own
  `<system-reminder>` blocks (Claude Code's "the task tools haven't been
  used recently") can't be edited out. A counter-instruction would leave
  both visible, which is worse ("Harness-injected system-reminders").
- **The first turn needs a person's prompt.** "Never prompts" holds for a
  cold start too: an idle agent stays idle when work is filed. A person
  sends the first prompt.

## The three seams

An agent session runs a **harness** (`AgentHarness`,
`crates/oxplow-domain/src/agent/harness.rs`). oxplow meets the agent at
three seams:

| Seam | What crosses it | Where |
|---|---|---|
| Launch | The process or thread: its command or Codex thread request, env, runtime files, system prompt, resume | `AgentHarness::launch` → `LaunchSpec::{Pty, CodexAppServer}` ("Launching the agent") |
| Observe | Hooks, tool calls, transcripts and token telemetry, mapped into oxplow's vocabulary and logged as `agent.*` events | `tool_use`, `prompt`, `subagent`, `turns`, `token_readings`, `render` ("Observing") |

The agent's MCP tools ("The agent's tools") are the one channel the agent
itself starts.

## What every agent is told (the text contract)

Every agent session, whatever its harness, is told the same things; a
harness only delivers them.

| Piece | What | Where it comes from |
|---|---|---|
| Preamble | Four lines: oxplow observes work in the background, use your normal project workflow, read oxplow URIs in the person's message with `oxplow_read`, and see the `oxplow-runtime` skill for commands and project data | `oxplow_agent_text::WORKING_IN_OXPLOW` (`crates/oxplow-agent-text/assets/working-in-oxplow.md`) |
| Launch prompt | The preamble; the session context only for a harness without `prompt_context`; the thread's and stream's custom prompts; `agentPromptAppend`; the testing hint (`testing.agentHint`); the skill index only for a harness without `runtime_text` | `agent_prompt::assemble_system_prompt(config, stream, thread, around, harness, skills)`, given the session's declared harness (`crates/oxplow-app/src/agent_prompt.rs`) |
| Skills | Core's (`oxplow-runtime`, `-knowledge`, `-mermaid`, `-collection`, `-metrics`, `-extension`, `-codebase`) and the active extensions' (one with `when:` only while its extension's setting holds it, `extensions-commands.md` "Skills") | `AgentText` (`oxplow_agent_text::core_text` + `capabilities::agent_text`, which reads the settings that offer skills) |
| Slash commands | Core's (`configure`, `new-metric`, `note`: keep what was just worked out as a knowledge page) and the active extensions' | Same |
| Session context | Stream, worktree, branch, thread, the other live agents in the worktree | Prompt-time context |
| Extension context | What an enabled extension says the agent should know now (`context:`), each as `<context from="<namespace>/<id>">`, once, then on change; 1,500 characters each, 3,000 in all | Prompt-time context (extensions-commands.md "Agent context") |
| Nudges | Advisory hints, the collection nudge | Prompt- or tool-time context ("Nudge persistence") |
| Server instructions | What oxplow knows, writes are commands, answer with `oxplow_find_page` / `oxplow_show_page` | MCP `get_info` ("The agent's tools") |

**A project's instruction files are the harness's own business.** oxplow
never reads or forwards `AGENTS.md`, `CLAUDE.md` or any other: each agent
reads its project's files as it does outside oxplow, and nothing oxplow
tells an agent goes through them (`the_prompt_carries_no_project_instruction_files`).

**Skills are short.** A core `SKILL.md` is at most `SKILL_BODY_LIMIT`
(4 KB, a test), because loading one costs the whole file. Detail lives in
`assets/skills/<name>/references/<ref>.md`, which `SKILL.md` routes to
(what each holds, when to open it). A harness that ships skill files
writes the references beside `SKILL.md` (`shared::write_skills`); one that
doesn't gets the index and reads a body with `oxplow_get_skill { name,
reference? }`. Each rule has one home skill (wikilinks:
`oxplow-knowledge`; waiting on the person: `oxplow-runtime`;
foreground tests: `oxplow-collection`; `v_dataset`/`oxplow_query_sql`:
`oxplow-extension`); the others point to it by name. No skill text names
a task id or `.context/` (tests).

**Two context channels, declared.** What oxplow returns to a hook reaches
the agent only if its harness carries it there. A harness declares the
channels it has as `agent_harness` features: `prompt_context` (context
returned to a prompt hook reaches the agent with that prompt) and
`tool_context` (context returned to a tool hook reaches it before its next
step). `AgentContext::prompt_context` / `post_tool_context`
(`crates/oxplow-app/src/agent_context.rs`) take the session's harness and
give nothing for a channel it lacks (`agent_context::carries`; a hook from
no session is answered), so nothing meant for its agent is taken: nudges
wait in the store. Claude Code and opencode declare both; Codex declares
`prompt_context` only, delivered as items injected into its thread's
history between turns ("Codex: its TUI on a thread oxplow observes",
"Context between turns"), since its TUI owns the prompt.

**Once, then on change.** On every prompt, `prompt_context` builds the
`<session-context>` block and sends it only
when it differs from what that agent session was last sent (`should_emit`,
keyed by the harness's session id). Each extension's agent context
(`context:`) follows the same rule, per contribution: right after the
session context, its block goes only when it differs from the one that
session last got from it (`would_emit_extension_context`, keyed by
session and `<namespace>/<id>`). It is not a nudge: no ledger row, no
muting, its own budget (`CONTEXT_BUDGET`, `CONTEXTS_BUDGET`). The first
prompt of a session always carries it, which is why the launch prompt
leaves the session context out for a harness with `prompt_context`.
Between them rides the thread's undelivered nudges, prompt advisories
included. `injectSessionContext: false` in `.oxplow/project.yaml` turns
the session-context block off.

**Compaction.** A compaction or a fresh start forgets what was sent
(`AgentContext::reset_session`): on `PreCompact` (Claude Code's; its HTTP
`SessionStart` never arrives), on `PostCompact` (opencode's bridge posts
only that one, after the fact) and on `SessionStart`, so the next
prompt carries the session context and every extension context again. The baselines
are in memory: a daemon restart costs at most one repeated block.

A compaction is also logged, by every harness, as `agent.context.compacted@2`
(`thread`, `turn` when one is open, `trigger`, `summary`): Claude Code's
`PostCompact` (`trigger`, and `compact_summary`, kept as event content and
read as `event:<id>#summary`), Codex's finished `contextCompaction` item
(through the chat host; it names neither), and opencode's
`session.compacted` (through the bridge; it names neither). The log is all
that opencode's compaction records besides the reset on its `PostCompact`. Codex's
compaction also resets the session's baselines (`ObserverHost::compacted`),
and the context is injected again once no turn runs. Thread activity
(foundation) counts a turn's compactions. `v_turn_activity.compactions` counts them per turn (a compaction between turns belongs to none), beside the turn's `deliveries` and `person_prompts`.

**Who else is in the worktree.** A stream's threads share its worktree,
and any number of agents may work there at once — in one thread or
several. The block names the others, so an agent knows it isn't alone:
`- **Also in this worktree:** <name> (`<harness>`, thread <title>
(`<thrN>`)); …`, one line, left out when there are none
(`agent_context::others_in_worktree`, `build_session_context_block`). An
agent is in it while its session is open on an open thread of the stream
and its recorded pid is alive (`daemon_supervisor::process_alive`); it is
named by `v_agent_session.display_name` (read through the SQL gateway, so
the view stays the one place a session is named), and the asking session
is left out by its row id (the hook's `agent_session_id`, the chat host's
session, the launch's row). Nothing in the line is for the agent to act
on: no pid, no session id. An agent arriving or leaving changes the
block, so the next prompt carries it; Codex has no prompt, so the change
is injected as it happens, while no turn runs.

### Per-harness delivery

| | Claude Code | Codex | opencode |
|---|---|---|---|
| Launch prompt | `--append-system-prompt` | The thread's `developerInstructions` on `thread/start` / `resume` / `fork`, which Codex puts at the head of its developer message | A per-session instructions file (`.oxplow/runtime/opencode-plugin/prompts/<ses>.md`) named by `OPENCODE_CONFIG_CONTENT`'s `instructions` |
| Skills | Plugin dir `.oxplow/runtime/claude-plugin/skills/` (`--plugin-dir`) | The worktree's `.agents/skills/<name>/` | `.oxplow/runtime/opencode-plugin/skills/`, named by the config's `skills.paths` |
| Slash commands | Plugin `commands/`, run as `/oxplow:<name>` | A skill per command, `oxplow-<name>`, run as `$oxplow-<name>` | Inline config `command` key, run as `/oxplow-<name>` |
| Hooks | HTTP hooks (`hooks.json`) | None: the thread's notifications, in-process (`ServicesObserverHost`) | A JS bridge plugin posting Claude-shaped bodies |
| Context channels | Both, as `additionalContext` | Prompt context only, as a developer item injected into the thread between turns (`thread/inject_items`); no tool channel | Prompt: a `synthetic` text part on the user's message; tool: appended to the tool's result |
| Token capture | OTEL metrics + the transcript's turns | The thread's `tokenUsage` across the turn | None |
| Final message | Stop's `last_assistant_message` | The turn's last agent message on the Stop body | The bridge reads the session's last assistant message onto `Stop` |

Codex finds skills only beside the directory it runs in (`skills.config`
entries don't add one, and a plugin must be installed into `~/.codex`),
and has no project-level slash commands (its prompts live in the person's
`~/.codex/prompts`, which oxplow doesn't write) — hence skills in the
worktree, each folder with a `*` `.gitignore` and the `.oxplow` marker,
and commands as skills (`skills_and_commands` in `codex.rs`). Another
agent that reads `.agents/skills` there sees them too. opencode has no
plugin namespacing, hence `oxplow-` instead of Claude's `/oxplow:`.

### How it's tested

- **Launch half:** `crates/oxplow-harnesses/src/text_contract.rs`
  launches every built-in harness with a marked system prompt
  and checks that the prompt, every core skill and every core slash
  command reach its agent: its command line, its environment, the request
  that opens its thread, or a file it was given
  (JSON values searched inside). What a launch doesn't deliver is pinned
  in `KNOWN_GAPS` (empty), and the test fails when a gap opens or closes.
- **Hook half:** `crates/oxplow-control-plane/tests/hook_goldens.rs` pins
  the hook answers byte for byte (`UPDATE_GOLDENS=1` rewrites them; a
  changed golden is a changed agent contract), including the deny every
  built-in renders, a harness without channels being acked and keeping
  its nudges, and a compaction bringing the context back.
- **Prompt assembly:** `agent_prompt.rs`'s tests (the session context
  only without a prompt channel, the skill index only without skill
  files).

## The agent's tools

Core (`crates/oxplow-mcp/src/lib.rs`) builds two tools, `list_commands`
and `run_command`. Every other tool is a foundation command or page that
declares `tool:` and is offered as `oxplow_<name>`; any extension declares
its own the same way, under its namespace ([extensions-commands.md](./extensions-commands.md)
"Agents: the MCP surface"). With the harness's namespace on top the agent
calls `mcp__oxplow__run_command`, `mcp__oxplow__oxplow_query_sql`. No other
spelling is accepted.

Every tool foundation declares is named below
(`every_foundation_tool_is_in_the_agent_model_doc` pins it).

| Purpose | Tools |
|---|---|
| Read oxplow's data | `oxplow_catalog` (every model with its owner, what it holds and its columns; one model's column docs), `oxplow_query_sql` (one read-only statement over the `v_*` models, live models included — [semantic-layer.md](./semantic-layer.md) "Live models"; its result is also structured content, typed by its output schema), `oxplow_read { uri }`, `oxplow_search` (BM25 over every searchable kind — tasks, thread notes, knowledge pages, an extension's — and file contents; each hit is a URI) |
| Code and revisions | `oxplow_vcs_log` (a stream's history from any revision, paged, past the index), `oxplow_vcs_status`, `oxplow_vcs_branches`, `oxplow_diff` (versions `working`, `snap:<id>`, `git:<rev>`; `since_fork`), `oxplow_vcs_blame` (through the `Vcs` provider; `working`, `snap:<id>` or `git:<rev>`), `oxplow_code_definition`, `oxplow_code_references`, `oxplow_code_hover`, `oxplow_code_call_hierarchy`, `oxplow_code_symbols` (a file's symbols from its language server, else oxplow's tree-sitter reading of the live file, saying which), `oxplow_code_workspace_symbols` (the editor's language servers, [lsp.md](./lsp.md)), `oxplow_ensure_change` (a change's analysis, then `v_change_*` through `oxplow_query_sql`) |
| Write | `run_command`, after `list_commands` |
| Ask a model | `oxplow_ai_decide` (typed questions about some text, on the `decide` role), `oxplow_ai_summarize` (on `summarize`): recorded computations paid with the person's key, as the asking thread ([ai-providers.md](./ai-providers.md)) |
| Pages | `oxplow_show_page` (shows the person a page beside the conversation — a page with params, or a `spec` of your own, written as a page file on `oxplow:temp` and shown as `oxplow:oxplow/page?page=temp/<id>`; returns its text and whether a window opened it), `oxplow_find_page` (the pages that answer a question, best first, each with its params; `about` a uri keeps the pages that show that kind of thing and binds its uri), `oxplow_run_page` (a page's text, each node under `[node <id>]`, or its loaders' rows; `page` or an inline `spec`), `oxplow_run_page_action` (a page's action as the page acting for you; a row action on one of the node's rows); `oxplow.page.describe { uri, text?, params? }` through `run_command` (what a page file is made of and where each node comes from; `uri` is the page's file on `oxplow:extensions`, `oxplow:temp` or the worktree; a file that is no page gives no rows) — [pages-and-tabs.md](./pages-and-tabs.md) "Page files" |
| Show the person | `oxplow_navigate` (takes them to any URI's page, switching stream when needed; by default the shown tab goes there, pushed; `replace` rewrites its entry, `new_tab` opens another) |
| Common questions, typed | foundation pages with `tool:`: `oxplow_thread_tasks`, `oxtask_next_task` (oxtask's page), `oxplow_catalog`, `oxplow_request_summary { request }` (a request at a glance: its facts, tasks, files, tests and coverage, metric changes and commits; foundation's `request-summary`) (each runs `oxplow.page.run` on its page, its params the tool's input: `agent_tools::ToolTarget::Page`). Every other page is `oxplow_find_page` → `oxplow_run_page` (`git-history { path }`, `recent-commits`, `snapshots`, `thread-sessions`, `session-transcript`) |
| What the person has | `oxplow_list_tabs` (the tabs each of their windows shows, per thread, stream or project: selected, on screen, stale), `oxplow_get_open_page` (the page open in a thread in the window they were in last, the others' beside it, or one listed tab: `page` is its identity, `view` its `#`; the page's run as a tree with their params, and the focused node) |
| Pinned refs | `oxplow_ref_drift { uri, path }` (one pinned file ref of any model row: its diff against its pin) |
| Extension authoring | `oxplow_validate_extension`, `oxplow_preview_collector`; its tests run in a process of their own, `$OXPLOW_BIN extension test <name>` (`OXPLOW_BIN`: the daemon's binary, set for every agent session) |
| oxplow's guidance | `oxplow_get_skill { name, reference? }` |

A tool's `read_only_hint` (so a client can auto-approve) is its
command's access: `View` and `Read` commands are hinted, `oxplow_show_page`
(it opens a tab) and `oxplow_run_page_action` are not, and nor are
`oxplow_ai_decide` and `oxplow_ai_summarize`: they change no record but spend
the person's provider key (a scope with `spends`), so the client asks before
they run. Each tool
but `list_commands` and `run_command` is a foundation command over a read
scope (`oxplow.records.read`, `oxplow.sql.query`, …; [commands.md](./commands.md)
"Scopes"), so an agent reaches it through `run_command` too; `v_scope`
lists the scopes. An anonymous connection may call the read-only ones.
Almost every read is a published model: an agent's thread, requests,
tasks, commits, snapshots, dashboards, diagnostics and errors are
`v_*` rows. A tool stays a tool only where a model can't be one: file and
revision content, live language-server answers, site search, a page's
rendering, a dry run.

**`oxplow_read { uri }`** (`oxplow_app::ref_read`) reads anything a URI names and
returns `{ uri, resource, row, related, content, truncated }`: the owning
model's row, `related` lists (a task's links and comments, a
commit's or a request's files), and `content` for the resources whose body no
model holds — a file at a revision (`oxplow:oxplow/file?path=src/a.rs` is the working tree;
`oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-40`; `rev=snap:<id>` a local-history snapshot),
an event's stored body (`oxplow:oxplow/event?id=<id>#part=input`, `part=output`, `part=prompt`, `part=summary`; the
caller's own stream's events only), a recorded model call's bodies
(`oxplow:oxplow/ai_call?id=<id>#part=request`, `part=response`; any call but one another agent's
thread asked for — an extension's, a collector's or a job's are the
project's), a page's definition (`oxplow:oxplow/page?page=<namespace>/<slug>`). A file or page is
read in the caller's stream's worktree, or another's named by `stream`
(`str2`). Content stops at 64 KB with `truncated: true`; ask for
lines to read further. A captured file's row comes from
`v_snapshot_file`; restoring it is the destructive
`oxplow.snapshot.restore_file`, which writes
into that row's stream's worktree.

**Commands.** Every write is a command through `run_command { id, input }`.
A completed call returns `{ result, command_run_id, event_id, inverse? }`.
When present, `command_run_id` names the integer `v_command_run.id`.
`list_commands` with no arguments is an index of `{ id, summary }`;
with `id` or `prefix` (`oxtask.task.`) it gives each command's
`input_schema`, an `example` input, whether a person `confirm`s it and
whether it's `undoable` ([commands.md](./commands.md) "Exposure to
agents"). A command's `confirm` asks only a person: an agent's run of
it runs without asking and without a proposal. What stays a proposal is
an effect's run that asks and a handler's consent check
([commands.md](./commands.md) "Proposals"; the person decides in the
thread and in Alerts; `v_command_proposal`). Every config key, including
the program- and model-choosing ones (`agents`, `lsp`, `agentConfig`,
`extensions`, `agentPromptAppend`, …), is set by `oxplow.config.set`
without asking. An anonymous connection may read and list but not run.
Which commands an agent may call is its harness's to decide (its
permission mode); an agent changes only its own stream.

**Pages.** When the person asks to see, list, compare or track something,
the agent looks first: `oxplow_find_page` finds the page that answers it
(by the questions pages say they answer, `answers:`), `oxplow_run_page`
reads one (text by default, `format: "json"` for rows; `draft: true` reads
the caller's stream's draft of the extension), and `oxplow_show_page` shows
it beside the conversation: an existing page with params, or its own
`spec`, an inline page (read-only: no commands, forms or actions) written as
a page file on `oxplow:temp` and shown as a tab `page=temp/<id>` that the person can save to
a knowledge page (an `oxplow-page` block) or as a page. It returns the page's text.
`oxplow_run_page_action` presses a page's declared action as the page acting
for the caller, under the same rules as running the command directly.

**Server instructions** (`get_info`) are the one text every harness
shows its agent: what oxplow knows (`oxplow_query_sql` over `v_*`), that every
write is a command, and to answer "show me" with `oxplow_find_page` then `oxplow_show_page` (or
`show_page { spec }` for a one-off) rather than printing a table. They ship to users' projects, so they never name this
repo's docs.

**Tools read the interface.** No tool belongs to one implementation:
task reads (`oxplow_query_sql` over `v_oxplow_task`, `oxplow_read` of a task's
URI, the next-task page) go through whichever list is active (empty with
none), so the tool list never changes with a switch. What an
implementation owns is commands, offered only while it's active
(`capabilities::Active::refusal`).

`McpError` (`lenient_from_object`, `to_snake_case`). A declared tool's
input reaches a command whose schema refuses unknown keys, so there a
top-level camelCase key is *renamed* to the schema property it stands for
(`fold_to_schema`). Inferable fields are optional and inferred in the
handler where safe.

### The ServerHandler is hand-rolled — re-diff it on every rmcp bump

`impl ServerHandler for OxplowMcp` writes out `list_tools`, `call_tool`
and `get_tool` instead of `#[tool_handler]`: the router holds only core's
two tools, and the declared ones are listed and resolved per caller
(`oxplow_app::agent_tools::offered`; `tools/list` is `cache_scope:
private`, since what a caller is offered depends on who it is). rmcp can change what the macro generates without
our copy following (rmcp 3 added `result_type` / `ttl_ms` /
`cache_scope` and made `call_tool` return `CallToolResponse`). On an rmcp
upgrade, diff the three methods against `rmcp-macros/src/tool_handler.rs`.
`crates/oxplow-control-plane/tests/mcp_wire.rs` runs a real
Streamable-HTTP session (initialize → tools/list → tools/call) against
`/mcp`, the only test that crosses rmcp's protocol layer.

### Surface parity with the IPC adapter

The MCP surface (agent) and the Tauri IPC surface (UI,
`crates/oxplow-tauri-ipc/`) are two thin adapters over `Services`.
`crates/oxplow-surface-parity` keeps them honest: its `MANIFEST`
classifies every op as `Both`, `AgentOnly`, `UiOnly { why }` (a person's
consent or setting, their selection or input, live status pushed to the
UI, runtime infra, an ancestry walk the VCS answers, file I/O the agent
does with its own tools) or `Model { models }` (a UI read an agent makes
through those published models with `oxplow_query_sql`). `tests/parity.rs`
enumerates the names on each side (core's tools,
`oxplow_mcp::core_tool_names()`, plus the declared ones an agent is
offered on a booted project; a capturing `tauri_specta::LanguageExt`) and
fails on anything unclassified, dangling, or a `Both` row missing a side.
A new `tool:` or `#[tauri::command]` needs a row; a UI-only row says why
(`every_ui_only_row_says_why`); a model row names published core models
(`every_model_counterpart_is_published`).

### MCP tool deferral is a harness decision

Claude Code defers MCP tool schemas (names only until `ToolSearch` fetches
one) by its own heuristics; it is not a signal the server sends.
`tools/list` reports every tool with its full schema, and there's no MCP
annotation or plugin setting to keep a tool loaded.

## Launching the agent

Each harness launches its own sessions: `AgentHarness::launch(&LaunchInput)`
writes whatever runtime files the agent needs and returns `Launch { spec,
transcript_missing }`. The spec is `LaunchSpec::Pty { command, env }` (a
shell command and the env the spawn sets — anything secret goes in `env`,
never `command`), or `LaunchSpec::CodexAppServer { socket,
program, method, params, attach }` (the request that opens a thread on
Codex's daemon, and the TUI's argv that attaches to it, "Codex: its TUI
on a thread oxplow observes"). The built-ins live in `crates/oxplow-harnesses/`
(`claude.rs`, `codex/`, `opencode.rs`; shell quoting and
shared runtime writers in `shared.rs`) and depend only on the domain;
core's text lives in `oxplow-agent-text`. The one caller is
`launch_session` in `crates/oxplow-rpc/src/commands/terminal.rs`, used by
`open_terminal_session` (a PTY, Codex's TUI included). It hands
the harness the session/thread/stream ids, the worktree and project dir,
the control-plane endpoints with the session's bearer, the identity env,
the assembled system prompt, the session's resume id, the agent text, the
harness's config, `HOME` (local streams only) and the program search
path. A harness never touches the database.

Restart Terminal replaces the agent session's PTY through this same
launch path. It reads the saved binding after the old process exits,
so stopping the TUI leaves the session's conversation intact. See
[Terminal](./terminal.md#when-a-session-ends-or-goes-away).

**What a harness implements.** `AgentHarness` requires what every harness
answers: `id`, `title`, `launch`, `tool_use` (its calls in
oxplow's vocabulary, "Tool vocabulary") and `render` (a `HookAnswer` in
its wire shape). Everything else defaults to none: `env_markers`,
`settings`, `refresh_text`, `turns`, `token_readings`, `prompt`,
`subagent`, `project_data_dir` (the folder it keeps of its own for the
project, what an extension's `files:` folder with `root: agent:<id>` is
relative to: Claude's `<config dir>/projects/<project, encoded>`; a session
in a worktree keeps its memories there too, under the main checkout's
name). The operations are async and their data serde shapes
(`LaunchSpec` and `HookAnswer` tagged by `kind`), because a harness may be
a provider's process ([providers.md](./providers.md), the `agent_harness`
contract): what a built-in answers in code, a provider declares or answers
by a verb of the same name. A harness's declaration names the optional
verbs it has as features — `transcript` (`turns`), `telemetry`
(`token_readings`), `runtime_text` (`refresh_text`), `subagents`
(`prompt`, `subagent`), `conversation_list` (`conversations`: its saved
conversations for a worktree), `conversation_fork` (a launch with
`LaunchInput.fork` continues a copy), `native_rename` (the person may name
the conversation in the harness's own UI too; Codex's is `thread/name/set`
on the observer's connection) — and the two context channels,
`prompt_context` and `tool_context`. `session_title(body)` (Claude's
`session_title` on a prompt) is the name the harness gives the session's
conversation, written to `agent_session.native_title`. **Two names.**
`agent_session.title` is oxplow's alias (Rename sets it; clearing it gives
the fallback back), `native_title` the harness's own; a tab shows
`v_agent_session.display_name` (alias, else native, else harness and day).
Rename can also name the conversation in the harness, only where it
declares `native_rename`: `oxplow.agent_session.rename { session, title,
native: true }` checks the feature and that the session's observer runs
(refusing the whole rename otherwise, so nothing changes), sets the alias,
and after commit runs `ObserverManager::rename_native` → `Connection::rename`
on the observer connection; a late failure there is logged, the alias kept
(the result says `native_renamed`). The agent session page's **Rename…**
is a form over it (the title, and the box). The new name
returns through `thread/name/updated` into `native_title` like any other.
The UI reads each
harness's features from `list_agent_harnesses` (`HarnessListing.features`),
and its logo (`HarnessListing.icon`: the SVG its `implementations:` entry
names in `icon:`, as a data URI, null when it declares none — no harness id
is named in the UI; "Implementations" in [extensions-commands.md](./extensions-commands.md)):

| Harness (key) | Features |
|---|---|
| Claude Code (`claude`) | terminal, permission_prompts, resume, transcript, telemetry, runtime_text, subagents, prompt_context, tool_context, conversation_list, conversation_fork |
| Codex (`codex`) | terminal, permission_prompts, resume, runtime_text, subagents, conversation_list, conversation_fork, native_rename |
| opencode (`opencode`) | terminal, permission_prompts, resume, runtime_text, subagents, prompt_context, tool_context, conversation_list, conversation_fork |

(`crates/oxplow-app/src/capabilities.rs`, the `harness(…)` built-ins.)

**Codex is Unix-only for now.** Its app-server client talks over the daemon's
unix control socket (`codex/app_server.rs`); a Windows transport (named
pipes) is its own piece of work. On Windows `Codex::launch`, `conversations`
and `AppServer::connect` answer "Codex sessions aren't available on Windows
yet" (`app_server::unavailable`, the one place that says it): no fallback to
anything else. The scripted-daemon tests (`app_server::fake`) and the Codex
tests that use them are `cfg(unix)`. Claude Code and opencode run in a PTY
and are not affected.
`crates/oxplow-app/src/harness_conformance.rs` checks the floor every
harness keeps, built-in or provider: answers render as objects and a deny
or context reads differently from an ack; the bearer stays off the
command line; a body naming no tool maps to none; a plain prompt is a
person's.

**A harness is a registry key, never an enum.** A session's `harness` is
the key of a registered harness (`HarnessRegistry`: the `agent_harness`
declarations in declaration order, then provider instances); nothing in
core names one (`source_guards::only_harness_implementations_name_a_harness`).
Each activation sets the declared ones (`harnesses::prepare_built_ins`
→ `set_declared`) and keeps a registered instance. Every harness has an
independent `oxplow.agents.<id>.enabled` boolean, default true; all may be
disabled. Each declared preference is registered as
`oxplow.agents.<id>.<setting>` with its own schema, level and origin.
External instance ids `namespace/instance` use `namespace.instance` in the setting key.
The runtime projects these scalars into `agent_config` only for launch.
`oxplow.agent.set_setting` checks that the harness declares a preference
and writes only that scalar; blank unsets the project override.
Sessions explicitly name an enabled harness: there is no default or priority.
New threads open the picker and never start an agent automatically. The UI
reads the harnesses through `list_agent_harnesses` (`useAgentHarnesses`,
`agentKinds.ts`). Each session keeps its own harness, so Claude and Codex
sessions run side by side.

### What each harness writes

Each harness writes its runtime files on every launch, from the agent
text it's handed. Where it can't be pointed at a folder it writes beside
the directory the agent runs in (the stream's worktree) and nowhere else:
a launch never writes into another stream's checkout. `refresh_text`
rewrites the skills and commands of runtimes already on disk, creating
none, across the project and every local stream's worktree
(`RuntimeRoots`); `capabilities::refresh_agent_text` calls it on every
registered harness at boot, after each extension activation and on a
`capability.switched`, so an agent outliving an upgrade or a switch reads
the current text. A skill folder oxplow wrote carries an `.oxplow` marker;
one no longer offered is removed, a person's own stay.

- **Claude Code** writes `.oxplow/runtime/claude-plugin/` (manifest
  `oxplow`, `hooks/hooks.json`, `skills/`, `commands/`) and a per-session
  `mcp-config.<ses>.json` holding the session's bearer (owner-only; its
  MCP config can't interpolate env). It runs `claude --plugin-dir <abs>
  --append-system-prompt <text> --mcp-config <file>
  [--strict-mcp-config] [--model <m>] [--effort <e>] [--resume <id>]`:
  `--model` / `--effort` come from `oxplow.agents.claude.model` / `.effort`
  (flags only; the person's `~/.claude/settings.json` is never written).
  Oxplow's server rides beside the
  person's own (local, project, user, plugin servers, claude.ai
  connectors), and `--strict-mcp-config` is added only by the
  `oxplow.agents.claude.mcpServers: strict` setting (`all` is the default;
  anything else is a config error), falling back to `~/.claude/local/claude` when no
  `claude` resolves. Its HTTP hooks (`HOOK_EVENTS`) are `PreToolUse`,
  `PostToolUse` (matcher `"*"`), `UserPromptSubmit`, `SessionStart`,
  `SessionEnd`, `Stop`, `Notification`, `SubagentStart`, `SubagentStop`,
  `PreCompact`, `PostCompact` (logged, see "Compaction"), plus `TaskCreated` /
  `TaskCompleted`, acked unread. Each
  carries `Authorization: Bearer $OXPLOW_HOOK_TOKEN` (`allowedEnvVars`)
  with an 8 s client timeout, above the server's 5 s budget: a lower one
  made Claude Code discard answers the server was about to give. Its OTEL
  env points the metrics exporter at `/v1/metrics` with the bearer in
  `OTEL_EXPORTER_OTLP_HEADERS` (10 s interval).
- **Codex** keeps nothing under `.oxplow/runtime/`: its launch is the
  request that opens the session's thread on Codex's shared daemon, its
  MCP server and system prompt in the thread's own config, and the TUI
  that attaches to it ("Codex: its TUI on a thread oxplow observes").
  `oxplow.agents.codex.model` is the open request's `model` param and
  `.effort` its `config.model_reasoning_effort` (never the person's
  `~/.codex/config.toml`). Its skills (and
  commands, as skills) go to the worktree's `.agents/skills/`
  (`shared::write_worktree_skills`).
- **opencode** writes `.oxplow/runtime/opencode-plugin/`
  (`plugin/oxplow-hooks.js`, `skills/`, `prompts/`) and runs `opencode [-m
  <model>] [-s <id>]`, the model only from `oxplow.agents.opencode.model`
  (blank: no `-m`, opencode's own default). oxplow used to force
  `github-copilot/gpt-5-mini`; GitHub Copilot stopped offering it and
  opencode 1.17.4 gave no error: it ran its most recent other model (a
  local one), which `session.created` / `chat.message` show only as that
  model's `modelID`, and the Stop's `model` reports. So oxplow names no
  model of its own. Hooks, MCP (bearer by
  opencode's own `{env:OXPLOW_HOOK_TOKEN}`), the instructions file, skills
  and commands ride `OPENCODE_CONFIG_CONTENT`, inline config opencode
  merges last. The bridge (`crates/oxplow-harnesses/assets/opencode-hooks.js`)
  translates: `chat.message` → `UserPromptSubmit`; `tool.execute.before`
  → `PreToolUse` (a deny throws, which blocks the call, so the write guard
  works; lowercase tool names and `filePath` become Claude's
  `Edit`/`Write`/… and `file_path`); `tool.execute.after` → `PostToolUse`
  (`callID` as `tool_use_id` so a call's two hooks pair, its bash tool's
  `metadata.exit` as `exit_code`); `session.idle` → `Stop` (with the last assistant message's `model`, `variant` and `tokens`); `session.compacted` →
  `PostCompact`. A child session (`parentID` set) posts no
  `UserPromptSubmit`/`Stop`; its tool hooks are posted as the parent's
  with `agent_id: <child>` and `agent_type: "task"`, read by
  `shared::subagent_of`, so the parent's resume id is never the child's.
  No `SessionStart`, `SessionEnd` or `Notification`. The bridge runs under
  bun in `its_bridge_sends_the_call_id_the_exit_code_and_a_childs_parent`
  and `its_bridge_delivers_hook_context_and_sends_the_final_message`.

**Identity.** Every process oxplow starts for a session gets
`OXPLOW_HOOK_TOKEN` (its session's bearer, which alone says who it is —
"Caller identity"), `OXPLOW_HOOK_BASE_URL`, and `OXPLOW_STREAM_ID`,
`OXPLOW_THREAD_ID`, `OXPLOW_SESSION` (`ses<n>`): the agent's own knowledge
of where it runs, which nothing trusts (`identity_env` in `terminal.rs`).

**What an agent inherits.** Every agent and terminal is
spawned without `agent_path::not_inherited(&harnesses)`: an oxplow
agent's identity (`agent_path::NOT_INHERITED`: the `OXPLOW_*` above and
the recordings' `OXPLOW_HOOK_DEBUG`, `OXPLOW_CODEX_DEBUG` and
`OXPLOW_OPENCODE_RAW`) and every registered harness's `env_markers()`
(Claude Code's `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, …). Otherwise
oxplow run from inside an agent's terminal starts child sessions — Claude
Code turns transcript saving off, breaking resume and token counts —
whose hooks point at the outer oxplow, and an agent's test runs would
append to the person's hook recording. It's a list, not a prefix:
`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_USE_BEDROCK` and `OXPLOW_HOME` are the
person's configuration and pass.

**The PTY.** The command runs as `sh -lc 'cd <worktree> && exec …'`
(`shared::in_shell`, `resume_or_fresh`), so the agent is the PTY's
process and its exit ends it. `open_terminal_session` keys it by the
agent session (the URI is the session's, `oxplow:oxplow/agent_session?id=ses3`), so a re-attach shows
the live process and two sessions in a thread are two processes; a
terminal's URI is a plain `$SHELL -l` in its stream's worktree.
Switching streams or threads doesn't kill sessions: the daemon keeps them
and a re-attach replays the buffer. They end with the daemon
(`Services::shutdown` → `TerminalSessionRegistry::close_all`, which ends
each PTY's whole process group: `.context/terminal.md` "Ending a
session's processes"); the next open resumes. There is no terminal multiplexer.

### The agent is spawned by absolute path, on purpose

A harness resolves its CLI through `LaunchInput::resolve_program` over
the launch's `search_path` (`agent_path::search_path`: the PATH, then
well-known install dirs; data, so a provider harness gets it too) and
launches the absolute path. **Don't "simplify" it back to the bare
binary name** — the bug only reproduces on a GUI launch:

- A GUI-launched app (Finder, dock, oxplow's launcher) gets macOS's
  minimal PATH; a terminal-launched one inherits the shell's, which is the
  only reason the bare name ever worked.
- `sh -lc` is bash in sh mode: as a login shell it reads `/etc/profile`
  and `~/.profile`, never `~/.zshrc` / `~/.zprofile`, so `-l` doesn't
  recover a zsh user's PATH.

Together those killed every agent pane launched from the launcher with
`sh: claude: command not found` (Claude Code's default `~/.local/bin` sits
in the gap). When resolution finds nothing the command keeps the bare
name but prefixes a `command -v` preflight that reports the real cause
(`shared::program_and_guard`).

`agent_path::base_pty_env` is the single source of the env every PTY
spawns with (`TERM`, `COLORTERM`, the widened `PATH`, and
`TERM_PROGRAM=Oxplow` with oxplow's version as `TERM_PROGRAM_VERSION`);
every spawn site calls it. The terminal names oxplow, never the terminal
oxplow was started from: with `TERM_PROGRAM=Apple_Terminal` and that
window's `TERM_SESSION_ID`, macOS zsh runs Terminal.app's session restore
in every terminal oxplow opens, and all of them share one saved session
("Restored session: …" and a mixed history). `TERM_SESSION_ID` is in
`NOT_INHERITED` and isn't set: nothing reads it once the program isn't
Terminal.app. One that hand-rolls its env reopens this bug for the tools the
agent shells out to, which the absolute path doesn't cover. Known limit:
version-manager shims (mise, nvm, volta) live in versioned dirs no fixed
list can guess; capturing a login shell's env (`$SHELL -ilc`) was
deliberately not taken (a subprocess per launch, and it can hang on a
person's rc file).

### Resuming

A session's `resume_session_id` is its harness's own session id
("Session tracking"). `shared::resume_or_fresh` `exec`s the resume
directly (`Resume::Saved`): there is no fallback, and a resume that fails
never starts a fresh conversation — only `agent_session.start_fresh`
does. The Claude harness only classifies: it looks for the transcript
under the effective config dir (`$CLAUDE_CONFIG_DIR`, else
`$HOME/.claude`) at `projects/<cwd with non-alnum → '-'>/<id>.jsonl`
(`resume_state`) and reports `transcript_missing` when the project dir
exists but the file is gone, so a failed resume can say why ("Resume
outcomes" in [terminal.md](./terminal.md)). An absent project dir, or a
stream on another host (no `HOME` passed), reads as unknown.

What can be resumed is listed two ways, both reads of
`AgentHarness::conversations` (`live_datasets::harness_conversations`):
for an open session, the `agent-session` page's `agent_session_conversations` loader (a choice list) over
`v_agent_conversation` (the page's Fork into New Session… form:
"Forking a conversation" in terminal.md); for a thread with no session in hand, the
`new-session-earlier` loader over `v_agent_conversation` (the New Session
page, "Earlier sessions"), which adds the stream's closed sessions and says per row whether taking it
reopens, resumes (`agent_session.open { resume }`), switches to a tab or
does nothing. Nothing is adopted until a person picks a row.

### The observer machinery (`agent_observer/`)

oxplow once opened ACP agents as chat sessions with their own view, and
later ran the in-app Assistant on the same session actor. Both are gone;
on 2026-11-20 the machinery was cut down to what observing a Codex thread
needs. It sends the agent nothing but context between turns and a rename
the person asked for: no prompt, no permission answer, no file request.

- **`agent_observer/conn.rs`** — `Connection` (open → the agent's own id for the
  conversation, closed, can_inject, inject, rename) and the `Incoming`
  messages a connection feeds the actor in arrival order: updates, a
  name, a wait (said or cleared), a compaction, a child agent's start or
  finish, a child's tool call, a turn's start (the agent's id) and end.
  Content text is capped at 64 KiB per block (`MAX_TEXT`, `cap`,
  `cap_value`). `agent_observer/codex.rs` is the one production connection; the
  tests script their own (`Script` in `session_tests.rs`).
- **`agent_observer/model.rs`** — oxplow's side of what a connection says:
  `ToolCall` / `ToolCallPatch` (the kinds Codex's
  items map onto) and `ChatUpdate` (the person's prompt, the agent's
  text, a thought, a tool call, a tool update).
- **`agent_observer/mapping.rs` (pure)** — tool kinds onto oxplow's vocabulary:
  edit, delete and move → `edit`; read → `read`; search → `search`;
  execute → `shell` (`rawInput.command` / `cmd`, else the title); fetch →
  `fetch`; an `mcp__…` title or name → `mcp`; other → `other`; think and
  switch-mode record nothing. Paths come from `locations`, then the
  diffs, then path-like `rawInput` keys, deduplicated. The stored
  `content` is `rawInput`, and once the call finishes
  `{is_error, …rawOutput}`, so a shell call's `exit_code` survives.
- **`agent_observer/session.rs`** — one actor task per session owns the connection
  and records, through `ObserverHost`, what a hooked turn records: the session
  start and the conversation's id; a prompt the person typed in the
  agent's UI as a `UserPromptSubmit` (only with no turn in flight; a turn
  that starts with no prompt is nobody's); each tool call at its start
  (`PreToolUse`) and once at its finish (`PostToolUse`), a finished write
  also checked against the policy (`check_tool`), so a denial is on the
  record although the agent's UI approved it; the turn's end (`Stop`)
  with its tokens, model and effort, and its answer — the agent's last
  run of text, which a tool call or a thought ends; waits, compaction,
  child agents and names; `Interrupt` when the agent goes away. A replay
  (a resumed thread's turns) records nothing but its name. It keeps no
  transcript: the open tool calls, the answer so far and a few flags.
- **`agent_observer/manager.rs`** — `Services.chat`: each agent session's command
  sender and a small shared `SessionView` (generation, the conversation's
  id, stopped). `open_codex` reserves the session's slot under one lock
  before connecting, so concurrent opens start one observer; `close`
  marks it closed at once, and a closed session still winding down is
  replaced and marked not current, so it records no Interrupt over its
  successor. `rename_native` and `close` are its only other calls.
  Closing a thread stops every open session's process through
  `agent_sessions::SessionProcesses::kill`.
- **`agent_observer/host.rs`** — `ObserverHost`, the seam (tests use a recording
  double). `ServicesObserverHost::new(svc, stream, session)` holds
  `Weak<Services>` and routes through `AgentPolicy`, `AgentContext` and
  hook ingest, so an observed turn lands in the same tables as a hooked
  one.
- **`agent_observer/guard_tests.rs`** scans the source: no production code starts a
  Codex turn (`"turn/start"`), and context reaches a Codex thread only from
  `codex.rs`.

**Gotcha: `serde_json` runs with `preserve_order` workspace-wide** (the
workspace `Cargo.toml` declares it). JSON objects serialize in insertion
order. Anything that needs a stable text form builds it from a
`BTreeMap` (`metric_cube::dims_key` does), and hashes never hash JSON.

### Codex: its TUI on a thread oxplow observes

A Codex session is Codex's own TUI in the session's terminal, on a thread
of Codex's shared app-server daemon that oxplow opens first and keeps
observing. Because the thread lives on the daemon, `codex agents`, the
Codex app and other Codex TUIs see it like any other; because oxplow
opened it, it carries oxplow's MCP server and system prompt.

**Why this shape.** A `codex` started with any `--config` override runs
embedded, never on the daemon, and its command-line hooks need a trust
review before they run, so the old way (hooks, MCP server and exporter as
overrides) kept sessions off the daemon. The daemon's API takes what
oxplow needs per thread instead: `thread/start { cwd, sandbox, config,
developerInstructions }`, where `config.mcp_servers.oxplow` (an HTTP
server with the session's bearer as a literal `Authorization` header)
reaches oxplow and the person's own `~/.codex/config.toml` stays theirs.
A per-thread `config.hooks` doesn't apply, so there are no hooks: oxplow
reads the thread's notifications. Probed on the 0.163 daemon:

- `codex resume <id>` attaches to a thread another client holds, without
  forking, and the thread keeps the config it was opened with. A TUI
  resuming a thread **no client holds** re-applies the TUI's own config,
  dropping oxplow's MCP server — so oxplow opens and holds the thread
  before the TUI attaches, and keeps holding it after the TUI exits.
- A client-started thread has no rollout until something persists it,
  and `codex resume` needs one: `thread/goal/set` writes it
  (`thread/settings/update` doesn't).
- `codex --remote unix://… -c …` starts a thread on the daemon but
  doesn't carry the overrides to it, so the TUI can't bring oxplow's
  config itself.
- Each resume writes a history segment whose `session_meta` names the
  thread as its parent; those are not threads (resuming one has "no
  rollout").

**The client.** `oxplow_harnesses::codex_app_server` is JSON-RPC over a
websocket on the daemon's control socket,
`$CODEX_HOME/app-server-control/app-server-control.sock` (`$CODEX_HOME`
defaulting to `~/.codex`); when nothing answers it runs `codex app-server
daemon start` once. Each user opens its own connection: a session's
observer (`oxplow_app::chat::codex`), one `thread/list` for the
conversation picker. The handshake is `initialize { clientInfo: oxplow,
capabilities: { experimentalApi } }` then `initialized`; the client reads
messages up to 256 MiB (a thread's answers run past the websocket's
16 MiB default). The protocol is marked experimental, and the recorded
fixture (`crates/oxplow-app/tests/fixtures/codex/app-server.jsonl`,
0.163.0) pins the shapes oxplow reads.

**Launch.** The harness (`crates/oxplow-harnesses/src/codex/`) writes
the worktree skills and returns `LaunchSpec::CodexAppServer`: the socket
under the person's home (none for a stream on another host: no launch),
`codex` to start the daemon, the request that opens the thread —
`thread/start` for a new conversation, `thread/resume { threadId,
excludeTurns }` for the bound one, `thread/fork { threadId }` — each with
the worktree as `cwd`, oxplow's system prompt as `developerInstructions`
and oxplow's MCP server in `config`; and `attach`, the TUI's argv
(`<codex> resume --cd <worktree>`, no `-c`: an override runs it
embedded). No `codex` where oxplow looks is a launch error.
`open_terminal_session` opens the observer (`ObserverManager::open_codex`,
which keeps one already running and answers its thread's id), then
spawns the session's PTY as any agent pane, running `attach` with the
thread id appended. The thread's id is the session's resume id.

**The observer.** The chat session's actor (`agent_observer/session.rs`) over
Codex's `conn::Connection`, with no view. It:

- persists a new thread before answering (`thread/goal/set` with
  `status: "paused"`, then `thread/goal/clear`), so the TUI can resume it.
  The goal is paused because an **active** goal makes Codex run a turn
  with no prompt (it reads skills and asks "What would you like me to work
  on?", on the person's quota). Recorded on the 0.167.1 daemon, where
  `turn/started` follows the goal's clear before oxplow's inject is even
  sent, and reproduced with a goal set and nothing else; a paused goal
  writes the rollout and starts no turn (pinned by
  `a_new_thread_is_saved_so_the_tui_can_resume_it`). A turn that starts
  with no `userMessage` anyway is nobody's prompt: no turn of the person's
  opens (`a_turn_with_no_prompt_is_not_a_persons_turn`);
- passes only messages naming its thread (the daemon also sends a
  client what concerns other threads, Codex's title-generation thread
  among them);
- records what the person does in the TUI: a `userMessage` with no turn
  in flight is their prompt (`external_prompt`, a `UserPromptSubmit`);
  `commandExecution` → a tool call (read when every parsed action is a
  read, search when they're searches or listings, else execute; the
  model's command, not its shell wrapper; `exit_code` once it ends),
  `fileChange` → an edit (delete, move) of its paths, `mcpToolCall` →
  `mcp__<server>__<tool>`, `webSearch` → fetch, anything else → other;
  `turn/completed` → the Stop with its last agent message and the turn's
  tokens (the change in the thread's cumulative counts: input less the
  cached input, cache reads separate) and the model and effort then
  (the open result's `thread.model` / `reasoningEffort`, moved by
  `thread/settings/updated`'s `model` / `effort`) and the context fill
  (`thread/tokenUsage/updated`'s `last.totalTokens` and
  `modelContextWindow`, last said); a finished `contextCompaction` item →
  `agent.context.compacted` (`PostCompact`); `thread/name/updated` → the
  conversation's name (`HookIngestService::name_conversation`);
- sends no turn and answers no request: approvals and questions are the
  TUI's (`prompt` refuses; `guard_tests::nothing_starts_a_codex_turn`
  finds no `"turn/start"` in production code).

**Recorded on CLI 0.166.0, daemon 0.166.0 → 0.167.1** (the daemon
self-updated mid-session; each fixture's header says which it ran on;
`tests/fixtures/codex/approval.jsonl`, `interrupt.jsonl`, taken on oxplow's
own Codex home, with no turn before the person's prompt):

- *Approval the TUI answers.* `thread/status/changed` with
  `waitingOnApproval` comes **before** `item/commandExecution/requestApproval`
  (which names the command, `reason` and `commandActions`). The wait is said
  at the status, and said again with the command once the request arrives
  (`Approve in Codex: <the model's command>`); `serverRequest/resolved`
  clears it, and the status falling back to `active` says nothing more.
  oxplow sends no answer. Pinned by
  `a_recorded_approval_the_tui_answers_is_waited_on_then_cleared`.
- *Interrupt.* Esc gives `turn/completed` `status: "interrupted"` with
  `error: null` (an interrupted turn, not a failed one). The command is not
  killed: it goes on as a background terminal, and `/stop` later gives
  `item/completed` `commandExecution` `status: "failed"` (`exitCode: -1`)
  after the turn's end. That late finish is recorded on its call (a
  `PostToolUse` with `ok` false) and opens or ends no turn. Pinned by
  `a_recorded_interrupt_ends_the_turn_and_a_late_failed_call_opens_none` and
  `a_late_failed_call_after_an_interrupted_turn_opens_no_turn`.
- *Background terminal.* Every Codex command is a `commandExecution` with
  a `processId` and `source: "unifiedExecStartup"`; one the agent leaves
  running past its turn is only seen as a call whose `item/completed`
  (`exitCode: 0`, `aggregatedOutput`, `durationMs`) arrives ~40 s after
  `turn/completed`. **The exit does not wake the agent**: no `turn/started`
  follows. oxplow records no job for it: nothing says at the call's start
  that it is a background one, and a job with no report would read as
  `running` (a request waiting on it) for ever. The finish is the call's
  (`agent.tool.finished` with its exit code and duration, anchored to no
  open turn); no turn opens. Pinned by
  `a_recorded_background_terminal_finishes_its_call_and_wakes_nothing`.

- *Failed turn* (`failed_turn.jsonl`, an invalid API key; pinned by
  `a_recorded_failed_turn_ends_once_with_the_final_error` and, through a
  session, `a_recorded_failed_turn_is_one_errored_turn`). After
  `turn/started`: ten `error` notifications with `willRetry: true`
  (`Reconnecting... n/5`, over WebSockets, then HTTPS after a `warning`
  "Falling back from WebSockets to HTTPS transport"), `thread/status/changed`
  `systemError`, an `error` with `willRetry: false`, and `turn/completed`
  `failed` with `turn.error` and no items. Only that last message ends the
  turn: once, `Errored`, with the final error's class and message. The
  retries, the warning and `systemError` say nothing (the status is no wait
  on the person).
- *Other threads on the daemon.* The daemon sends every connection what
  concerns every thread. After each user message Codex starts an ephemeral
  naming thread (`ephemeral: true`, no parent, its own model) that arrives as
  `thread/started` and closes a minute later; a thread of an earlier session
  arrives as `notLoaded` and `thread/closed`. A message passes only when it
  names the session's thread (`threadId`) or one of its children, and a
  `thread/started` only registers a child that names the session's thread as
  its parent, so neither opens a thread, session, turn or job (pinned by
  `another_thread_on_the_daemon_is_not_this_sessions`, which replays all five
  recordings through a session).
- *Per-turn bookkeeping.* `turn/moderationMetadata` (Codex's verdict on the
  prompt and output) is not read; `thread/settings/updated` is, for its
  `model` and `effort` (`moderation_is_ignored_and_settings_are_read`).
- *The daemon restarts mid-session.* The daemon self-updates (0.166.0 →
  0.167.1 in one recording session); the restart killed a background process
  and its `item/completed` never came
  (`codex-background-terminal.jsonl.daemon-restart` in the local recordings).
  The observer's connection closes, so the session ends: the host gets an
  interrupt, which ends the open turn `interrupted` and abandons the calls
  it left running (`tool_call.project` does that for any turn end, `Stop`
  or `Interrupt`), so nothing stays `running` for ever. The observer does not
  reconnect by itself (the TUI died with the daemon); the next open resumes
  the thread on the daemon that came back. Pinned by
  `a_daemon_restart_mid_turn_interrupts_the_turn_and_the_call_in_flight`,
  `a_restarted_daemon_is_observed_again_on_the_next_open` and
  `an_interrupted_turn_abandons_the_call_in_flight`.

It stays open when the TUI exits (the PTY's exit ingests the
`SessionEnd`), so a reopened TUI attaches to a held thread; it closes
with the session or its thread (`SessionProcesses::kill`). After an
oxplow restart the next open resumes the thread with `config` again
(a loaded thread no client holds takes it).

**The sandbox confines Codex to its worktree.** No hook runs before a
Codex edit, so the cross-stream rule ("Isolation") is Codex's sandbox:
the open request always carries `workspace-write` with the thread's
worktree as its root, so Codex may change that tree and nothing outside
it — another stream's included. oxplow never changes the sandbox after.
A thread the TUI starts itself (`/new`, or a switch of its own) runs on
the TUI's own config and is outside oxplow: it is the person's Codex
session, and oxplow leaves it alone.

**Context between turns.** The TUI owns the prompt, so oxplow cannot
put context on one. Codex declares `prompt_context` all the same, and
delivers it through the daemon's `thread/inject_items`, which appends a
developer-role message to the thread's history. Checked live on the
0.163 daemon (a CLI 0.162.1 thread), so it is context and not Drive:

- on an idle thread it starts no turn and spends no tokens (no
  `turn/started`, no `thread/tokenUsage/updated` after it); it answers
  `{}`;
- a pending approval stays pending: the request is not answered or
  changed, and the person's prompt in the TUI is untouched;
- the agent reads it at its next turn (the `developer` and `user` roles
  both work; oxplow uses `developer`), and it survives
  `thread/compact/start` and a `thread/resume`;
- **during a turn it is read as more input**: injected right after
  `turn/started`, the person's turn answered "unknown", then answered
  again with the injected text, in the same turn. So the timing rule is:
  inject only while no turn runs.

The session actor (`agent_observer/session.rs`, `inject_context`) therefore builds
the context with `AgentContext::build_context` — the same "once, then on
change" baselines and nudges a prompt
carries — and injects it (`Connection::inject`), only when the connection
says it can (`Connection::can_inject`: the thread runs no turn, read from
`turn/started` / `turn/completed`; and the daemon knows the method, else
the first refusal, "unknown variant", turns it off for the connection).
It does so at four moments: when the session opens, when a turn ends,
after a compaction (`ObserverHost::compacted` resets the baselines; one inside
a turn waits for that turn's end), and **on a change**: the
`agent.context_changes` pump consumer (`context_watch.rs`) bumps
`Services::context_changes` when the log records a fact the context
reads — `agent.session.started` /
`.ended` (who else is in the worktree) — and each session watching it
builds again. A build that changes nothing is `None` (the baselines), so
a no-op event injects nothing; a change during a turn waits for its end.
No polling. The launch prompt's `developerInstructions` leave the session
context out (the harness declares `prompt_context`); the first injection
carries it.

**The context basket.** What the person collects for agents
(`context_item`, `v_context_item`, the `oxplow.context.*` commands) is in a
**basket**: each thread has its own (the app calls it "This Thread"), and
the person keeps **named baskets** (`context_basket`, `v_context_basket`)
for the project, across every stream — collected into while reading, with
no thread needed. A thread's next prompt sends **one** basket: its own, or
the named basket set to go next for it (**Send Next**,
`oxplow.context.send_next`; one per thread, a basket next for one thread at
a time; a basket set next but empty sends the thread's own instead). The
next prompt of *any* agent in the thread carries it as a `<context-basket>`
block after the session context (`agent_context::basket_block` over
`context_store::waiting_for_thread_tx`; **`v_context_next` is the one
statement of which items that is** — the prompt, the Context panel and its
badge (`nextPrompt`, `sendingNext`) and the Context page's `next` loader all
read it), a named basket's items under a line
`From your "<name>" basket:`, numbered. Each page is
`[oxplow uri …]` with its label; each piece of text is quoted (up to
`BASKET_TEXT_IN_CONTEXT` characters) with where it came from; a text whose
source has no lines to read back (a terminal) brings its `around` in a
second fence under "Selected from these lines:", so the agent tells the
selection from its surroundings. Each item records the stream it was
collected in: one delivered into a thread of another stream says
`Collected in stream str2 ("<title>", branch <b>): read it with oxplow_read
and stream: "str2"`, since a file URI with no filesystem is read in the
reader's own worktree otherwise. The person's note on an item (`note`,
never cut) is indented under it; when any item has one, the header tells
the agent to act on each and answer by number. `commit` runs
`take_delivered_tx`: it links the turn to every item (`prompt_mention`),
**deletes** the items (keeping a copy as the thread's **last send**,
`context_sent_item`, which the next send replaces) and clears the choice,
so the thread sends its own again (Send Next lasts one send; the basket stays, empty, until the person
deletes it).
There is no delivered history: the turn's links and the transcript are the
record. Only the last send is kept, so the person can **Put Back**
(`oxplow.context.put_back`) what they did not mean to send: each item, with
its note and the time it was first added, returns to its basket (a deleted
named basket made again); it chooses nothing to go next, so the next
message doesn't send it straight away. A named basket goes only when it is
chosen; an item back in the thread's own basket is **held**
(`context_item.held_at`, V74): the own-basket branch of
`waiting_for_thread_tx` skips it, an add doesn't release it, and only
**Send Next on This Thread** (`send_next` with no `basket`,
`release_held_tx`) does, so it goes with the next prompt; undoing that
holds it again (`send_next`'s `hold`), and undoing a `remove`, `clear` or
`move` of a held item keeps the hold (`add`'s and `move`'s `held`; a move
itself drops it). An agent may run it for its own thread (the only basket command it
may), when the person says something went by mistake. Claude Code and opencode get it at prompt submit. Codex gets it by
injection while no turn runs (`context.add`, `move`, `send_next` and
`put_back` wake `context_watch`; a held put-back changes nothing, so it
injects nothing), so with two agents in a thread an idle Codex can take
the baskets before the agent the person prompts. The baskets are the
person's: an agent's `context.*` run is refused, but it can read
`v_context_item` and `v_context_basket` ("look at my Auth basket"). Pinned
by `the_context_basket_reaches_the_next_prompt_once`,
`a_put_back_waits_until_send_next_on_this_thread` and
`a_basket_sent_next_replaces_the_threads_own_for_one_prompt`.

*The UI* (`apps/desktop/src/contextBasket.ts`, the window's selected thread
and its stream from `setSelectedThread`): every add opens the **Context
dock panel** (`dockStore.open("context")`) with an add card on top (`components/Context/ContextAddCard.tsx`):
what is being added by its name (`contextTitle.ts`: the label, else its
page's title from the one title cache — an agent session's display name —
else a file's path and lines; never a raw URI, and a title found is sent
as the item's `label`), an optional multiline note, and **To**, a row of
choices: This Thread (disabled with no thread selected), each named basket,
and **+ New basket**, which swaps in a name field (Enter adds; Escape goes
back to the choices). To starts at the basket last added to (the app
preference `context.lastBasket`), else This Thread, else the first named
basket, else a new one. Ctrl/Cmd+Enter or Add adds (with no note too) and
names the window's stream; Escape (not passed on, so the dock stays) or
Cancel drops it. The draft (note, choice, new name) is kept in the add's
store, so the dock closing on a press outside loses nothing.
**Add to Context** is on every page's ☰ (`menu-item-page-add-to-context`,
beside Ask; the page's canonical ref and title; offered with no thread
too) and on selections: an editor's and a diff's right-side selection
(**Add Selection to Context**, text plus the `#lines=` URI), a page row's
menu (**Add to Context**: the row's first linked page, else the row as
text), the terminal's menu and a page's menu over its selected text. Each
toasts with Undo. The **Context** dock panel (`@`, badge = what this
thread's next prompt carries, `nextPrompt`) has one tab per basket, drawn as
the main area's tab strip (the panel-header tint, ruled tabs, the open one
tinted and underlined in the accent): This Thread (with a thread) and each
named basket with its count; the one tab the next prompt sends wears a
small grey paper plane after its count (`sendingNext`).
holds the open tab's actions on the left: the **Send Next** switch (the
same paper plane, icon only, its tooltip saying what happens; on for the
basket that goes, and pressing it on a named basket sends This Thread
again), then This Thread's Clear, or a named basket's Rename (in place),
Clear, Delete ("Sent next by another thread" beside them when so) — also
on the tab's right-click. The open tab is the app preference
`context.openTab`. At the bar's right, "Sent N items from <basket>" and
**Put Back** while the thread has a last send (`v_context_sent`). Items are one row each: the name, a note mark (white
when there is a note, grey when not, its text on hover), the text on one line (cut to
fit) and a × at the far right; a row's right-click has **Move to ▸** and
Remove. A row's click expands it in place (a chevron turns, the details
grow open): its whole text, its note edited in place, and Open / Move to…; another click, or Escape outside the note, folds it; one row is
open at a time. While an add waits, its card is all the panel
shows; the add then opens the tab it went to. The **Context** page (foundation page `context`) lists Next
prompt, Baskets and In other baskets, with the same actions.

**Building takes nothing; delivery commits.** `build_context` returns a
`PendingContext` and changes no state: no baseline moves, no nudge is stamped. `AgentContext::commit` records the blocks as the
session's baselines and marks its nudges, and runs only once the agent has the text: the hook
answer for Claude Code and opencode (`prompt_context` is build then
commit, the answer counting as the success), and for a Codex thread
the daemon's `{}` to the injection. A refused injection commits
nothing, so the next chance carries it whole.

Pinned by `context_is_injected_between_turns_never_during_one`,
`a_refused_inject_loses_nothing` and
`a_change_while_idle_is_injected_before_the_next_turn` (a scripted
daemon), `a_context_that_was_not_delivered_takes_nothing` and
`a_recorded_inject_starts_no_turn_and_waits_for_none`
(`tests/fixtures/codex/inject.jsonl`, recorded on the live daemon),
`nothing_is_injected_into_a_running_turn`, and the source guard
`only_the_codex_connection_injects_context`, and `nothing_starts_a_codex_turn`.

**Pitfalls.** `/new` in the TUI starts a thread with the TUI's own
config, which oxplow doesn't bind: use Start Fresh.

**Collab agents are subagents** (feature `subagents`). A Codex thread can
spawn child agents, each its own thread on the daemon, and the daemon
sends this client the child threads' notifications too. The observer
(`agent_observer/codex.rs`, `on_message`) maps what it sees; it never calls a collab
tool and never starts, interrupts or closes a child, and a child gets no
session slot, bearer or process of oxplow's:

- `subAgentActivity` `started` / `completed` / `interrupted` on the
  parent thread (`agentThreadId`, `agentPath`) become
  `agent.subagent.started@2` / `.finished@2` (`Incoming::Subagent` →
  `ObserverHost::subagent` → a `SubagentStart` / `SubagentStop` hook), the
  subagent's id the child thread's id and its kind the `agentPath`
  (`/root/count_lines`). A `collabAgentToolCall`'s `agentsStates` do the
  same for the children it names (a terminal state finishes one, with its
  `message` as the result), once per child whichever item says it first.
  Recorded 2026-10-16 on 0.162.1: a spawn shows only as a
  `subAgentActivity` (no `spawnAgent` collab item); `wait` is a
  `collabAgentToolCall` with no receivers. Recorded again on CLI 0.166.0 /
  daemon 0.167.1 (`tests/fixtures/codex/subagent.jsonl`): the same — the spawn is
  `subAgentActivity` items only, `wait` names no receiver and has empty
  `agentsStates` although it waited for the child (so it is the parent's
  own call and starts or finishes no child), and the child's thread has no
  `thread/started`; the child's `turn/started` carries `rootTurnId` = the
  parent's `turn.id`, which is the turn the start is attributed to (the
  parent's running turn when the first `subAgentActivity` shows). Pinned by
  `a_recorded_subagent_is_attributed_to_the_turn_that_spawned_it`.
- A `collabAgentToolCall` is also the parent's own tool call, kind
  `subagent`, named by its tool (`wait`, `spawnAgent`, …).
- A child thread's **finished tool calls** (`item/completed` of a command,
  a file change, an MCP call, …) are recorded in the parent's session as the
  subagent's (`ToolUse.subagent`, `v_tool_call.subagent_id`), so its edits
  are the parent's turn's. Only those: a child's messages,
  token counts, status and approval waits are its own thread's and do not
  reach the parent (a child waiting on approval does not park the parent).
  Which threads are children is learned from the parent's items, so a child's
  messages before then are dropped.
- The status rule holds as for Claude: the parent's turn ending with a
  child still running leaves the session working.

A child's tokens are not recorded: its `thread/tokenUsage/updated` is its
own thread's and is ignored (per-subagent token facts are future work;
`.context/metrics.md`). Pinned by `a_recorded_codex_collab_agent_is_attributed`
(`tests/fixtures/codex/collab.jsonl`, a live spawn on 0.162.1),
`a_collab_agent_is_a_subagent_of_the_thread` and
`a_codex_child_agent_is_a_subagent_of_the_parents_session`.

## Observing

### Tool vocabulary

Core never reads a harness's tool names. Each call is mapped onto
`oxplow_domain::agent::tool`: a `ToolUse` with the harness's `name`
(recorded, shown, never matched), a `ToolKind` — `read`, `edit`, `shell`,
`subagent`, `ask`, `plan`, `search`, `fetch`, `mcp`, `other` — the files
it names (`paths`), a shell call's `command`, a short `detail`, the
`call_id`, its outcome (`ok`, `exit_code`), an `ask`'s `question` and the
`subagent` that made it.

- **Who maps.** The harness (`AgentHarness::tool_use`). Claude Code's
  shape is `shared::claude_shaped_tool_use` (Claude, and opencode, whose
  bridge renames its tools to Claude's — but for `apply_patch` with a
  `patchText`). opencode's `apply_patch` → `edit` of every file the
  patch names (`shared::patch_paths`). A Codex
  thread's items are mapped by `chat::mapping` ("Codex: its TUI on a
  thread oxplow observes").
- **Where.** The hook route maps a PreToolUse / PostToolUse body with the
  bearer's harness before the policy runs and hands the call to the
  ingest (`HookEnvelope.tool`); the Codex observer does the same with its
  mapping. The ingest maps nothing: it runs in one transaction, and a
  harness's `tool_use` may be a provider's call. A body naming no tool
  records nothing.
- **What reads it.** The write guard (`edit`, every path), the status
  derivation (`subagent`, `ask`, `plan`), file claims (`edit`), test-run collection (`shell`, its `command`
  and `exit_code`), a turn's writing calls (`thread.checkpoint`, by
  `v_tool_call.kind`), the bundled struggle model, `v_context_read`, and
  `agent.tool.requested@4` / `agent.tool.finished@4`, which carry `tool`,
  `kind` and `paths`.

### Caller identity

Every agent session gets its own bearer when it launches
(`terminal::session_endpoints` → `SessionAuth::issue`,
`crates/oxplow-app/src/session_auth.rs`), bound in memory to its session,
thread, stream and harness (a `Principal`). The control plane's routes —
`/hook/{event}`, `/v1/metrics`, `/mcp`, `/dev/ping` (a
health check) — admit a request only with a live session's bearer and
take who sent it from the bearer alone: `auth_middleware` puts the
`Principal` in the request's extensions, the hook and OTLP handlers read
the session, thread and stream from it, and MCP's `caller_of` reads it
from the request parts rmcp hands each tool. No header or query names the
caller (an `X-Oxplow-Thread` is ignored), so one agent can't run
commands, post hooks or export tokens as another, and two sessions on one
thread are told apart.

- **One bearer per session, for the daemon's life.** Every launch of a
  session is handed the bearer it holds: a launch can attach to the
  process already running, which keeps the bearer it started with, so a
  new one would lock it out (every hook a 401). A session moved to another
  thread gets a new one. Closing the session, its thread or its stream
  revokes it (`SessionProcesses::kill`). Bearers end with the daemon.
- **Where it rides.** The agent's env (`OXPLOW_HOOK_TOKEN`, from
  `LaunchSpec::Pty.env`, never in the `sh -lc` command), Claude's
  owner-only per-session MCP config (no runtime file every session shares
  holds a bearer), the OTLP exporter's headers
  (`OTEL_EXPORTER_OTLP_HEADERS`), opencode's `{env:…}` header, and a Codex thread's `config.mcp_servers`
  header, sent over the daemon's socket. A bearer is never
  on a command line, which any process can list.
- **What it protects.** One agent acting as another by naming it. It is
  not a wall between processes of the same OS user, who can read another
  agent's env or owner-only files; agents run as the person, so that
  boundary is the OS's.
- **In process.** The Codex observer and the UI's interrupt build envelopes
  directly, naming the session.
- **The actor carries its session.** An MCP call's actor is
  `Actor::Agent { session_id, thread_id, stream_id }`; its contribution run
  records the session (`contribution_run.session_id`) and its events are
  anchored to it (`Actor::anchors()`). `source` stays `agent:thr<n>`.
- A hook's harness is the bearer's, so even a timed-out hook is answered
  in its harness's shape.

### Hook ingest and recording

**Recording a session.** Claude Code and opencode: `OXPLOW_HOOK_DEBUG=<file>`
(the dump below). opencode's lines are what its bridge translated, so
events the bridge doesn't forward leave no trace. While the variable is
set, oxplow also launches opencode with `OXPLOW_OPENCODE_RAW=1`
(`raw_recording_env`; `OXPLOW_HOOK_DEBUG` itself is never in an agent's
environment) and the bridge first posts every plugin hook input/output and
every event untouched to `/hook/opencode.raw`; the route appends each as a
line `{event: "opencode.raw", thread, at, payload: {kind: "hook" | "event",
…}}`, in order among the translated hooks, and does nothing else with it.
The replay (`replayed_harnesses.rs`) reads only the hook lines: trim the
`opencode.raw` lines when a recording becomes a fixture.
Codex: `OXPLOW_CODEX_DEBUG=<file>` (`AppServer::connect`,
`crates/oxplow-harnesses/src/codex/app_server.rs`) appends every message
oxplow's app-server client sends and receives, one JSON line each in the
fixtures' shape: a header `{recorded, codex, note}` first (`codex` is
`null`: the version isn't known at connect; fill it in), then `{method,
params, emittedAtMs}` for a notification (a server request adds `id`),
`{sent, params}` for what oxplow sent, `{response}` / `{error}` for the
answer, `{answered: id, result}` for oxplow's answer to a server request.
Every line but the header also carries `conn` (a number per connection)
and `purpose` (`observer`: the session's connection, `listing`: a thread
picker's). Each line is one `write_all` under a process-wide lock, so
connections writing at once never interleave. The daemon broadcasts
(`thread/started`, `thread/status/changed`, …) reach every connection;
only the observer records what it hears unasked, so a broadcast is in the
file once, and a listing's lines are its own `sent` / `response` ones. Trim it
to the case, scrub paths and ids like the other fixtures, and read it back
with `chat::codex::tests::parse_recording`
(`a_session_recorded_by_the_client_replays_like_the_fixture`).

Each hook POSTs to `/hook/<event>` with the session's bearer. The route
(`handle_hook_inner`) dumps it when `OXPLOW_HOOK_DEBUG=<file>` is set (one
JSON line per hook: `event`, `thread`, `at`, `payload` — how to learn a
harness's real payload shapes before depending on them), maps the prompt
(`AgentHarness::prompt`, below) and its turn id (`AgentHarness::harness_turn_id`:
Claude Code's `prompt_id`) and the tool call with the bearer's harness, runs the policy on a `PreToolUse`, hands
one `HookEnvelope` to `HookIngestService::ingest`
(`crates/oxplow-app/src/hook_ingest.rs`), and answers: context after a
`PostToolUse` or a person's `UserPromptSubmit`, a deny, or an ack. An
event it doesn't know is acked unread; `PreCompact` only resets the
session's context baselines, and `PostCompact` is logged (`agent.context.compacted`).

**Core reads no harness body keys.** What a body means the adapter says,
in the envelope; `AgentHarness::reading` (Claude and opencode share
`shared::claude_shaped_reading`; the chat host fills it for Codex;
a harness that says nothing leaves the default) fills
`HookEnvelope.reading` (`HookReading`):

| Field | Used for | Claude Code's key |
|---|---|---|
| `transcript` | the turn's transcript path, the session's start/end bookkeeping | `transcript_path` |
| `final_message` | a Stop's `answer`, a SubagentStop's result | `last_assistant_message` |
| `waits_on_person`, `notice` | a Notification that parks the thread on the person, and its text | `notification_type` (`permission_prompt`, `elicitation_dialog`, `agent_needs_input`), `message` |
| `compaction` | a SessionStart that is a mid-turn compaction, not a new process | `source: compact` |
| `compact_summary`, `compact_trigger` | a PostCompact's summary and trigger | `compact_summary`, `trigger` |
| `end_reason`, `clears_conversation` | a SessionEnd's reason; whether it clears the conversation | `reason`, `reason: clear` |

A tool call's content comes the same way: `ToolUse.input` / `.output`
(Claude's `tool_input` / `tool_response`), stored by hash with the call.
An external provider harness has no verb for the reading yet, so reads none
of these.

**Which agent session a hook came from** (`agent_session_store::resolve_tx`,
shared with OTLP exports), in order: the session the sender is (the
bearer's, the Codex observer's, the UI's interrupt's; refused with a warning
when it's another thread's); the session whose resume id is the hook's
harness session id (an open one first); the thread's newest open session,
one with a turn running first (logged as a warning — every process oxplow
starts names its session); none (an agent oxplow didn't start: recorded
with the thread's anchors only).

**One transaction per envelope** writes the state the hook changes and
the `agent.*` events that record it, anchored to the thread's stream, the
session and its open turn
(`activity_anchors_tx`). **Attribution is a best effort, by design:**
several agents, the person and outside processes can change a worktree at
once, so no rule ties every change to the right work exactly. The anchor
stays thread-scoped (the thread's own open turn), because that says
more than the stream would even when it's sometimes wrong.

- **Prompt** → with no turn open, a turn opens (`agent.turn.started@3 {
  turn, thread, session?, source, cause_turn? }`), recording **what opened
  it** as the harness read the prompt (`Prompt`, `crates/oxplow-domain/src/agent/observe.rs`):
  `source` is `person`, `background_job` (a job the agent started in the
  background reported back: Claude Code's `<task-notification>`, which
  names the `<tool-use-id>` that started it), `subagent` (a hand-back),
  `peer` (another session's message) or `unknown` (a prompt hook with no
  prompt). The turn keeps the text it was opened with, whoever's words
  they are (`agent_turn.prompt`; only `source = 'person'` is a person's),
  the harness's turn id, and **what it goes on with**: `cause_turn_id`,
  the turn whose tool call started the job (found by the tool events'
  dedupe key, `<session>:<call>:requested`) or in which the subagent
  started (its `agent.subagent.started`), `cause_basis` `reported`;
  `cause_ref`, the harness's own name for it (the call id, the subagent
  id, the sender's socket); `sender` on a peer's turn. Core never reads a
  harness's tags: the adapter classifies. **Every delivery is logged
  once** — `agent.prompt.submitted@3 { thread, turn, session, reprompt,
  source, prompt, sender?, job? }`, whether it opened a turn or landed in
  the open one (`reprompt: true`; it then opens nothing): its text in
  `event_content` (`oxplow_read` on `event:<id>#prompt`), the sender of a
  peer's message, the job a report or hand-back is for. A hand-back also
  finishes the subagent. A reader that means "a person's prompt" reads
  `source = person`; status derivation treats every delivery as the agent
  going to work. V8 rewrote the logged @2 rows (`source: unknown`: they
  were a person's prompts and background jobs' reports alike) and folded
  the old `agent.peer_message.received` into it. Pinned by
  `a_turn_records_what_opened_it_and_the_turn_it_goes_on_with`.
- **PreToolUse** → `agent.tool.requested@4` with the policy's `decision`;
  **PostToolUse** → `agent.tool.finished@4`, caused by its request. Both
  carry the harness's call id (`call`), the finish the harness's own
  timing (`duration_ms`, Claude Code's PostToolUse field). Paths, detail,
  `ok` and `exit_code` are computed here against the thread's worktree;
  input and output go to `event_content` by hash. The harness's
  `tool_use_id` is the dedupe key, so a re-posted hook logs once. **A
  call's row follows its whole life** (`tool_call.project`): it opens at
  the request, `running` (or `denied`, ended at once: it never ran),
  closes `finished` at the finish its request caused (`duration_ms`: the
  harness's timing, else since the request), and is `abandoned` when its
  turn's `agent.turn.ended` (or its session's `agent.session.ended`)
  comes with it still running, with no duration. A finish with no
  request seen (a harness that reports only the finish) still gets a row,
  with no start. Readers that mean "a tool that ran" leave out `denied`
  (`thread.checkpoint`'s `writing_tools`).
- **Stop / Interrupt** close the session's open turns
  (`agent.turn.ended@5`, with Claude's `transcript_path`), then the
  turn-end snapshot ("Snapshot tracking"). **A turn stores how it ended**
  (`agent_turn.outcome`, the event's `outcome`): `completed` (a Stop),
  `interrupted` (the person, or the harness reported an abort),
  `errored` (the harness ended it with an error: `TurnRun.outcome`, and
  its words, `name: message`, in `agent_turn.error` /
  `TurnRun.error`, never in `answer`), `session_ended` (its harness session ended
  with it open), `session_restarted` (a new harness process started with
  it open), `session_closed` (oxplow closed the agent session), `restart`
  (oxplow's recovery). A Stop whose harness names no outcome is `completed`. `answer` is only ever the agent's own words: NULL
  when it said nothing (an old turn whose end left no trace is
  `unknown`). A Stop also reports **what the turn left running**
  (`TurnRun.left_running`: Claude Code's Stop `background_tasks`, the
  running ones, `{ id, kind, description }` with the id its report back
  names), carried on its `agent.turn.ended` and recorded as jobs
  ("Background jobs"); no column holds it. A Stop also sets the turn's
  `model` and `effort` (`agent_turn`) from `HookEnvelope.turn_run`: the
  route asks the bearer's harness (`AgentHarness::turn_run(body)`: Claude's
  `effort.level` and the newest non-synthetic `message.model` among the
  transcript's last 256 KB; opencode's `model` / `variant`), and the chat
  host sets it from the observer (Codex). The same `TurnRun` carries the
  **context fill** (`agent_turn.context_tokens` / `context_window`): Claude's
  is the newest such assistant message's `input_tokens` plus cache reads
  and writes (the window is not in the transcript, so NULL); Codex's the
  last `thread/tokenUsage/updated`'s `last.totalTokens` and
  `modelContextWindow`; opencode's the Stop's `tokens` (`input` plus
  `cache.read` and `cache.write`; the bridge has no window, so NULL). A
  harness that says nothing leaves NULLs.
- **Status.** A status the hook sets (Running, Idle / AwaitingUser,
  Stopped) is logged as `agent.status.changed` when it differs from the
  newest logged one, read in the same transaction (`last_status_tx`). The
  log is the status: a restarted daemon still knows a thread waits on the
  person. `HookIngestService::set_status` (the Codex observer's waits) is the
  only other writer. One lock spans each status-deciding transaction and
  its `AgentStatusChanged` emit, so announcements reach the UI in commit
  order; the lock is per thread.

**Session tracking.** A session id a prompt or `SessionStart` names for
the first time on a thread logs `agent.session.started` once (dedupe
`session:<id>:started`) and becomes the agent session's
`resume_session_id`. A tool hook's id never does: opencode's subagent
posts under its child's id. Claude Code drops HTTP hooks for
`SessionStart` ("HTTP hooks are not supported for SessionStart" in
`claude --debug-file`), so its id is adopted from the first prompt. **A
`SessionStart` is a process start:** every one except `source: "compact"`
closes the turns the previous process left open (`session_restarted`), logs
`agent.session.started` again (`resumed: true` for the resume id) and sets
Idle, so a turn that died without a Stop reads idle after a restart, not
stalled. The Codex observer posts the same `SessionStart`. **`SessionEnd`**
closes the turns that session left open (`session_ended`; an exit
mid-turn sends no Stop), status Stopped, and leaves other
sessions' turns alone; it logs `agent.session.ended`, and `reason: clear`
of the resume id blanks it (`/clear` starts a fresh session with no HTTP
hook). A close passes the body's `transcript_path` so an interrupted
turn's tokens are counted. A PTY's exit ingests a `SessionEnd` naming its
session (`TerminalSessionRegistry::ingest_exits_into`), once.

**PreToolUse cost.** `agent_policy::may_refuse` is false for anything but
an edit, so `pre_tool_check` returns before any database read or git stat
for reads, searches, shell, MCP and subagents. The HTTP round trip still
fires for every tool (Claude's matcher is `"*"`).

**Side-band steps are best-effort by design.** Reactors that decorate a
call (collection observations, knowledge attribution) try and warn: a coverage
parse failure must never fail the hook or block the agent. The durable
writes they decorate are covered by the transactional invariants in
[data-model.md](./data-model.md).

**Hook handling is time-bounded.** Harnesses block on the answer, so the
control plane races the post-auth pipeline against 5 s
(`HOOK_HANDLING_TIMEOUT`, `bounded_hook_response`); on expiry it logs and
acks — the call allowed — so a wedged database can't stall the agent.
Availability over enforcement: MCP writes are commands, gated again on
the bus; an edit tool's file write isn't re-checked. The ingest itself
runs as its own task and finishes when the route stops waiting (its
status announcement, the pump's wake, a closed turn's end snapshot,
without which the turn would get no `thread.checkpoint`).

### Agent policy

The write guard is one policy every transport asks, not logic in the hook
route. It is isolation only; nothing waits on tracked work
([work-tracking.md](./work-tracking.md)).

**The two rules:** no thread may change another stream's worktree, and a
store's row files in a worktree are written by the store's command.
Everything else is open to every thread: its own stream's worktree, files
outside every stream, the shared knowledge pages (through the store's command:
`oxwiki.page.write`, or directly in the primary checkout), and oxplow's own
objects —
tasks, notes. The bus's `WriteGate` asks only that
a `Write` command's thread be open; `Record` commands skip it
([commands.md](./commands.md) "The pipeline").
**An agent may open a sibling agent session** in its own stream
(`oxplow.agent_session.open`): the row makes the UI attach and start its
process, and nothing types into it; its close is destructive (a
person's run asks first).

- **Pure rules** in `crates/oxplow-runtime/src/policy.rs`:
  `decide_tool(ToolIntent{label, kind, paths}, PolicyFacts)` returns
  `Allow` or `Deny { layer, reason }`. Paths are normalized first
  (`normalize_path`: `..` resolved, the deepest existing ancestor
  canonicalized), so a `..` or a symlink can't spell a guarded path as an
  outside one. With several paths, the first refused wins.
- **I/O** in `crates/oxplow-app/src/agent_policy.rs` (`Services.agent_policy`):
  `check_tool(svc, thread, intent)` gathers its stream's worktree, the
  other streams' and the project dir; `check_command(thread, spec,
  may_write)` is the bus's agent gate — an agent runs a command only when
  its spec admits agents, and a `Write` only from an open thread
  (`DenyLayer::Command`); `intent_of(tool)` makes an
  edit a worktree write of every file it names, and nothing else can be
  refused (`may_refuse`).
- **Transports only render.** The route's answer is a neutral
  `HookAnswer` (`Ack`, `Deny { reason }`, `Context { event, text }`)
  rendered by the hook's harness: the session's (`IngestOutcome.harness`),
  else an empty acknowledgement. Every built-in renders Claude Code's shape
  (`hookSpecificOutput`, `shared::render`), which opencode's bridge speaks
  too. A Codex thread's agent gets a permission reject or an
  fs error.

### Isolation

An edit (`kind: edit` — Claude's `Write`/`Edit`/`MultiEdit`/`NotebookEdit`,
opencode's `apply_patch`, a Codex thread's edit, delete
or move) is refused in two places, and nowhere else
(`crates/oxplow-runtime/src/write_guard.rs`):

- **Another stream's worktree** (the primary checkout included), for
  every thread (`write_guard::foreign_worktree_reason`, naming the path
  and the tree). Its own stream's worktree, and an absolute path outside
  every stream's (`~/.claude/plans/foo.md`), are allowed.
- **A store's row files in a worktree are written by command.** A thread
  in the primary checkout may write a file a store keeps its rows in (a
  fill's `files: { folder, suffix }`, `.oxplow/oxwiki/<slug>.md` for oxwiki):
  the store's collector converges it, and `record.read` and the touch follow
  as for any page. A thread in any other worktree is refused a write to
  `<its worktree>/<folder>/<id><suffix>` of any fill's `files:`
  (`write_guard::record_file_reason`, from the fills the running generation
  holds, so any store that declares files is covered): only the primary
  checkout's folder is watched (`.oxplow` is git-ignored), so the page would
  be silently lost. The refusal names the store and its extension, whose
  command writes to the primary checkout from anywhere.
- **Shell isn't classified.** An agent's shell commands aren't checked.
- MCP tools are never refused here: their writes are commands, gated on
  the bus.

**Why there is no writer thread.** A stream used to have one writer
thread whose agent alone could edit its worktree; the others were
read-only. It never protected the checkout: a thread runs several agents
at once, shell commands go unchecked, and a Codex TUI starts threads of
its own outside oxplow's sandbox — a demoted thread still wrote a file
through one. Closing those holes means reaching into each harness's
internals and imposing oxplow on sessions it didn't start. So a thread is
open or closed, nothing more; who works in a checkout at once is the
person's call (a new stream is the isolated option), and each agent is
told who else is there ("Who else is in the worktree"). A thread's
corner fold in the Navigator is the person's own mark
(`oxplow.thread.mark`), not a role.

### Stop hook

A Stop is never refused ([work-tracking.md](./work-tracking.md) "No
gates"). The ingest closes the turn, keeps `last_assistant_message` as the
turn's `answer` (each harness's source in "Per-harness delivery";
`hook_ingest::LAST_ASSISTANT_MESSAGE`), takes the turn-end snapshot, sets
the status — `Idle`, however the final message is worded (a question at
the end of a reply is not a wait), keeping an `awaiting_user` the turn set — and acks `{}`. A request's summary is its last turn's answer.

### Subagents

**What opencode reports and what oxplow reads** (the bridge,
`opencode-hooks.js`; `OPENCODE_FACTS` in `opencode.rs` is this table and a
test keeps the two equal; recorded with 1.17.4: the fixtures under
`crates/oxplow-harnesses/fixtures/opencode/` are replayed in
`replayed_harnesses.rs`):

| Fact | opencode reports | oxplow reads |
|---|---|---|
| A subagent starts | `session.created` with `info.parentID` | `SubagentStart` for the parent, `agent_id` the child session |
| A subagent ends | `session.idle` / `session.error` of the child | `SubagentStop` with the child's last message (and the error) |
| A subagent's result returns | a synthetic `<task id="…" state="…">` part on a new user message | `Prompt::Handback` from the child, not the person |
| A turn's id | the user message id | `harness_turn_id` (`message_id`) |
| An aborted turn | `session.error` `MessageAbortedError`, then `session.idle` | `TurnRun.outcome` `interrupted` |
| An errored turn | `session.error` with another error, then `session.idle` | `TurnRun.outcome` `errored`, `TurnRun.error` `name: message` |
| What a turn left running | the children not yet idle at the `session.idle` | `TurnRun.left_running` |
| A call's duration | the gap between `tool.execute.before` and `.after` | `duration_ms` on the PostToolUse |

**Compaction, recorded.** `/compact` posts no `chat.message`, so no
prompt and no turn of the person's. opencode adds a `compaction` part
(`auto: false` for `/compact`, `true` when it compacts by itself) in a
`message.part.updated`, the summary streams as text of an assistant
message (`mode: compaction`), then `session.compacted` carries only the
`sessionID`, then `session.idle`. The bridge remembers the part's `auto` per
session and posts it on the PostCompact as `trigger` (`manual` / `auto`;
none when no part came first); core records one `agent.context.compacted`
with it. The `session.idle` after it is a Stop with no open turn whose
last message is the summary: it changes nothing but re-marks the thread
idle. (`auto: true` is read from the schema, not recorded.)

**Subagents, recorded.** A foreground `task` call: PreToolUse Task,
`session.created` of the child (SubagentStart), the child's tools posted
as the parent's session with the child's `agent_id` (the bridge asks the
client for the session's `parentID`), the child's idle (SubagentStop),
then the Task's PostToolUse, then the parent's Stop; the Task call is the
parent's own and finishes once, after the SubagentStop. A background
`task` (`OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS`): the Task's
PostToolUse comes at once (`<task id state="running">`), the parent's Stop
carries the child in `background_tasks`, and the result comes later as a
user `chat.message` whose only part is `synthetic: true` text
`<task id="ses_…" state="completed">…`. The bridge posts it as a
UserPromptSubmit (`prompt` that text, `message_id`), then the child's
SubagentStop, then the parent's second Stop; `prompt()` reads it as
`Prompt::Handback`, so it is a subagent's turn (source `subagent`, going
on with the turn that started the child), never the person's prompt. Core
needed no change for either.

**A question is one tool call.** Recorded (`question.jsonl`): the
`question` tool's `tool.execute.before`, then `question.asked` (with
`tool.callID`, the same call, and its own `que_…` request id), then, after
the person answers, `question.replied` and `tool.execute.after`. The
bridge used to post the call twice (a PreToolUse and PostToolUse named
`question` from the tool hooks, another pair named `AskUserQuestion`
from the events, the Post under the request id), which core recorded as two
finished calls. Now the tool is posted as `AskUserQuestion` and the
question events post nothing; only `question.rejected` posts a
PostToolUse for the asking call (its tool throws, so no
`tool.execute.after` follows; not recorded, read from the source). The
`question.jsonl` fixture is the recorded hooks with the events' duplicates
removed and the tool renamed; `its_bridge_reports_an_answered_question_as_one_call`
feeds the recording's raw inputs to the bridge and gets exactly those
hooks.

**One abort or error gives two idles.** Recorded (`abort.jsonl`,
`error.jsonl`): `session.error` comes right before the first
`session.idle`, so the first Stop carries the error (`MessageAbortedError`
is `interrupted`; the 401 `APIError` is `errored`, `APIError: Missing
Authentication header`). Then a second `session.idle` follows (an
aborted bash call's `tool.execute.after` lands between the two), and the
bridge posts a second Stop with no error. Core already makes that one
turn: the first Stop ends it, the late PostToolUse is the call finishing
(it opens no turn), and a Stop with no open turn changes nothing but
re-marks the thread idle. An aborted shell call reports `exit: null`, so
its PostToolUse has no `exit_code`. The bridge's `tool_use_id` for the
late call is the same `callID` as its PreToolUse.


**Who in a session acted.** A subagent (Claude Code's `Agent`/`Task`, an
opencode child session, a Codex collab agent) is not a session: it has
no bearer and no process oxplow knows. It is who made a call inside its
parent's session: `ToolUse.subagent { id, kind? }`, on
`agent.tool.requested@4` / `agent.tool.finished@4` and
`v_tool_call.subagent_id`. The harness fills it when it can (Claude's
`agent_id` / `agent_type`, `shared::subagent_of`); else the ingest
**brackets** it: a call landing while the session's open turn has a
`subagent` call requested and not finished is that call's subagent.
`SubagentStart` / `SubagentStop` log `agent.subagent.started@2` /
`.finished@1` (dedupe `subagent-event/<session>/<id>/<phase>`), read by
`AgentHarness::subagent`. A Claude Code **background** subagent outlives
its parent's turn — its calls arrive after the parent's Stop — and hands
its report back as a prompt (`<agent-message from="<id>">…`), which
`AgentHarness::prompt` reads as `Prompt::Handback`. Status keeps a
session working while a subagent it started runs. A provider harness
answers both as its `prompt` and `subagent` verbs (feature `subagents`).
Recorded in `fixtures/claude/subagent.jsonl` and replayed
(`a_recorded_claude_code_subagent_is_attributed`).


**What Codex reports and what oxplow reads** (the chat host,
`agent_observer/codex.rs`; `CODEX_FACTS` there is this table and a test keeps the
two equal; written from the app-server protocol and the recordings in
`tests/fixtures/codex/`; an approval and an interrupted turn are recorded
(`approval.jsonl`, `interrupt.jsonl`, Codex CLI 0.166.0 / daemon 0.166.0);
a background terminal (`background_terminal.jsonl`) and a spawned child
(`subagent.jsonl`) are too, and so is a failed turn (`failed_turn.jsonl`,
an invalid API key):

| Fact | Codex reports | oxplow reads |
|---|---|---|
| How a turn ended | `turn/completed` `turn.status`: `completed`, `interrupted`, `failed` with `error.message` and `codexErrorInfo` | `completed` is a Stop; `interrupted` is `TurnOutcome::Interrupted`; `failed` is `Errored`, `error` `codexErrorInfo: message` |
| The turn's id | `turn.id` on `turn/started` (and `rootTurnId`) | `agent_turn.harness_turn_id`, set when `turn/started` arrives (also for a prompt oxplow itself sent) |
| A tool call's life | `item/started`, `item/completed` with the item `id`, `exitCode`, `durationMs` | `agent.tool.requested` at `item/started`, the finish with the same call id, `exit_code` and `duration_ms` |
| A child agent starts and ends | `collabAgentToolCall` `agentsStates`, `subAgentActivity`, `thread/started` `thread_spawn` | `SubagentStart` / `SubagentStop`, an `agent_job` row |
| The turn that spawned a child | the parent's `turn.id` (the child's `rootTurnId`) | the job's `started_turn`, basis `reported` |
| A child's tool calls | `item/completed` on the child's own thread | `ToolUse.subagent` of the child, at the finish |
| Messages inside an agent tree | `senderThreadId` on a collab item | not read |
| A background terminal | `processId` and `source` on a `commandExecution`; `thread/backgroundTerminals/list` | nothing of its own: an ordinary shell call, whose `item/completed` may land after the turn's end (recorded on the call, no job, no turn) |
| Files changed | `fileChange` paths and diffs; `turn/diff/updated` | `fileChange` paths and diffs; `turn/diff/updated` not read |
| Moderation of a turn | `turn/moderationMetadata` (Codex's verdict on the prompt and output) | not read |
| The model and effort a thread runs on | `thread/settings/updated` `threadSettings.model` / `effort`, before the turn; the open result's `thread.model` | `TurnRun.model` / `effort` on the turn's end |
| A thread that is not the session's | `thread/started` (Codex's ephemeral naming thread, no parent), `thread/status/changed` and `thread/closed` for it or for an earlier session's thread (`notLoaded`) | not read: only a message naming the session's thread, or a child it spawned, passes |
| A turn's retries and failure | `error` notifications (`willRetry`), a `warning`, `thread/status/changed` `systemError`, then `turn/completed` `failed` | only the `turn/completed` ends the turn (once, `Errored`); `error`, `warning` and `systemError` say nothing |
| Compaction | `contextCompaction` item | `PostCompact`, with no summary or trigger |

**What a shell call reports it changed.** Claude Code's Bash
`PostToolUse` can carry `tool_response.bashEditDiff { files, changedFiles,
moreFiles }` (on 2 of 4 Bash calls in `fixtures/claude/hooks.jsonl`);
`shared::claude_shaped_tool_use` reads `changedFiles` and `files[].filePath`
into the call's `paths`, so `agent.tool.finished` and the tool-call row
carry them, and the file-claim consumer claims them with basis
`reported`. A shell call without the field names no file. Codex and opencode
report nothing per shell call.

**Prompt references.** A person's prompt can carry references oxplow
inserted or the agent's picker wrote: `@<path>`, `[oxplow uri <uri>]`,
`[oxplow <uri>: "<title>" (<state>)]`, `[oxplow page <id> row: …]`. They
stay as text in `agent_turn.prompt` and are also **links from the turn**:
when the delivery is logged (`log_prompt_tx`, a reprompt's too), each
reference is read by `uri::prompt_refs::parse` — the one definition that
also spells the Rust writers (`uri_mention`, `item_mention`,
`page_row_mention`; the context basket uses them) — and stored as a
`page_ref` edge from `oxplow:oxplow/turn?id=trn<n>` to the target URI with
ref type `prompt_mention`. A turn fits `page_ref` as a source: an identity
URI, written once in the delivery's transaction, never restated. The
desktop's row mention is TypeScript; `tests/fixtures/prompt_refs.json`
pins its text in both languages. A task's page lists the turns that named
it (`v_page_ref.target_uri`, `ref_type = 'prompt_mention'`); only a
person's prompt is read. An `@path` is linked only when the file exists in
the thread's worktree (`@types/node` is no file). What the prompt's
**context** carried is linked too: the context basket's pages and sources
(`AgentContext::commit`, `context_store::take_delivered_tx`) are
`prompt_mention` links from the turn that was open when the prompt was
delivered (a delivered context item is deleted).

**`v_subagent`** (`subagent:ses<n>/<id>`, `uri::types::SUBAGENT`; its
page is its session's tab for now) pairs the two events per `(session,
subagent id)` and counts `v_tool_call.subagent_id` calls: `state` is
`running` (started, not finished), `finished`, or `unknown` — only tool
calls name it. That last one is what **opencode children give**: they bracket nothing, so they never leave `unknown`. Claude's
`SubagentStop` carries `last_assistant_message`; it is stored as event
content on `agent.subagent.finished` (`result`) and read back as
`oxplow_read subagent:ses3/<id>#result`. A background subagent's
hand-back finishes it before its `SubagentStop`, so that one has no
result (the dedupe keeps the first). The foundation page
`thread-sessions` (`oxplow_run_page { page: thread-sessions }`) lists a thread's sessions
with their subagents beneath each.

**A session's transcript** is the foundation page `session-transcript`
(`oxplow_run_page { page: session-transcript, params: { session, turn?, page? } }`): the session's
turns from `v_agent_turn` oldest first with the thread's notes where
they fall, a subagent's hand-back marked (a turn whose source is `subagent`), each
row a turn URI. A turn URI opens it at that turn (`v_agent_turn
.transcript_page`, built with `uri('oxplow/page', …)`). Its action **Resume in a new session**
runs `oxplow.agent_session.open` with `resume`: a saved conversation of
the harness, bound `chosen` in the same transaction, refused when it is
open in another session. A person's: an agent may open a session but not
choose its conversation.

Subagent tool calls reach the hooks like any other, so their edits and
test runs land in the subagent's starting turn, and so its request, with nothing from them: there is
no dispatch protocol or subagent skill.

**What to pick up next** is the page `oxtask/next-task`
(the tool `oxtask_next_task` with `thread: "thr3"`) over
`v_oxtask_ready_queue`, oxtask's dataset: an epic with its ready descendants, or every ready
non-epic item, open `blocks` links holding an item back; empty with
nothing ready or no list. oxtask's `next-task` command calls that tool and
moves the item it picks to `in_progress`, which assigns it to the thread.
Claude Code's `TaskCreate` is a within-turn planner and never mirrors
oxplow's items.

### Background jobs

**A job is a record from first sight to report** (`agent_job`,
`oxplow_db::agent_job_store`, `v_agent_job`). A job is work an agent
started that runs on after the call that started it: a background shell,
a background subagent. The hook ingest writes it in the transaction of the
hook that shows it, so a turn opened by its report finds the turn it goes
on with at once, with no projection lag:

- **Opened by** a `SubagentStart` (kind `subagent`, started in the open
  turn, `start_basis` `reported`), the call that starts it (a finished
  tool call that names the job it started, `ToolUse.started_job`: Claude
  Code's Bash `PostToolUse` `tool_response.backgroundTaskId`; kind
  `shell`, the call's id as `start_call_id`, the open turn as the start,
  `reported`), or a Stop's running work
  (`TurnRun.left_running`, Claude Code's `background_tasks`: started in
  the turn that Stop closes — oxplow's guess, `matched`). A later sight
  fills what the row lacks, and a reported start replaces a matched
  guess. A job seen running again after it reported (a subagent resumed
  under the same id) is a new row.
- **Reported by** its notice (`Prompt::BackgroundJob`: Claude's
  `<task-notification>` gives the job's `<task-id>`, the starting call's
  `<tool-use-id>`, `<status>` and `<summary>`) or a subagent's hand-back.
  The report's turn (opened, or the open one it was delivered into) is
  `report_turn_id`; a second report keeps the first's turn.
- **The report's turn goes on with the job's start**: the call the notice
  names gives the turn it was made in (the tool events' dedupe key, built
  by one `tool_key`), else the job's record does; `cause_turn_id` and
  `cause_basis` are the job's start and its basis. Its `cause_ref` is the
  job's id.
- **A subagent's stop ends its job** (its `SubagentStop`; a background
  one's hand-back may come after and fills the report's turn). **Its tool
  calls are the work of the turn that started it** (their turn anchor is
  the job's started turn), also when they land after that turn ended or
  while another turn runs.
- **Lost** (`lost_at`, oxplow's own fact, not the harness's status) when
  its agent session ends, restarts or is closed before it reported: its
  process is gone. A tool call still running then is `abandoned` too (by
  its session, for a call made with no turn open).

Claude Code's ids line up: the notice's `<task-id>`, the Stop's
`background_tasks[].id`, the Bash call's `backgroundTaskId` and a
subagent's `agent_id` are one id (`claude/subagent.jsonl`,
`claude/background_shell.jsonl`). **Every Stop is followed ~1.5 s later by
a `SubagentStop` with an `agent_id` and `agent_type: ""`** (an internal
helper; no subagent was asked for): the Claude adapter reads no subagent
from it (`AgentHarness::subagent` is `None`), so it opens and closes no
subagent record or job. A job with no report and not lost is
`running` (`v_agent_job.state`): that is what a request is waiting on.
Pinned also by `a_jobs_start_basis_resumed_runs_and_lost_jobs`. Pinned by
`a_background_job_is_a_record_from_start_to_report`,
`a_background_shell_job_opens_at_its_starting_call` and the recorded
`claude/background_shell.jsonl`
(`a_recorded_claude_code_background_shell_is_a_job_from_its_start_to_its_report`).

### Requests

**A request is the turn that started something and every turn that
followed from it** (`request`, `agent_turn.request_id`, V11): a background
job's report and a subagent's hand-back go on with the turn their job
started in, so they join its request. A turn joins its request when it
opens (`open_turn_tx` → `join_request_tx`): its cause turn's request, else
a request it starts (`request.first_turn_id`; who started it is that
turn's `source`). The cause always opened first, so membership is known at
once and never changes after. Every new turn has one; a re-prompt opens no
turn and joins nothing. A request's address is
`oxplow:oxplow/request?id=req<n>` (foundation's `request` resource,
resolving to `v_request`: who started it, with what, when); a turn's
`v_agent_turn.request` is that URI. It has no page of its own yet.

**What a request holds and where it stands are read live** (`v_request`):
its turns, threads, last activity, tool calls, tokens, running jobs, and
`state` — `waiting_on_person` (an agent session whose latest turn is the
request's waits on the person, `v_agent_session_status` `awaiting_user`:
the rail's own reading), else `running` (a turn is open),
`waiting_on_work` (a job it started has not reported, its session alive),
`finished`. Its time split sums `v_turn_activity` over its turns; its files
come with the turns' changes. Pinned by
`a_requests_state_and_totals_are_read_from_its_turns`.

**Identity and membership are stored; everything that adds up or changes
is derived.** Work recorded against a turn — tool calls (a subagent's are
its starting turn's), tokens, test runs, jobs — is its request's through
`agent_turn.request_id`: no event anchor or other row repeats it. A
cross-session message joins the sender's request when it is matched to the
turn that sent it, at the moment it opens a turn; unmatched, it starts its
own, and membership never changes after. Pinned by `a_turn_joins_its_request_when_it_opens`.

### Native features: observed, set, left native

Each harness has interactive features of its own. oxplow **observes and
shows** what it can hear, **sets at launch** what a flag or open request
takes, and **leaves to the harness** the rest — never answering for the
person: a prompt or question is answered in the harness's own TUI, which
is in the tab already ("Key invariant"). The audit behind this table was
made against Claude Code 2.1.301, Codex CLI 0.166.0 (daemon 0.166.0 → 0.167.1)
and opencode 1.17.4, from their documentation and protocol schema;
*italic* is available from the harness but not used yet, — is not
available from outside.

| | Claude Code | Codex | opencode |
|---|---|---|---|
| Waiting on a permission | `Notification` `permission_prompt` / `agent_needs_input`, and the `PermissionRequest` hook (observed, answered `Ack`) → AwaitingUser | `thread/status/changed` `activeFlags` `waitingOnApproval` → AwaitingUser, the request's command or reason as detail; cleared by the flags or `serverRequest/resolved` | the bridge forwards `permission.asked` (a `Notification`) and `permission.replied` |
| Asked a question | `AskUserQuestion` PreToolUse → AwaitingUser with its question; a `?`-ending Stop | `waitingOnUserInput` with `requestUserInput`'s first question; a `?`-ending Stop | the `question` tool, posted as `AskUserQuestion` (its PreToolUse to its PostToolUse); a `?`-ending Stop |
| Answering from oxplow | — (no interface into an interactive session; a cross-session message can't consent) | — (a request goes to the turn's client) | *HTTP `permission` / `question` reply* — left native |
| Model / effort | `agent_turn.model` / `effort`: the Stop's `effort.level` and the transcript tail's newest `message.model`; transcript `message.model` → `v_token_usage.model`; *`SessionStart.model`, `PostModelSwitch`* | `agent_turn.model` / `effort`: `Thread.model` / `reasoningEffort` at open, moved by `thread/settings/updated` | `agent_turn.model` / `effort`: the bridge's Stop carries the last assistant message's `modelID` and `variant` |
| Set at launch | `oxplow.agents.claude.model` → `--model`, `.effort` → `--effort` | `oxplow.agents.codex.model` → the open request's `model`, `.effort` → its `config.model_reasoning_effort` | `oxplow.agents.opencode.model` → `-m` (none when blank) |
| Context fill, compaction | `agent_turn.context_tokens` (window NULL): the transcript tail's newest assistant usage; `PostCompact` → `agent.context.compacted` (`trigger`, `compact_summary`); `PreCompact` resets the context baselines | `agent_turn.context_tokens` / `context_window`: `thread/tokenUsage/updated` `last.totalTokens` / `modelContextWindow`; the `contextCompaction` item → `agent.context.compacted` (resets nothing: no prompt channel) | `agent_turn.context_tokens` (window NULL): the Stop's `tokens`; `session.compacted` → `agent.context.compacted` (`trigger` `manual` / `auto` from the preceding `compaction` part) |
| Tokens | OTLP and the transcript tail | `turn/completed` deltas | — (the bridge sends none) |
| Skills, commands | shipped (`--plugin-dir`; skills reload live) | shipped (`.agents/skills`) | shipped (plugin dir, inline `command`) |
| Native worktrees | `--worktree`, subagent `isolation: worktree` → `<repo>/.claude/worktrees/<name>`; hook `cwd` follows | none (`cwd`, `runtimeWorkspaceRoots`) | none |
| Rewind | `/rewind` (files and/or conversation), no hook | `thread/revert` trims history only, `thread/reverted` | `/undo` (files and messages) |

**Left native, and why.**

- **Answering a prompt or question.** Claude Code has no interface for
  it in an interactive session; Codex sends a request to the client
  driving the turn, the TUI; opencode could take an HTTP reply, but one
  harness doesn't justify a prompt UI when the TUI is in the tab. A
  later embedded chat (the chat library) would take a person's answer as
  UI input, not oxplow's.
- **Skills and commands.** All three already get oxplow's; nothing to
  add. Claude Code reloads skills live; a command change needs its
  `/reload-plugins`.
- **Native worktrees.** Claude Code's `.claude/worktrees/<name>` sits in
  the checkout, so under isolation it is the stream's own tree: oxplow
  neither adopts nor refuses it, and an edit there is claimed like any
  edit in the stream's worktree, unless git's exclude (`.git/info/exclude`
  or a `.gitignore`) names it: then the scans and the claims leave it out,
  as for any ignored path. Another stream's agent can't edit it. A person
  can adopt it as a stream like any git worktree
  (New Stream's worktree choice lists it). Adopted, it belongs to that
  stream alone: the write guard gives a path to the deepest stream tree
  that holds it (`foreign_worktree_reason`), so the enclosing checkout's
  agent can't edit it. Pinned by
  `a_worktree_inside_a_checkout_belongs_to_the_deepest_stream`,
  `a_native_worktree_follows_git_s_exclude`,
  `an_edit_in_a_native_worktree_follows_git_s_exclude`,
  `a_native_worktree_inside_the_checkout_is_adoptable`.
- **Rewind.** Three different things: `conversation_fork` copies a
  conversation (files untouched); `oxplow.snapshot.restore_file`
  restores one file from oxplow's snapshots; a harness's rewind restores
  files with no signal oxplow hears (Codex's `thread/revert` restores no
  files at all). The turn-end snapshot and `thread.checkpoint` record
  what a rewind leaves behind.
- **Cost in dollars.** Never recorded ([metrics.md](./metrics.md)).

### Native messaging

Work moving between threads uses each harness's own messaging, which
oxplow does not see, which nobody acknowledges, and which wakes the
recipient at the sender's choice. Oxplow keeps no record of it and
delivers no message of its own. Handoffs, a record oxplow kept for this
move, were removed on 2026-11-20: they were used once, and the context
basket (Send Next to a thread's next prompt) and tasks do the useful part.

What the harnesses offer (Claude Code 2.1.301, Codex 0.163, opencode
1.17.4; "wakes" is: starts a turn, so it spends that session's quota):

| | Claude Code | Codex | opencode |
|---|---|---|---|
| Discovery | `ListAgents`: sessions on this machine that bind an inbox socket, teammates, named subagents | the `listAgents` collab tool, its own thread's tree only | none for the model; the server's `/session/:id/children`, `session.created { info.parentID }` |
| Agent to agent | `SendMessage`, same machine, plain text: an idle recipient **wakes**, a busy one reads it between tool calls; held messages expire after 5 min; no acknowledgement. Teams (experimental): a mailbox file per teammate | `sendMessage` / `sendInput` inside one thread's agent tree | none built in; a background subagent's result is pushed to its parent as a user message (**wakes**, experimental) |
| Delegation | `Agent` (foreground, background), teammates | `spawnAgent`, `followupTask`, `resumeAgent`, `wait`, `closeAgent`; each child is a thread | `task` → a child session (`parentID`); `task_id` resumes it |
| Interruption | `TaskStop`, Escape | `interruptAgent`, `turn/interrupt` | `POST /session/:id/abort` |
| Restart | subagents live on only in the resumed session; teammates aren't restored by `/resume` | `thread/resume`; children are threads | sessions persist; runners, pending permissions and questions are lost |
| Delivery from outside | inbox socket, channels (research preview), `claude -p --resume`: all **wake**; hook context doesn't | `turn/start` **wakes**; `thread/inject_items` doesn't | `prompt_async` / `message` **wake**; `message` with `noReply` doesn't |
| What oxplow can hear | an inbound message is a turn of its own (no dedicated hook); `TeammateIdle`, `TaskCreated`, `TaskCompleted` are lifecycle only | `collabAgentToolCall`, `subAgentActivity` items | `session.created { parentID }`, the task part's `{ parentSessionId, sessionId }` |

Every path that puts words in front of a model wakes it. The ones that
don't are context channels, which wait for a turn someone else starts.
None is cross-thread, visible to the person, and acknowledged.

**Nothing wakes a model.** oxplow never opens an inbox socket, a channel,
a server prompt or `turn/start` to deliver what it holds: those would
drive an agent on an interactive surface ("oxplow never drives an agent").
`source_guards::nothing_wakes_an_agent_with_what_oxplow_delivers` pins
that no production code names `prompt_async`, Claude Code's channels, or
its inbox socket (outside the harness that scrubs it from a child's
environment). What oxplow delivers reaches an agent only in the context
of its next prompt, or injected between turns where the harness allows it.

**How a Claude Code session is addressed** (checked 2026-10-16, Claude
Code 2.1.301). `SendMessage { to }` takes the session's **name** as
`ListAgents` prints it (`oxplow-main [9f6722]`: name, then a six-hex
`[uri]` that is needed only when two rows share a name, as two `slopp`
sessions do). It takes no pid. Each session binds an inbox socket,
`/tmp/cc-socks/<pid>.sock`, where `<pid>` is its `claude` process: the
agent's own environment says so (`CLAUDE_CODE_MESSAGING_SOCKET`, which
oxplow scrubs from a child's), and it is the pid oxplow records for the
session (`agent_session.pid`), so `uds:/tmp/cc-socks/<pid>.sock` is
derivable without asking the agent. An inbound message names its sender
twice, `from="uds:/tmp/cc-socks/<pid>.sock"` and
`from-name="<name>"` (the name `ListAgents` shows); Claude Code's own
instructions to reply are to copy `from` as `to`, so the socket form is an
address too (documented, not exercised here). A name is not an identity:
it can repeat and the person can change it, and nothing in a hook body
carries it. What oxplow can learn today: the socket (from the pid) always,
the name only from a message the session receives from that sender
(`Prompt::PeerMessage { from }`). Learning a session's own name is still
open (no hook field was found).

**Native messaging stays native, and is observed:** Claude Code's inbound
cross-session message arrives as a prompt wrapped in
`<cross-session-message from="<socket>" from-name="<session name>" …>`.
The Claude harness's `prompt` reads it as `Prompt::PeerMessage { from,
socket, text }` (the session name, else the socket), not the person's
words, as it reads a subagent's hand-back. The control plane passes it as
`HookEnvelope.prompt`; the ingest opens a turn with source `peer` and the
message as its text when none is open (none if one is), logs the
delivery (`agent.prompt.submitted`, `source: peer`, `sender`) and sets the
session running.
`v_agent_turn.sender` names the sender of a turn it opened (its socket in
`cause_ref`), and Thread activity (foundation) titles that turn "Message from …".
**The receiving turn is matched to the turn that sent it** (V15,
`agent_message`, `oxplow_db::agent_message_store`): the Claude adapter reads
a `SendMessage` call's recipient and text (`ToolUse.sent`), recorded with
its turn when the call is requested; on receipt the adapter gives the
message without its wrapping (`Prompt::PeerMessage.message`), and the one
unmatched send of the same text (content hash) in the last ten minutes —
recorded once per call (with the call's id, V17), only when the policy let it
run and only with text, and not a send whose tool-call row says it was denied,
abandoned or failed — is the cause (`cause_basis` `matched`): the receiving turn joins the sender's
request, across threads. No send, or two of the same text, links nothing,
and the turn starts its own request. Known limits: a message read by a busy
session between tool calls opens no turn (no hook was seen to fire for it);
a held message expires after five minutes unacknowledged, so a send does
not prove a receipt; a sender outside oxplow is known by its name and
socket only. Pinned by `a_peers_message_is_matched_to_the_turn_that_sent_it`, `a_message_is_matched_across_threads` and `a_send_that_never_ran_is_not_a_candidate`. Pinned by
`a_peers_message_opens_a_turn_of_its_own_with_its_text`, `a_peers_message_is_not_a_persons_prompt`
and the recorded `claude/peer_message.jsonl`
(`a_recorded_claude_code_peer_message_opens_a_turn_of_its_own`).
The **idle notice** a `SendMessage` with `notify_when_idle` asks for
arrives as a prompt starting `[Cross-session idle notice] "<session name>"`
(not `[idle notice]`): the adapter reads it as a `PeerMessage` from that
session (no socket, no message), never a person's prompt
(`an_idle_notice_is_a_peers_and_not_a_persons_prompt`). It has no text
to match on, so it is linked by who it names: a `SendMessage` with
`notify_when_idle` says so on its `agent_message` row (V27:
`notify_when_idle`, and `idle_notice_turn_id` once answered; `recipient` is
the call's `to`), and the notice joins the turn of the one unanswered ask to
that session name within an hour (`cause_basis` `matched`; none or two for one name link nothing, and a
denied or failed call is no candidate). Pinned by
`an_idle_notice_is_matched_to_the_send_that_asked_for_it` and the recorded
exchange. A send to a recipient given as a session id, not its name, is not
linked. Recorded from the
sender's side in `claude/cross_session.jsonl`
(`a_recorded_claude_code_cross_session_exchange_is_not_the_persons_prompts`);
the reply is `<cross-session-message from="uds:/tmp/cc-socks/<pid>.sock"
from-name=… from-mode="prompting">`. A `[Cross-session delivery notice]`
(a recipient in another permission mode) has not been recorded. Codex's
collab agents are subagents ("Collab agents are subagents"). Observe
only: oxplow never sends a peer message and doesn't read Claude's team
mailboxes (not a documented interface).

Cross-thread ownership and duplicate work are visibility, not gates
([work-tracking.md](./work-tracking.md) "Cross-thread ownership").

### Validated against, and what pins each scenario

The session integration is checked against these versions: **Codex CLI
0.166.0 / daemon 0.166.0 → 0.167.1** (the approval, interrupt, background-terminal
and subagent recordings; older fixtures are 0.162.1 / 0.163), **Claude Code 2.1.322**, **opencode 1.17.4**; the
recorded fixtures (`crates/oxplow-harnesses/fixtures/{claude,opencode}`,
`crates/oxplow-app/tests/fixtures/codex/app-server.jsonl`) were taken at
the versions current when recorded. A harness declares what it supports
as features and answers "unsupported" for the rest
(`a_built_in_harness_answers_unsupported_exactly_for_what_it_doesnt_declare`),
so a failure is attributable to a declared capability, not assumed
equivalence between tools.

| Scenario | Pinned by |
|---|---|
| Native title changes | `a_slot_records_its_conversations_and_their_titles` (Claude's `session_title`); Codex's `thread/name/updated` in `a_prompt_in_the_tui_is_a_recorded_turn`, the name on resume in `a_new_thread_is_saved_so_the_tui_can_resume_it`; rename: `a_native_rename_is_a_thread_name_set` |
| Binding changes (started, switched, forked, chosen) | `a_slots_bindings_are_its_conversations_in_order`, `a_person_resumes_a_chosen_conversation`, `a_fork_opens_a_new_session_and_leaves_the_source_alone`, `a_fork_launch_binds_the_new_session_not_the_source`, `a_child_sessions_tool_hook_leaves_the_resume_pointer` |
| Child-agent attribution | `its_subagents_are_read_from_their_hooks` (recorded `claude/subagent.jsonl`), `its_bridge_sends_the_call_id_the_exit_code_and_a_childs_parent` (opencode), `a_call_inside_a_subagent_call_is_the_subagents`, `a_background_subagent_hands_back_without_a_prompt` |
| opencode compaction | recorded `opencode/compaction.jsonl`: `a_recorded_opencode_compaction_is_recorded_and_opens_no_turn`; the bridge: `its_bridge_names_what_triggered_a_compaction` |
| opencode subagents | recorded `opencode/foreground_subagent.jsonl`, `opencode/background_subagent.jsonl`: `a_recorded_opencode_foreground_subagent_is_attributed`, `a_recorded_opencode_background_subagent_hands_back_without_a_prompt` |
| opencode question | recorded `opencode/question.jsonl`: `a_recorded_opencode_question_is_one_tool_call_and_one_wait`; the bridge: `its_bridge_reports_an_answered_question_as_one_call`, `its_bridge_reports_a_wait_on_a_permission_or_a_question` |
| opencode abort and API error | recorded `opencode/abort.jsonl`, `opencode/error.jsonl`: `a_recorded_opencode_abort_ends_one_turn_as_interrupted`, `a_recorded_opencode_api_error_ends_one_turn_as_errored` |
| Unsupported capability | the conformance test above; `a_harness_without_a_list_says_so`; `a_native_rename_needs_the_feature_a_name_and_a_running_observer` |
| Live client reconnect | `an_agent_session_is_reattached_not_spawned_again`; the pane's reopen (`terminalSession.test.ts`); Codex: `a_restarted_daemon_is_observed_again_on_the_next_open` |
| Backend restart | `a_resumed_session_resets_a_turn_that_died_without_a_stop`, `a_restart_close_is_logged_as_recovery`, `a_process_exit_ends_its_session_once`; a Codex session end to end, `a_codex_session_resumes_its_thread_after_a_backend_restart` (a fake `codex` on the search path, the scripted daemon, Services reopened over the same database file: the bound thread is resumed, not replaced, and opens writable) |
| Lazy launch | `mayStartNow` (`terminalSession.test.ts`); `opening_adds_a_slot_and_undo_closes_it` (opening starts nothing) |
| Missing vs inaccessible transcript | `a_missing_transcript_is_still_resumed_and_flagged`, `resume_state_reads_the_transcript_dir`, `an_unreadable_transcript_dir_is_unknown_not_missing` |
| Custom config namespaces | `the_transcript_is_looked_for_in_the_effective_config_dir` (`CLAUDE_CONFIG_DIR`), `codex_home_follows_the_environment_when_set` (`CODEX_HOME`), `the_persons_mcp_servers_stay_unless_strict_is_asked_for` (`agentConfig.<harness>`) |
| Native new / resume / fork | the harness fork tests (`a_fork_resumes_a_copy`, `a_fork_continues_a_copy`, `a_saved_conversation_resumes_or_forks_its_thread`), `a_person_opens_a_session_on_a_chosen_conversation` |
| Duplicate ids | `a_conversation_bound_in_another_open_slot_is_found`, the refusals in `a_person_resumes_a_chosen_conversation` and `a_person_opens_a_session_on_a_chosen_conversation` |
| Two-window tab context | `two_windows_each_report_their_own_page`, `tabs_are_listed_per_thread_with_selection_visibility_and_staleness` |
| Stream / worktree assignment | `no_thread_edits_another_streams_worktree` (a thread never writes another stream's worktree, the primary checkout included), `an_agent_opens_in_its_own_stream_and_its_close_is_proposed` |
| No terminal input from restore, metadata, tab discovery or delivery | `terminal_input_has_single_production_caller` and `no-agent-input-automation.test.ts`: the one way bytes reach a PTY is the person's keystrokes |

Not yet covered: a recorded Claude hook carrying `session_title` (filed).

### Plans

A plan the agent puts to the person is not recorded. Claude Code's
`ExitPlanMode` is still a `Plan` tool call (`ToolKind::Plan`), so the
thread waits on the person until it is answered ("Agent status"); its text
is not kept. Plan capture (`agent.plan.proposed`, `v_agent_plan`, Codex's
`plan` item as a `PlanProposed` hook) was removed on 2026-11-20 with the
thread Plan part, its only reader; V52 deleted its events.

### Agent status

**Status is per agent session.** Each session has its newest
`agent.status.changed` (anchored to it) and a status derived from its own
activity; a thread's activity no session claims has one too. A thread's
status is its sessions' roll-up and a stream's is its threads':
`oxplow_domain::agent::roll_up_status`, the desktop's `rollUpAgentStatus`
and the `v_agent_status` model share one rule and one truth table
(`crates/oxplow-domain/fixtures/agent_status_rollup.json`). The ranking is
what the person owes first: `awaiting_user > stalled > running > error >
stopped > idle`. A dead turn means the person owes the next move, so it
outranks work in flight. `v_agent_session_status` holds each open
session's logged status; `AgentStatusChanged` carries the session.

**Derivation.** `derive_session_status`
(`crates/oxplow-app/src/agent_status_derive.rs`) reads the session's
newest 200 `agent.*` events (`RECENT_ACTIVITY`,
`SqliteEventLogStore::recent_in_session`) as `Activity`: prompts (every
one), allowed tool requests (a refused request opens no tool), tool
finishes, turn ends, session starts, and status changes (`awaiting_user`
parks the session until a prompt or another status moves it). The hook
ingest logs `awaiting_user` for a Stop whose final message ends in a
question (its last line is the detail), a pending call that waits on the
person (`kind: ask` or `plan`, until its PostToolUse), and Claude's
`Notification` with `notification_type` `permission_prompt` or
`elicitation_dialog` or `agent_needs_input`, and Claude's
`PermissionRequest` hook (the detail names the tool; registered to observe
only, so its answer is always `Ack` and Claude's own dialog decides).
Codex's observer logs it from `thread/status/changed` active flags
(`waitingOnApproval`, `waitingOnUserInput`; the detail is the request's
command, reason or first question) and clears it on the cleared flags or
`serverRequest/resolved`. opencode's bridge forwards `permission.asked`
(a `Notification`; `permission.replied` becomes a tool-less `PostToolUse`
that clears the wait). Its `question` tool is posted as `AskUserQuestion`,
so the wait runs from the call's PreToolUse to its PostToolUse.
Codex's approvals in its TUI log it through `set_status`.

**`permission_prompts`** means: oxplow learns when the agent waits on a
permission or a question, and on what. Observe only: nothing answers or
decides for the person. The observe conformance checks it for a harness
that declares it (`observe_conformance::waits_on_the_person`; the
`permission.jsonl` fixtures of Claude and opencode are written from the
documented hooks, not recorded).

- **Subagents in flight.** Unreturned `subagent` calls (and the reducer's
  `running_subagents`) keep a Stop from flipping the session to idle
  while a subagent works.
- **Waiting on the person.** Unreturned `ask` / `plan` calls
  (`ToolKind::waits_on_person`: `AskUserQuestion`, `ExitPlanMode`) make
  the derived state `AwaitingUser`, exempt from the stall thresholds:
  their PostToolUse only arrives once the person answers, and no Stop
  fires meanwhile.
- **Interrupts.** Claude Code doesn't reliably fire `Stop` on an Escape
  cancel. The terminal pane calls `onUserInterrupt` on a plain Escape and
  the session page records a synthetic `Interrupt` for the session
  (`recordUserInterrupt`, the `ingest_hook_event` RPC): the open turn
  closes `interrupted` and the status is Stopped, which clears open
  tools and subagents. Only while a turn is open: an Escape that closes a
  menu with no turn running changes nothing, so a thread waiting on the
  person still is. One during a turn can't be told from a cancel; the
  agent's next hook reopens a turn.
- **Stalls.** Claude Code emits no hook when a turn dies on a transient
  API error or an unavailable model, and drops back to its prompt.
  `derive_session_status(events, now)` degrades a derived Running whose
  newest event is older than a threshold to `Stalled` (derived only, never
  logged): `AGENT_STALL_AFTER_MS` (15 min) while a tool call is open (a
  Bash may run silently up to 10 min), `AGENT_DEAD_AFTER_MS` (5 min) when
  none is. **PTY output is a second liveness signal:** one long turn can
  stream tokens for many minutes with no hook, so
  `derive_session_status_with_activity(events, last_output_at, now)`
  measures silence from the later of the newest event and the session's
  last PTY output (`output_activity::OutputActivity`, per agent session,
  in memory, stamped by the terminal forwarder for agent panes and by the
  Codex observer); both must go quiet. Since no hook arrives to re-derive,
  `AgentStallWatch` (`agent_stall_watch.rs`, spawned at boot) re-derives
  every open session once a minute and pushes `Stalled`. It raises
  nothing about in-progress tasks: one left in progress while its agent
  is idle is normal.

**In the UI** (`AgentStatusDot`): working is a pulsing yellow dot, waiting
(idle, ready for the next prompt) a still grey one, stalled a pulsing red
one ("agent exited or errored mid-turn — re-run"), awaiting a pulsing
blue one with the question as its tooltip. To know a turn finished, watch
for the transition out of working; terminal rows are unreliable
(scrollback, progress lines).

### Snapshot tracking

oxplow keeps a content-addressed history of each worktree so what changed
per turn is known without git. The store and its
guarantees are [data-model.md](./data-model.md) "snapshot +
file_snapshot"; this is when takes happen. Every take is a mark of the
active snapshots implementation ([work-tracking.md](./work-tracking.md)
"Capabilities").

- **Dirty set.** A per-stream in-memory set of changed paths, fed by the
  fs watcher and the PostToolUse hook. A take drains it; only those paths
  are re-stat'ed. `request_snapshot` waits `DEFAULT_PREDRAIN_DELAY`
  (300 ms) first so the watcher's 250 ms debounce delivers in-flight
  events. There is one `SnapshotCaptureService` per stream
  (`SnapshotCaptureRegistry`), each watching its own worktree.
- **Turn end.** When a turn closes (Stop or interrupt, every harness
  through `HookIngestService::ingest`), `turn_snapshots::CaptureTurnSnapshots`
  runs a `turn_end` take anchored to the turn and its thread
  (`snapshot_op.turn_id`). **What the turn changed is its files**
  (`turn_file`, a time join with the stream's snapshot line), not the
  take's op parent, which other takes during the turn (a commit, another
  thread's turn end) move. One take per Stop, for the newest turn it closed
  (`AgentTurnStore::close` reports whether it closed anything), after the
  status is set so the UI isn't held on running.
- **The budget.** The Stop waits at most `snapshotTurnBudgetMs` (default
  2000, min 100; an agent may set it). A slower take is never aborted: it
  finishes in the background and its op and `snapshot.taken` record
  `over_budget`. The clock starts before the per-stream take lock.
- **Turn events.** `agent.turn.started@3 { source, cause_turn? }` /
  `agent.turn.ended@5 { outcome, left_running?, error? }` (outcome as above), in
  the same transaction as the `agent_turn` row.
- **Quiet period.** The worktree's snapshots do not depend on turns: a
  watcher change arms a deadline (`DEFAULT_QUIET_PERIOD`, 3 s, debounced);
  when it fires with paths dirty, a `quiet` take records them, whoever
  changed them and whatever turns are open. A take another request made
  meanwhile has emptied the set, and the firing does nothing.
- **No brackets.** An unchanged tree records an op on the current
  snapshot instead of a new row. Agents never flush explicitly.
- **Boot and HEAD.** The primary stream's startup sweep
  (`enqueue_startup_diff`, then a `startup` take) records what changed
  while oxplow was down; a move of this worktree's HEAD (not a ref moving elsewhere in the repository) runs a `git_refs` take, then on
  a clean tree a `head_moved` re-stamp, and logs `vcs.head.moved@3`.

### Files a turn changed

A turn's files are recorded as the work happens, never declared
([work-tracking.md](./work-tracking.md) "The record").

- **Claimed: edits.** On `agent.tool.finished` every file an edit names
  is a path of the tool call (`agent_tool_call_path`, basis `named`; a
  shell call's `bashEditDiff` files are `reported`). Paths are relative
  to the thread's worktree; an absolute path (outside it) is never
  claimed. The same event feeds `tool_call.project` (the
  `agent_tool_call` row) and `knowledge.attribution` (an edit of an indexed
  `.oxplow/oxwiki/<slug>.md` marks the page touched by the thread).
- **Everything else a turn changed** (shell, codegen, formatters) comes
  from the time join of the turn with the stream's snapshot line
  (`v_turn_file`): no step records it per turn, and changes between
  turns (the person's own) belong to no turn. Nothing is reconciled at
  close.
- **A run is the turn its causing tool call was in.** Tests, analysis
  and coverage are recorded whether or not anything else is going on, and
  are the turn's request's. `oxplow.test.record_run` is only for counts oxplow
  couldn't parse.

Claims carry the agent session where one is known
([work-tracking.md](./work-tracking.md) "Requests, files and assignment").

### Token usage capture

**OTEL (Claude Code).** The control plane's `/v1/metrics` OTLP
receiver (`handle_otlp_metrics`, behind the session's bearer) hands each export to `otlp_ingest.rs`, which asks every
registered harness's `token_readings` once per export and logs one
`agent.tokens.reported` event anchored to the turn it measured; the
`token_usage.otlp` consumer turns it into `oxplow.tokens` facts (cache
kinds on `oxplow.cache_tokens` / `oxplow.cache_usage`; tokens only, never
dollars). Claude Code's counter is `claude_code.token.usage`. A Codex
observer's tokens ride its Stop instead ("Codex: an
app-server client"). Details: [metrics.md](./metrics.md)
"OTEL token tracking".

**Per-turn rows (the transcript).** For a harness with `transcript`
(Claude Code), the `token_usage.turns` reactor (`TurnTokensConsumer`,
`crates/oxplow-app/src/token_usage.rs`) reacts to `agent.turn.ended@5`
with `TokenUsageService::on_stop`; nothing is parsed in the hook. It reads
the per-session cursor (`agent_token_cursor`), takes only the complete
lines past it, and the harness's `turns(tail)` splits them into one
`Turn` per agent turn: the person's prompt that opened it plus the summed
usage and `model` of the assistant messages answering it (tool-result
user messages fold into the current turn; a message counts once by its
id). Each turn becomes one `agent_token_usage` row (provenance
`observed`) on the oxplow turn it ran in (so on its request), and the cursor advances in the same transaction
(`record_batch`), so a redelivered turn never reads the same bytes twice.
**Bootstrap:** with no cursor yet (a fresh daemon, or the first Stop on a
long transcript) it seeds the cursor at the last complete line and
records nothing, rather than lumping the whole history into one row; the
turn in flight at that first Stop goes uncounted. The Codex observer's counts
come on the Stop body instead (`TURN_USAGE_KEY`) and are recorded as one
row keyed by the event (`agent_token_usage.cause`).

**Prompt capture is observation only.** The prompt text is read from the
transcript the agent wrote — what the person typed into the real CLI.
There is no second input box and no "send prompt" tool. Everything reads
the rows through `v_token_usage`: the bundled `task-tokens` page
(`oxplow/task-detail.sections` slot), foundation's Thread activity (per
turn) and the usage pages. Tables: [data-model.md](./data-model.md).

MCP initialization briefly describes how to query data, run commands
and open pages. Tool availability is separate from how the agent chooses
to answer the person.

### Collection command and skill

The `configure` command (`crates/oxplow-agent-text/assets/configure.md`;
`/oxplow:configure` in Claude Code) has the agent instrument the
project's test tooling to write standard-format reports at stable paths,
then records the `testing:` block in `.oxplow/project.yaml` and one report
collector per report in the project's extension
(`.oxplow/extensions/project/extension.yaml`). The `oxplow-collection`
skill describes how report collection works when the project wants it.
A missing report can leave a one-shot notice for the person in Alerts.
The integration makes the project's test commands available through
configuration. Repository guidance can describe when to use them.
[Collection](./collection.md)

### Nudge persistence

The notices and advisory hits (kind
`<namespace>/<advisory id>`) are written by pump
reactors (`collection`, `advisories.post_tool` and `advisories.post_tool.immediate` in
`post_tool_reactors.rs`; prompt and turn-end advisories too) to the
`agent_nudge` table (`crates/oxplow-db/src/agent_nudge_store.rs`) with
the kind, message, trigger, turn and **cause** (the event, so a
redelivered event can't fire the same kind twice). Missing-report notices
use the person audience. For agent-directed advisories,
`post_tool_context` checks the active agent advisories and waits only when
one can respond to the recorded tool event. It watches the matching consumer's
checkpoint through that event, up to 2.5 s. Advisories that name an `after:`
run in `advisories.post_tool`, which waits on the immediate consumer and on
what their targets run in (`advisories::ordered_gates`); those with none run
in `advisories.post_tool.immediate` once the call is projected. Collection and person-facing
notices run in the background. The hook takes the thread's undelivered nudges
(`take_for_agent`, which stamps `delivered_at`; oxplow's own kinds before
advisories; at most `NUDGE_BUDGET`, 2000 characters, per hook), and
`prompt_context` takes them the same way. One that lands after its hook
answered reaches the agent on the thread's next tool call or prompt.
Nothing is taken for a session whose harness lacks the hook's channel:
its nudges stay undelivered rather than being stamped and lost. **One-shot marks
are durable** (`claim_once`, per request or thread): the report-less
nudge fires once per request, an advisory once per its `once_per` scope,
across restarts. A person reads what oxplow told the agent in the bundled
nudges page ("Agent Nudges", `oxplow/changes.sections` slot).

### Measuring context and hook overhead

For a dev sample, quit the running app and use this command.

```sh
RUST_LOG='info,rmcp=warn,oxplow::agent_overhead=debug' bun run run:dev
```

Filter `.oxplow/logs/daemon.*.log` for `oxplow::agent_overhead`. This
target is off at the default log level. Records carry byte counts, timings
in microseconds, and routing labels such as harness and reactor name.
They carry no prompt, tool, or context contents.

- `launch_prompt` measures assembled launch text and its contributing
  sections. `mcp_instructions` measures the instructions advertised
  during MCP initialization, where the harness can be unknown.
- `prompt_context` measures context prepared by the prompt hook, with
  separate costs for session context, changed extension context, the
  context basket, advisories, and reading pending nudges.
  `prompt_commit` records the bytes marked for delivery and the time spent
  updating delivery bookkeeping. The terminal hook does this before writing
  its response. The chat host does it after accepting the context.
- `post_tool_context` measures advisory eligibility, time waiting for the
  selected consumer, time reading pending nudges, and returned bytes.
  `waited_for` identifies the consumer when the hook waited. `settled=false`
  means its checkpoint did not reach the tool event within the hook's wait.

Total `bytes` fields use the returned UTF-8 text. Section lengths can
differ from the total because the total includes formatting and trimming.
Harness-owned instructions, tool schemas, and model token counts are
outside these measurements.

### Observe conformance

What core records of a harness's session is checked the same way for
every harness: `oxplow_app::observe_conformance::suite(svc, &Expect {
harness, thread, edited, tokens })` reads the thread's events and checks
the canonical stream — the session started under the harness's key
(`agent.session.started@3`), a prompt opened a turn, the edit was
requested and finished with its worktree-relative path, the turn ended
completed, and the exported tokens were read (`agent.tokens.reported`).
No findings passes.

- **Live, over the fake harness** (`crates/oxplow-harness-fake`; `-fake`
  keeps it off every shipped binary). `tests/observe_conformance.rs` in
  the control plane registers it, opens a session, runs its `launch()`
  command, and its binary posts a scripted session to the live hook route
  and OTLP receiver with its launch's identity. The fake renders its own
  answer shape (`{"fake": …}`) and fails on any other, so a pass also
  shows every answer was the harness's to render.
- **Every built-in renders the golden deny**
  (`hook_goldens.rs::every_built_in_harness_renders_the_golden_deny`,
  byte-equal to `pre_tool_write_guard_path.json`).
- **The real harnesses, replayed.** One session each of Claude Code
  and opencode, recorded with `OXPLOW_HOOK_DEBUG`, is checked in at
  `crates/oxplow-harnesses/fixtures/<harness>/hooks.jsonl` (`{event,
  payload}` per line; paths scrubbed to `<ROOT>`, `<HOME>`, `<TMP>`;
  `claude/subagent.jsonl` is a background-subagent session,
  `claude/background_shell.jsonl` a background shell, `claude/cross_session.jsonl`
a cross-session exchange; its ListAgents listing is an invented stub).
  `tests/replayed_harnesses.rs` posts each in order to a live route as a
  session of its harness, its own mapping and answers running, and the
  suite checks the record (tokens aren't replayed). When an agent's hook
  shapes change, record a new session, filter it by its `session_id`, and
  replace the fixture: the replay is what notices.

## Pitfalls

- **The write guard refuses edits into another stream's worktree.** An
  agent reporting "permission denied" on a write there is the guard
  working: do it from a thread of that stream.
- **Several agents may share a checkout.** Two agents in one worktree
  (one thread or two) can step on each other's files, builds and commits;
  each is told the other is there, nothing stops them.
- **Filing work doesn't start an idle agent.** Someone has to prompt it
  ("What we can't do from oxplow hooks").
- **oxplow never commits on its own.** No commit at Stop, no commit
  points. Commits, pushes and checkouts are the `oxplow.vcs.*` commands
  (a person's in the app, an agent's through `run_command`) or plain
  `git`. Shipped oxplow takes no position on whether an agent should
  commit; a person says so in their own instructions.
- **opencode's hooks are not Claude's.** opencode has no
  `SessionStart`, `SessionEnd` or `Notification`. Anything keyed on those
  events needs another path (a PTY exit ingests `SessionEnd`).
- **The launch prompt is read once.** What it carries (custom prompts,
  `agentPromptAppend`, the testing hint) reaches a running agent only on
  its next launch; the session context, by contrast, is rebuilt on every
  prompt.

## Harness-injected system-reminders (not ours)

Some reminders come from the Claude Code harness itself and can't be
suppressed from a plugin: the "task tools haven't been used recently"
nudge about `TaskCreate` / `TaskUpdate`, and the "user opened the file X
in the IDE" note. No hook, env var or plugin setting silences them;
revisit if Claude Code adds one.

## Related

- [work-tracking.md](./work-tracking.md) — requests, files, assignment and hints
  built on this record.
- [commands.md](./commands.md) — the command bus every agent write goes
  through.
- [data-model.md](./data-model.md) — the tables and models the agent
  reads.
- [providers.md](./providers.md) — a harness as a provider process.
- [terminal.md](./terminal.md) — the PTY pane and resume outcomes.
- [ipc-and-stores.md](./ipc-and-stores.md) — adding an operation to both
  surfaces.
