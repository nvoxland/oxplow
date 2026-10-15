# Agent execution model

How oxplow and a coding agent (Claude Code, Codex, opencode, an ACP
agent) meet: how a session is launched, what every agent is told, the
tools it calls, what oxplow observes of it, and the rules that keep one
thread from clobbering another's worktree. How oxplow turns that record
into efforts is [work-tracking.md](./work-tracking.md); the commands an
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

**Driving is an interface only.** `oxplow_domain::agent::drive::Drive`
(`programmatic`, `prompt`) is how oxplow or an extension would one day
prompt an agent, and only on a surface licensed for it (API-funded; a PTY,
or an ACP agent signed in with a plan, never is). Nothing implements,
holds or calls it: `source_guards::nothing_in_core_drives_an_agent` fails
on production code outside `drive.rs` that names it, so the first driver
is a decision, not a drift.

### No synthesized agent terminal input (no automation)

**The agent's input has one source: the person.** oxplow never
generates, injects or auto-answers agent input. The agent makes its own
tool calls and is driven by a human, never by oxplow typing at it.
Synthesizing input would be automating the agent CLI, which risks its
(Claude Code / Codex / opencode) license terms.

- The renderer's xterm (`TerminalPane.tsx`, its ordered sender
  `components/terminalInput.ts`) pipes the person's keystrokes, paste,
  scroll and resize to the PTY through `forward_terminal_input`
  (→ `terminal_sessions.send` → `pty.write`). It is the human-input
  transport, not an agent-messaging API, and it must stay: in remote mode
  the PTY is on the daemon, and browser keystrokes reach it only this
  way.
- `forward_terminal_input` is UI-only: never on the MCP surface
  (`oxplow-surface-parity`'s `ui("forward_terminal_input")` row), so an
  agent can't type at itself or a peer.
- An ACP agent's prompt is built only by `acp/human_prompt.rs`
  `compose`, from the prompt box ("ACP agents: sessions").
- Drag-and-drop and "Add to agent context" (`agent-input-bus.ts`) are a
  person's gesture: the visible pane pastes the text, or the ACP prompt
  box appends it to the draft.
- Steering (session context, nudges) reaches the agent through hook
  answers its own harness injects, never through the terminal.

**Guards** (each fails the build):
- `apps/desktop/src/no-agent-input-automation.test.ts` confines
  `forwardTerminalInput` and `{type:"input"}` construction to
  `TerminalPane.tsx`, `components/terminalInput.ts` and the `api.ts`
  facade, and `acpPrompt` to `AcpPromptBox.tsx`.
- A Rust source scan in `crates/oxplow-rpc/src/commands/terminal.rs`
  asserts the only production caller of `terminal_sessions.send(` is the
  `forward_terminal_input` core.
- `acp/guard_tests.rs` pins the ACP prompt path ("ACP agents: sessions").

### What we can't do from oxplow hooks

- **Hooks add context; they can't remove it.** A harness's own
  `<system-reminder>` blocks (Claude Code's "the task tools haven't been
  used recently") can't be edited out. A counter-instruction would leave
  both visible, which is worse ("Harness-injected system-reminders").
- **The first turn needs a person's prompt.** "Never prompts" holds for a
  cold start too: an idle agent stays idle when work is filed. A person
  sends the first prompt.

## The four seams

An agent session runs a **harness** (`AgentHarness`,
`crates/oxplow-domain/src/agent/harness.rs`). oxplow meets the agent at
four seams:

| Seam | What crosses it | Where |
|---|---|---|
| Launch | The process: its command or ACP program, env, runtime files, system prompt, resume | `AgentHarness::launch` → `LaunchSpec::{Pty, Acp}` ("Launching the agent") |
| Observe | Hooks, tool calls, transcripts and token telemetry, mapped into oxplow's vocabulary and logged as `agent.*` events | `tool_use`, `prompt`, `subagent`, `turns`, `token_readings`, `render` ("Observing") |
| Interact | How the person works with the session: the harness's own terminal, or oxplow's structured transcript (ACP) | `AgentHarness::interact()` (`Transcript::Terminal` / `Structured`); never something core invokes |
| Drive | oxplow prompting the agent | `drive::Drive`, an interface nothing implements |

The agent's MCP tools ("The agent's tools") are the one channel the agent
itself starts.

## What every agent is told (the text contract)

Every agent session, whatever its harness, is told the same things; a
harness only delivers them.

| Piece | What | Where it comes from |
|---|---|---|
| Preamble | How to work in oxplow: nothing to file, end with the question, reading the errors a person saw, the references oxplow puts in a prompt (`@<path>`, `[oxplow ref …]`, `[oxplow work_item: …]`, `[oxplow lens …]`) | `oxplow_agent_text::WORKING_IN_OXPLOW` (`crates/oxplow-agent-text/assets/working-in-oxplow.md`) |
| Launch prompt | The preamble; the session context only for a harness without `prompt_context`; the thread's and stream's custom prompts; `agentPromptAppend`; the testing hint (`testing.agentHint`); the skill index only for a harness without `runtime_text` | `agent_prompt::assemble_system_prompt(config, stream, thread, harness, skills)`, given the session's declared harness (`crates/oxplow-app/src/agent_prompt.rs`) |
| Skills | Core's (`oxplow-runtime`, `-wiki-capture`, `-mermaid`, `-collection`, `-metrics`, `-extension`, `-codebase`) and the active extensions' | `AgentText` (`oxplow_agent_text::core_text` + `capabilities::agent_text`) |
| Slash commands | Core's (`review-comments`, `configure`, `new-metric`) and the active extensions' | Same |
| Session context | Stream, worktree, branch, thread, access, the writer's agent, a role change | Prompt-time context |
| Decisions | The open effort's recorded ones | Prompt-time context |
| Nudges | Advisory hints, the collection nudge | Prompt- or tool-time context ("Nudge persistence") |
| Server instructions | What oxplow knows, writes are commands, answer with `oxplow_show_lens` | MCP `get_info` ("The agent's tools") |

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
`oxplow-wiki-capture`; waiting on the person: `oxplow-runtime`;
foreground tests: `oxplow-collection`; `v_model`/`oxplow_query_sql`:
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
wait in the store. Claude Code, Codex and opencode declare both; an ACP
agent declares `prompt_context` only.

**Once, then on change.** On every prompt, `prompt_context` builds the
`<session-context>` block and the decisions block and sends each only
when it differs from what that agent session was last sent (`should_emit`,
keyed by the harness's session id; the decisions under
`<session_id>#decisions`, at most `MAX_DECISIONS_IN_CONTEXT` (15),
recorded ones only — inferred decisions are the reviewer's). The first
prompt of a session always carries both, which is why the launch prompt
leaves the session context out for a harness with `prompt_context`.
Between them rides the thread's undelivered nudges, prompt advisories
included. `injectSessionContext: false` in `.oxplow/project.yaml` turns
the session-context block off.

**Compaction.** A compaction or a fresh start forgets what was sent
(`AgentContext::reset_session`): on `PreCompact` (Claude Code's; its HTTP
`SessionStart` never arrives) and on `SessionStart` (Codex's), so the next
prompt carries the session context and the decisions again. The baselines
are in memory: a daemon restart costs at most one repeated block.

**The writer's agent.** A read-only thread's block carries a
`**Writer thread:**` line naming the stream's writer thread and its live
agent (harness key, pid, the harness's session id), so the agent can find
it in `ListAgents` (`live_writer_agent`, `build_session_context_block_full`).

**A role change.** The block appends an `**Access changed:**` note when
the thread's role differs from the one it had when the session was first
seen (`initial_role_by_session_id`, captured on its first prompt):
promoted to writer, or made read-only (wiki pages still go through
`oxplow.knowledge.write_page`). A plan's PostToolUse (`kind: plan`)
carries the note alone, for a promotion while the plan waited for
approval, since no prompt comes between; nudges wait for the next call.

### Per-harness delivery

| | Claude Code | Codex | opencode | ACP agent |
|---|---|---|---|---|
| Launch prompt | `--append-system-prompt` | `--config developer_instructions="…"` (a TOML string, `shared::toml_string`), which Codex puts at the head of its developer message | A per-session instructions file (`.oxplow/runtime/opencode-plugin/prompts/<ses>.md`) named by `OPENCODE_CONFIG_CONTENT`'s `instructions` | `_meta.systemPrompt.append` for an adapter with `systemPrompt: meta` (the Claude adapter), else a block ahead of a new session's first prompt |
| Skills | Plugin dir `.oxplow/runtime/claude-plugin/skills/` (`--plugin-dir`) | The worktree's `.agents/skills/<name>/` | `.oxplow/runtime/opencode-plugin/skills/`, named by the config's `skills.paths` | The index in the prompt + `oxplow_get_skill` |
| Slash commands | Plugin `commands/`, run as `/oxplow:<name>` | A skill per command, `oxplow-<name>`, run as `$oxplow-<name>` | Inline config `command` key, run as `/oxplow-<name>` | Not delivered |
| Hooks | HTTP hooks (`hooks.json`) | Command hooks (`<oxplow> hook <event>`) via `--config` | A JS bridge plugin posting Claude-shaped bodies | In-process (`ServicesAcpHost`), no hook route |
| Context channels | Both, as `additionalContext` | Both, as `additionalContext` | Prompt: a `synthetic` text part on the user's message; tool: appended to the tool's result | Prompt only: a visible leading block on the person's next prompt |
| Token capture | OTEL metrics + the transcript's turns | OTEL (its `response.completed` log event) | None | The prompt response's `usage` |
| Final message | Stop's `last_assistant_message` | Its Stop's `last_assistant_message` | The bridge reads the session's last assistant message onto `Stop` | The transcript's last agent message on the Stop body |

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
  launches every terminal harness with a marked system prompt and checks
  that the prompt, every core skill and every core slash command reach
  its agent: its command line, its environment, or a file it was given
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
and `run_command`. Every other tool is a foundation command or lens that
declares `tool:` and is offered as `oxplow_<name>`; any extension declares
its own the same way, under its namespace ([extensions.md](./extensions.md)
"Agents: the MCP surface"). With the harness's namespace on top the agent
calls `mcp__oxplow__run_command`, `mcp__oxplow__oxplow_query_sql`. No other
spelling is accepted.

Every tool foundation declares is named below
(`every_foundation_tool_is_in_the_agent_model_doc` pins it).

| Purpose | Tools |
|---|---|
| Read oxplow's data | `oxplow_query_sql` (one read-only statement over the `v_*` models, live models included — [semantic-layer.md](./semantic-layer.md) "Live models"), `oxplow_read { ref }`, `oxplow_search` (BM25 over tasks, comments, notes, wiki and file contents) |
| Code and revisions | `oxplow_vcs_log` (a stream's history from any revision, paged, past the index), `oxplow_vcs_status`, `oxplow_vcs_branches`, `oxplow_diff` (versions `working`, `snap:<id>`, `git:<rev>`; `since_fork`), `oxplow_vcs_blame` (through the `Vcs` provider; `working`, `snap:<id>` or `git:<rev>`), `oxplow_code_definition`, `oxplow_code_references`, `oxplow_code_hover`, `oxplow_code_call_hierarchy`, `oxplow_code_symbols` (a file's symbols from its language server, else oxplow's tree-sitter reading of the live file, saying which), `oxplow_code_workspace_symbols` (the editor's language servers, [lsp.md](./lsp.md)), `oxplow_ensure_change` (a change's analysis, then `v_change_*` through `oxplow_query_sql`) |
| Write | `run_command`, after `list_commands` |
| Ask a model | `oxplow_ai_decide` (typed questions about some text, on the `decide` role), `oxplow_ai_summarize` (on `summarize`): recorded computations paid with the person's key, as the asking thread ([ai-providers.md](./ai-providers.md)) |
| Show the person | `oxplow_show_lens`, `oxplow_run_lens`, `oxplow_run_lens_action`, `oxplow_navigate` (takes them to any ref's page, switching stream when needed) |
| Common questions, typed | foundation lenses with `tool:`: `oxplow_thread_work`, `oxplow_work_next`, `oxplow_effort_files`, `oxplow_effort_summary`, `oxplow_file_history`, `oxplow_recent_commits`, `oxplow_snapshots`, `oxplow_thread_notes` — each returns what its lens shows, for a model that shouldn't have to write SQL for them |
| What the person has | `oxplow_get_open_page` (the page open in a thread, a lens re-run with their params), `oxplow_list_comments` (their anchored comments, hydrated) |
| Wiki | `oxplow_wiki_ref_drift` (one stale ref's diff against its pin) |
| Extension authoring | `oxplow_validate_extension`, `oxplow_preview_extension`, `oxplow_preview_collector`, `oxplow_review_extension` |
| oxplow's guidance | `oxplow_get_skill { name, reference? }` |

A tool's `read_only_hint` (so a client can auto-approve) is its
command's access: `View` and `Read` commands are hinted, `oxplow_show_lens`
(it records the answer) and `oxplow_run_lens_action` are not, and nor are
`oxplow_ai_decide` and `oxplow_ai_summarize`: they change no record but spend
the person's provider key (a scope with `spends`), so the client asks before
they run. Each tool
but `list_commands` and `run_command` is a foundation command over a read
scope (`oxplow.records.read`, `oxplow.sql.query`, …; [commands.md](./commands.md)
"Scopes"), so an agent reaches it through `run_command` too; `v_scope`
lists the scopes. An anonymous connection may call the read-only ones.
Almost every read is a published model: an agent's thread, efforts,
work items, commits, snapshots, dashboards, diagnostics and errors are
`v_*` rows. A tool stays a tool only where a model can't be one: file and
revision content, live language-server answers, site search, a lens's
rendering, a dry run.

**`oxplow_read { ref }`** (`oxplow_app::ref_read`) reads anything a ref names and
returns `{ ref, kind, row, related, content, truncated }`: the owning
model's row, `related` lists (a work item's links and comments, a
commit's or an effort's files), and `content` for the kinds whose body no
model holds — a file at a revision (`file:src/a.rs` is the working tree;
`file:src/a.rs@git:HEAD#L10-40`; `@snap:<id>` a local-history snapshot),
an event's stored body (`event:<id>#input`, `#output`, `#prompt`; the
caller's own stream's events only), a recorded model call's bodies
(`ai_call:<id>#request`, `#response`; any call but one another agent's
thread asked for — an extension's, a collector's or a job's are the
project's), a lens's definition (`lens:<ext>/<slug>`). A file or lens is
read in the caller's stream's worktree, or another's named by `stream`
(`stream:str2`). Content stops at 64 KB with `truncated: true`; ask for
lines to read further. A captured file's row comes from
`v_snapshot_file`; restoring it is the destructive
`oxplow.snapshot.restore_file` (an agent's is a proposal), which writes
into that row's stream's worktree.

**Commands.** Every write is a command through `run_command { id, input
}`. `list_commands` with no arguments is an index of `{ id, summary }`;
with `id` or `prefix` (`oxplow.work_item.`) it gives each command's
`input_schema`, an `example` input, whether a person `confirm`s it and
whether it's `undoable` ([commands.md](./commands.md) "Exposure to
agents"). A command that needs a person's confirmation isn't run: it's
kept as a proposal and `run_command` answers `{ proposal, message }`
(the person decides in the thread and in Alerts; `v_command_proposal`;
[commands.md](./commands.md) "Proposals"). Program- and model-choosing
config keys (`agents`, `lsp`, `acpAgents`, `agentConfig`, `extensions`,
`agentPromptAppend`, …) set with `oxplow.config.set` are proposals; keys
like `zones` or `snapshotTurnBudgetMs` an agent sets itself. An anonymous
connection may read and list but not run. Which commands an agent may run
is each spec's (`check_command`, "Agent policy").

**Lenses.** When the person asks to see, list, compare or track
something, the agent answers with `oxplow_show_lens` (the `oxplow.lens.show`
command): an existing lens with params, or its own `spec` (table, chart,
tree). It renders beside the conversation, live, with "Keep this", and
the tool returns its ref and its text rendering. `oxplow_run_lens` reads what a
lens shows (text by default, `format: "json"` for rows); `oxplow_run_lens_action`
runs a lens's declared action as the lens acting for the caller, under
the same rules as running the command directly.

**Server instructions** (`get_info`) are the one text every harness
shows its agent: what oxplow knows (`oxplow_query_sql` over `v_*`), that every
write is a command, and to answer "show me" with `oxplow_show_lens` rather than
printing a table. They ship to users' projects, so they never name this
repo's docs.

**Tools read the interface.** No tool belongs to one implementation:
work-item reads (`oxplow_query_sql` over `v_work_item`, `oxplow_read` of a `work_item:`
ref, the work-next lens) go through whichever list is active (empty with
none), so the tool list never changes with a switch. What an
implementation owns is commands, offered only while it's active
(`capabilities::Active::refusal`).

**Comments.** `oxplow_list_comments({ id, scope?, status? })`: `id` is a thread
(`thr…`) or a stream (`str…`); `scope` is inferred from the prefix
(`resolve_comment_scope`; pass it only to assert the kind); `status` is
`all`, `open` or `needs_response`. Each row is an `EnrichedCommentThread
{ thread, primary, context_chain, referenced }` — the anchor target, the
page regions it sat inside (innermost first) and the refs inside the
selection, each resolved by `oxplow_app::ref_resolver` — so the agent
gets what the highlighted thing is in one call. Replies and resolution
are `oxplow.knowledge.reply_comment` / `update_comment` (`comment:cmt12`);
a reply is authored by the actor and clears `needs_response`. oxplow
never prompts for comments: the person runs the `review-comments`
command.

**Code tools.** With no server for a language the error names the Mason
package and both fixes: `oxplow.lsp.install_server { package }` (a
proposal: which binaries oxplow downloads and runs is the person's call)
or an `lsp.servers` entry in `.oxplow/project.yaml`. `v_lsp_server` says
what's configured, installed and running.

**The wiki and links.** Wiki reads are `oxplow_query_sql` over
`v_knowledge_page` / `v_knowledge_ref` (stale pins) and `v_page_ref` (the
backlinks graph, both directions), plus `oxplow_search` with `kinds: ["wiki"]`;
`oxplow_wiki_ref_drift({ slug, path })` returns one stale ref's unified diff
(`status` drifted, unchanged, not_a_ref, no_pin, binary or no_contents).
Writes are the `knowledge.*` commands ([knowledge.md](./knowledge.md)).
`oxplow.effort.report`, `oxplow.knowledge.add_note` / `update_note` and
`oxplow.work_item.create` / `update` check the `[[…]]` links in the text
they keep (`oxplow_app::link_check`) and return `link_warnings`, so the
agent fixes them in the same turn; `knowledge.write_page` refuses
instead. What counts as a link is `oxplow_domain::refs::classify_wikilinks`
([refs.md](./refs.md)).

**Id checks at the boundary.** A tool taking a thread or stream id calls
`expect_id_kind(tool, param, value, expected)` (`IdPrefix`, `ID_STREAM`,
`ID_THREAD`), so a stream id where a thread id belongs is an
`invalid_params` naming the tool, the parameter, the value and what was
expected, not a downstream foreign-key failure. Add the call to any new
tool taking an id.

### Param casing is lenient (camelCase aliases tolerated)

Param structs are snake_case, and that stays the advertised schema. But
tool outputs are camelCase and weak models (opencode's default) send
`threadId` and hit an opaque `-32602 missing field`.
`lenient_params::Parameters` (`crates/oxplow-mcp/src/lenient_params.rs`)
is a drop-in for rmcp's `Parameters<T>` on core's two tools (same name,
so `#[tool]` still derives the schema from `T`): before deserializing it
adds a snake_case copy of any camelCase or kebab key, recursively, never
removing the original, and a genuinely missing field is a clear
`McpError` (`lenient_from_object`, `to_snake_case`). A declared tool's
input reaches a command whose schema refuses unknown keys, so there a
top-level camelCase key is *renamed* to the schema property it stands for
(`fold_to_schema`). Inferable fields are optional and inferred in the
handler where safe (`oxplow_list_comments`' `scope`).

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
never `command`) or `LaunchSpec::Acp { program, args, env,
system_prompt_via_meta }`. The built-ins live in `crates/oxplow-harnesses/`
(`claude.rs`, `codex.rs`, `opencode.rs`, `acp.rs`; shell quoting and
shared runtime writers in `shared.rs`) and depend only on the domain;
core's text lives in `oxplow-agent-text`. The one caller is
`launch_session` in `crates/oxplow-rpc/src/commands/terminal.rs`, used by
`open_terminal_session` (PTY) and `acp_open_session`. It hands the
harness the session/thread/stream ids, the worktree and project dir, the
control-plane endpoints with the session's bearer, the identity env, the
assembled system prompt, the session's resume id, the agent text, the
harness's config, oxplow's own executable, `HOME` (local streams only)
and the program search path. A harness never touches the database.

**What a harness implements.** `AgentHarness` requires what every harness
answers: `id`, `title`, `interact`, `launch`, `tool_use` (its calls in
oxplow's vocabulary, "Tool vocabulary") and `render` (a `HookAnswer` in
its wire shape). Everything else defaults to none: `env_markers`,
`settings`, `refresh_text`, `turns`, `token_readings`, `prompt`,
`subagent`. The operations are async and their data serde shapes
(`LaunchSpec` and `HookAnswer` tagged by `kind`), because a harness may be
a provider's process ([providers.md](./providers.md), the `agent_harness`
contract): what a built-in answers in code, a provider declares or answers
by a verb of the same name. A harness's declaration names the optional
verbs it has as features — `transcript` (`turns`), `telemetry`
(`token_readings`), `runtime_text` (`refresh_text`), `subagents`
(`prompt`, `subagent`) — and the two context channels, `prompt_context`
and `tool_context`:

| Harness (key) | Features |
|---|---|
| Claude Code (`claude`) | terminal, permission_prompts, resume, transcript, telemetry, runtime_text, subagents, prompt_context, tool_context |
| Codex (`codex`) | terminal, resume, telemetry, runtime_text, subagents, prompt_context, tool_context |
| opencode (`opencode`) | terminal, runtime_text, subagents, prompt_context, tool_context |
| ACP agent (`acp`) | structured_transcript, permission_prompts, resume, prompt_context |

(`crates/oxplow-app/src/capabilities.rs`, the `harness(…)` built-ins.)
`crates/oxplow-app/src/harness_conformance.rs` checks the floor every
harness keeps, built-in or provider: answers render as objects and a deny
or context reads differently from an ack; the bearer stays off the
command line; a body naming no tool maps to none; a plain prompt is a
person's.

**A harness is a registry key, never an enum.** A session's `harness` is
the key of a registered harness (`HarnessRegistry`: the `agent_harness`
declarations in declaration order, then provider instances); nothing in
core names one (`source_guards::only_harness_implementations_name_a_harness`).
Each activation publishes the declared ones (`harnesses::prepare_built_ins`
→ `set_declared`) and keeps a registered instance. `agents: [...]` in
`.oxplow/project.yaml` lists the enabled ones in priority order (the
first is a new session's default); left out, every registered harness is
enabled and the first declared is the default. `set_agents` and
`oxplow.agent_session.open` check keys against the registry. A harness's
own settings are `agentConfig: { <key>: { … } }`, what its `launch`
reads; it declares the ones a person sets (`AgentHarness::settings`:
opencode's `model`), and Settings → Agents renders a field for each,
saved by `set_agent_setting`, which refuses an undeclared one. The UI
reads the harnesses through `list_agent_harnesses` (`useAgentHarnesses`,
`agentKinds.ts`). Whether a session is a chat follows from `interact()`,
not its name. Each session keeps its own harness, so Claude and Codex
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
  --append-system-prompt <text> --mcp-config <file> --strict-mcp-config
  [--resume <id>]`, falling back to `~/.claude/local/claude` when no
  `claude` resolves. Its HTTP hooks (`HOOK_EVENTS`) are `PreToolUse`,
  `PostToolUse` (matcher `"*"`), `UserPromptSubmit`, `SessionStart`,
  `SessionEnd`, `Stop`, `Notification`, `SubagentStart`, `SubagentStop`,
  `PreCompact`, plus `TaskCreated` / `TaskCompleted`, acked unread. Each
  carries `Authorization: Bearer $OXPLOW_HOOK_TOKEN` (`allowedEnvVars`)
  with an 8 s client timeout, above the server's 5 s budget: a lower one
  made Claude Code discard answers the server was about to give. Its OTEL
  env points the metrics exporter at `/v1/metrics` with the bearer in
  `OTEL_EXPORTER_OTLP_HEADERS` (10 s interval).
- **Codex** keeps nothing under `.oxplow/runtime/`: its MCP server
  (`bearer_token_env_var = OXPLOW_HOOK_TOKEN`), command hooks and OTEL
  exporter ride `--config` overrides, the bearer its env. It runs `codex
  [--no-daemon] --cd <worktree> …` or `codex [--no-daemon] resume --cd
  <worktree> … <id>`; `--no-daemon` (embedded mode, keeping the overrides
  and identity in this process) is passed when the resolved CLI's
  `--help` lists it, probed once per binary. Its hooks run `<oxplow> hook
  <event>` (`oxplow_control_plane::hook_client`, which POSTs the hook's
  stdin), where `<oxplow>` is the running binary — the daemon when the
  desktop app runs one (`oxplow-daemon/tests/hook_command.rs`). Hook
  events: `PreToolUse`, `PermissionRequest` (acked unread),
  `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `Stop`; timeout 30 s.
  Its skills (and commands, as skills) go to the worktree's
  `.agents/skills/` (`shared::write_worktree_skills`).
- **opencode** writes `.oxplow/runtime/opencode-plugin/`
  (`plugin/oxplow-hooks.js`, `skills/`, `prompts/`) and runs `opencode -m
  <model> [-s <id>]`, the model from `agentConfig.opencode.model`, else
  `DEFAULT_MODEL` (`github-copilot/gpt-5-mini`). Hooks, MCP (bearer by
  opencode's own `{env:OXPLOW_HOOK_TOKEN}`), the instructions file, skills
  and commands ride `OPENCODE_CONFIG_CONTENT`, inline config opencode
  merges last. The bridge (`crates/oxplow-harnesses/assets/opencode-hooks.js`)
  translates: `chat.message` → `UserPromptSubmit`; `tool.execute.before`
  → `PreToolUse` (a deny throws, which blocks the call, so the write guard
  works; lowercase tool names and `filePath` become Claude's
  `Edit`/`Write`/… and `file_path`); `tool.execute.after` → `PostToolUse`
  (`callID` as `tool_use_id` so a call's two hooks pair, its bash tool's
  `metadata.exit` as `exit_code`); `session.idle` → `Stop`. A child session (`parentID` set) posts no
  `UserPromptSubmit`/`Stop`; its tool hooks are posted as the parent's
  with `agent_id: <child>` and `agent_type: "task"`, read by
  `shared::subagent_of`, so the parent's resume id is never the child's.
  No `SessionStart`, `SessionEnd` or `Notification`. The bridge runs under
  bun in `its_bridge_sends_the_call_id_the_exit_code_and_a_childs_parent`
  and `its_bridge_delivers_hook_context_and_sends_the_final_message`.
- **ACP** launches the agent's program as its config resolves it (an
  adapter's or a project's `acpAgents` entry, "ACP agents:
  configuration"); the session writes nothing to disk.

**Identity.** Every process oxplow starts for a session gets
`OXPLOW_HOOK_TOKEN` (its session's bearer, which alone says who it is —
"Caller identity"), `OXPLOW_HOOK_BASE_URL`, and `OXPLOW_STREAM_ID`,
`OXPLOW_THREAD_ID`, `OXPLOW_SESSION` (`ses<n>`): the agent's own knowledge
of where it runs, which nothing trusts (`identity_env` in `terminal.rs`).

**What an agent inherits.** Every agent and terminal (PTY and ACP) is
spawned without `agent_path::not_inherited(&harnesses)`: an oxplow
agent's identity (`agent_path::NOT_INHERITED`: the `OXPLOW_*` above and
`OXPLOW_HOOK_DEBUG`) and every registered harness's `env_markers()`
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
agent session (the pane target is its id, `ses3`), so a re-attach shows
the live process and two sessions in a thread are two processes; a
`shell` / `shell:<id>` target is a plain `$SHELL -l` in the worktree.
Switching streams or threads doesn't kill sessions: the daemon keeps them
and a re-attach replays the buffer. They end with the daemon; the next
open resumes. There is no terminal multiplexer.

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
spawns with (`TERM`, `COLORTERM`, the widened `PATH`); every spawn site
calls it. One that hand-rolls its env reopens this bug for the tools the
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
stream on another host (no `HOME` passed), reads as unknown. An ACP
session `session/load`s its resume id when the agent supports it.

### ACP agents: configuration

A chat session (its harness's transcript is structured: `acp`) names an
ACP agent in `agent_session.acp_agent`; `open_terminal_session` refuses
it.

**Configuration.** Agents are `acp_adapter` declarations in
`oxplow-foundation` (the `oxplow:acp-adapter` built-in, config `{
command, args?, env?, systemPrompt?: meta|prompt }`, read into an
`AcpAdapter` by `oxplow_harnesses::acp_adapter`, kept in declaration order
in `Services.acp_adapters`): `claude` (`claude-agent-acp`, the prompt in
`_meta`), `gemini` (`gemini --acp`), `codex` (`codex-acp`). The
project's `acpAgents: [{name, command, args, env}]` layer over them
(`acp::agents::resolve`): an entry with a declared id overrides its
program and keeps the adapter's prompt mode; a new name is a generic
adapter (the prompt ahead of the first message). An `acp` session that
names no agent runs the project's first entry, else the first declared
adapter (`acp::agents::default_agent`).

**Listing and approval.** `oxplow_app::acp::agents::list` (IPC
`list_acp_agents`, UI-only) reports each agent's source, whether it may
start, and its resolved path. A declared adapter may always start; a
project entry needs a person's approval in Settings → Data → Programs
(`exec_consent`, `ProgramKind::AcpAgent`). A personal (user-global)
`acpAgents` file isn't built.

**Opening sessions.** `oxplow.agent_session.open` takes `acp_agent`:
required for a chat harness (defaulted as above), refused otherwise, and
checked against the list. It declares `ui: { label: "New Agent Session…",
form: "page:new-session" }`, so a person choosing it gets the picker
("<harness title> · <name>" per ACP agent, flagged "not installed" or
"needs approval", `agentChoices` in `agentKinds.ts`). `oxplow-foundation`
also declares a direct command per agent — "New Claude Session", "New
Codex Session", "New OpenCode Session", "New … Session (ACP)" per declared
adapter (`agent_session.open_<agent>`, `ui.input` fixing `harness` /
`acp_agent`, `open_after` the new tab), each offered while its harness is
enabled (`when: enabledAgents =~ …`). Those are a person's; an agent uses
`agent_session.open`.

### ACP agents: protocol mapping and transcript

**The SDK is pinned (`agent-client-protocol = "=2.2.0"`) and fenced.**
`acp/wire.rs` is the only module that names its schema types; it converts
them to `acp/model.rs` types (`ToolCall`, `ToolCallPatch`, `AcpUpdate`,
`PermissionAsk` / `PermissionAnswer`), so an SDK bump touches one file.
Content text is capped at 64 KiB per block (`MAX_TEXT`).

**Gotcha: the SDK turns on `serde_json/preserve_order` workspace-wide**
(features unify). JSON objects serialize in insertion order. Anything that
needs a stable text form builds it from a `BTreeMap` (`metric_cube::dims_key`
does), and approval hashes never hash JSON.

**`acp/mapping.rs` (pure)** maps the protocol's tool kinds onto oxplow's
vocabulary: edit, delete and move → `edit`; read → `read`; search →
`search`; execute → `shell` (`rawInput.command` / `cmd`, else the title);
fetch → `fetch`; an `mcp__…` title or name → `mcp`; other → `other`;
think and switch-mode record nothing. Paths come from `locations`, then
the diffs, then path-like `rawInput` keys (`file_path`, `path`,
`absolute_path`, `notebook_path`, `source`/`destination`,
`old_path`/`new_path`), deduplicated. The stored `content` is `rawInput`,
and once the call finishes `{is_error, …rawOutput}`, so a shell call's
`exit_code` survives.

**`acp/transcript.rs`** is the in-memory conversation (no table). Items:
user, agent, thought, tool (merged with its updates by call id), plan
(replaced within a turn), permission, policy-denied, bypass, error. Each
has a stable `id` and a `seq` bumped from one counter whenever it
changes; `since(seq)` returns new and changed items, which clients upsert
by `id`. Chunks of one kind coalesce into the trailing item;
`usage_update` is state (the context meter), not an item. The ring holds
2000 items (`DEFAULT_CAP`).

### ACP agents: sessions

**Shape.** `Services.acp` (`acp/manager.rs`) holds each agent session's
command sender and a shared `SessionView` (status, transcript, stderr
tail), keyed by `AgentSessionId`. One actor task per session
(`acp/session.rs`) owns the connection; `wire::run` feeds every agent
message into one channel and the prompt's result is an ordered barrier,
so the actor sees updates, requests and turn end in wire order. Events
for every session share one broadcast channel (`AcpEvent`). `open`
reserves the session's slot under one lock before spawning, so
concurrent opens start one agent; `close` marks the handle closed at
once, and a closed session still winding down is replaced and marked not
current, so it records no Interrupt over its successor. The agent runs
via `tokio::process` with `kill_on_drop` and the augmented `PATH`; its
stderr's last lines explain a failed start. Closing a thread
(`oxplow.thread.close`) closes its open effort and, once committed, stops
every open session's process — ACP or PTY — through
`agent_sessions::SessionProcesses::kill`. A fork (`oxplow.thread.create {
from }`) keeps the source session's harness and `acp_agent`.

**Host.** `acp/host.rs` `AcpHost` is the seam (tests use a recording
double). `ServicesAcpHost::new(svc, stream, session)` holds `Weak<Services>`
and the session it hosts, and records what a hooked turn records:
`SessionStart` plus the resume id on start; `UserPromptSubmit` on the
person's prompt; `PreToolUse` on every policy check; `PostToolUse` on each
finished call; `Stop` with the turn's counts (`TURN_USAGE_KEY`) and its
last agent message (`LAST_ASSISTANT_MESSAGE`); `Interrupt` when the agent
goes away.

**Starting.** `initialize` offers fs read/write and no terminal. oxplow's
MCP rides `session/new|load` as an HTTP MCP entry with the session's
bearer; an agent without HTTP MCP support is refused with a clear error.
The resume id is `session/load`ed when supported; the replay rebuilds the
transcript and records nothing (no hooks, no tool rows, no bypass checks,
fs writes refused). The system prompt is `assemble_system_prompt` for the
`acp` harness: the session context is left out (the first prompt carries
it) and the skill index is in (no skill files).

**Prompts (the no-automation rule).** `acp/human_prompt.rs` `compose` is
the only way to make a `HumanPrompt`, and `wire::AgentConn::prompt` takes
nothing else. The session calls it only for `Command::Prompt`, which only
`AcpManager::submit_human_prompt` sends; a second prompt while a turn runs
is `TurnInFlight`, never queued. The session context, advisories,
decisions and waiting nudges ride the person's prompt as a visible
leading block (behind a disclosure on the user item). `acp/guard_tests.rs`
scans the source: `PromptRequest` only in `wire.rs`, no raw
`"session/prompt"` outside the fake, `compose(` only from the session,
`submit_human_prompt(` with no other callers.

**Permissions.** Each `session/request_permission` asks `AgentPolicy`. A
deny is answered `reject_once` (or `cancelled` when no reject option
exists) with a policy-denied item; the model sees only the rejection. An
allow shows a permission card that waits for the person with no timeout;
"Always allow" is dropped for writes; the thread is `AwaitingUser`.
Cancel answers every open card `cancelled`. Cards' "awaiting" is cleared
before the Stop is ingested, so the Stop decides the status from the
final message; after the cards, a question the thread already waited on
survives, else the status is Running inside a turn and Idle outside one.
**Teardown is one path:** `session::Teardown` cancels unanswered cards,
adds the error item, records the Interrupt (only while current) and emits
Stopped/Closed; the actor runs it on a clean exit and the connection task
again afterwards (a no-op once stopped), so a transport error still ends
the session.

**fs and bypass.** `fs/write_text_file` is policy-checked; a deny is a
JSON-RPC error whose message (the reason) does reach the model. oxplow
reads and writes files for an agent only inside the session's worktree
(paths normalized first; relative ones against the cwd). A write-kind
call that completes after the policy or a person rejected it gets a
bypass banner; one that completes with no one allowing it, and not every
path written through `fs/write_text_file`, is checked afterward and
bannered if the policy would deny it — the only backstop for adapter
modes that skip asking. At turn end, open cards are cancelled and the
Stop and tokens recorded.

**RPC and events.** `oxplow-rpc/src/commands/acp.rs`; every command is a
`ui(...)` parity row, so none can become an MCP tool. `acp_open_session`
(a hand-written Tauri adapter: it needs `plugin_runtime` for the MCP URL
and bearer) checks the session is open and `acp`, resolves the agent
(`find`, `may_start`, `resolve_command`), assembles the prompt, passes the
resume id, opens the session and returns the `AcpSnapshot`. The rest:
`acp_prompt` (the prompt box's Enter, the only caller of
`submit_human_prompt`), `acp_cancel`, `acp_respond_permission` (no option
means cancel), `acp_transcript(sinceSeq)` (`null` with no session),
`acp_close_session` (stops the process). Every one takes `sessionId`.
Events ride the `acp:event` channel (frame key `acp`): `AcpEvent {
agentSessionId, threadId, generation, type: item|status|usage|closed, …
}` over the daemon's `/events`. Raw agent JSON is TS `unknown`
(`specta_typescript::Unknown`).

**UI.** `AgentPage` renders `components/acp/AcpAgentView.tsx` (by
`sessionId`) for an `acp` session. `acpTranscript.ts` is a pure reducer
over the snapshot and live events: items upsert by id, the newer seq
wins; a seq gap or a remote reconnect refetches `since(headSeq)` (past a
gap `headSeq` stays at the last contiguous seq). Each open is a new
**generation** whose ids and seqs restart, and the reducer starts over on
one, so a restart never mixes transcripts. The view subscribes before
fetching. `AcpPromptBox.tsx` is the only renderer caller of `acpPrompt`:
Enter sends, Shift+Enter adds a newline, Escape or Stop cancels, nothing
sends while a turn runs, and "Add to agent context" appends to the draft
(`AcpAgentView.test.tsx`).

**Limits.** A permission reject carries no reason; only an
`fs/write_text_file` denial says why, and the Claude adapter writes to
disk itself, so its only gate is the permission request. Bypass
detection is after the fact. "UI-only" is enforced twice: the `ui(...)`
rows, and the daemon's `/ipc` requiring the per-launch UI token only the
renderer holds ([remote-daemon.md](./remote-daemon.md) "Auth"), so no
agent can prompt an agent through the daemon. Per-turn tokens come only
from the prompt response's `usage` (an unstable field, SDK feature
`unstable_end_turn_token_usage`); `usage_update` drives only the meter;
ACP agents get no OTEL env, so nothing double-counts.

**Tests.** `crates/oxplow-acp-fake` is a scripted fake speaking raw
JSON-RPC (deliberately not the SDK, so tests exercise real wire JSON);
`fake:<step>` lines in a prompt drive it (say, think, edit, bypass,
fswrite, fsread, bash, plan, usage, tokens, wait, crash).
`acp/session_tests.rs` runs it in-process over a duplex pipe;
`tests/acp_services.rs` against real `Services`, and once as its binary;
the browser suite drives it end to end
([working-in-this-repo.md](./working-in-this-repo.md) "The browser
suite"). `tests/acp_live.rs` is `#[ignore]`d: `OXPLOW_ACP_LIVE_CMD="<adapter
command>" cargo test -p oxplow-app --test acp_live -- --ignored`.

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
  `patchText`). Codex's and opencode's `apply_patch` → `edit` of every
  file the patch names (`shared::patch_paths`); Codex's `Bash` (older:
  `shell` / `exec_command` / `local_shell`) → `shell`, joining an argv;
  `…spawn_agent` → `subagent`; its result's `Exit code: N` first line is
  the exit code. An ACP agent's calls are mapped by `acp::mapping`.
- **Where.** The hook route maps a PreToolUse / PostToolUse body with the
  bearer's harness before the policy runs and hands the call to the
  ingest (`HookEnvelope.tool`); the ACP host does the same with its
  mapping. The ingest maps nothing: it runs in one transaction, and a
  harness's `tool_use` may be a provider's call. A body naming no tool
  records nothing.
- **What reads it.** The write guard (`edit`, every path), the status
  derivation (`subagent`, `ask`, `plan`), the role-change note (`plan`),
  effort file claims (`edit`), test-run collection (`shell`, its `command`
  and `exit_code`), a turn's writing calls (`thread.checkpoint`, by
  `v_tool_call.kind`), the bundled struggle model, `v_context_read`, and
  `agent.tool.requested@3` / `agent.tool.finished@3`, which carry `tool`,
  `kind` and `paths`.

### Caller identity

Every agent session gets its own bearer when it launches
(`terminal::session_endpoints` → `SessionAuth::issue`,
`crates/oxplow-app/src/session_auth.rs`), bound in memory to its session,
thread, stream and harness (a `Principal`). The control plane's routes —
`/hook/{event}`, `/v1/metrics`, `/v1/logs`, `/mcp`, `/dev/ping` (a
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
  (`OTEL_EXPORTER_OTLP_HEADERS`, Claude's and Codex's alike), opencode's
  `{env:…}` header, and an ACP agent's `McpHttp` entry. A bearer is never
  on a command line, which any process can list.
- **What it protects.** One agent acting as another by naming it. It is
  not a wall between processes of the same OS user, who can read another
  agent's env or owner-only files; agents run as the person, so that
  boundary is the OS's.
- **In process.** The ACP host and the UI's interrupt build envelopes
  directly, naming the session.
- **The actor carries its session.** An MCP call's actor is
  `Actor::Agent { session_id, thread_id, stream_id }`; its audit row
  records the session (`command_audit.session_id`) and its events are
  anchored to it (`Actor::anchors()`). `source` stays `agent:thr<n>`.
- A hook's harness is the bearer's, so even a timed-out hook is answered
  in its harness's shape.

### Hook ingest and recording

Each hook POSTs to `/hook/<event>` with the session's bearer. The route
(`handle_hook_inner`) dumps it when `OXPLOW_HOOK_DEBUG=<file>` is set (one
JSON line per hook: `event`, `thread`, `at`, `payload` — how to learn a
harness's real payload shapes before depending on them), maps the prompt
(`AgentHarness::prompt`: a person's or a subagent's hand-back) and tool
call with the bearer's harness, runs the policy on a `PreToolUse`, hands
one `HookEnvelope` to `HookIngestService::ingest`
(`crates/oxplow-app/src/hook_ingest.rs`), and answers: context after a
`PostToolUse` or a person's `UserPromptSubmit`, a deny, or an ack. An
event it doesn't know is acked unread; `PreCompact` only resets the
session's context baselines.

**Which agent session a hook came from** (`agent_session_store::resolve_tx`,
shared with OTLP exports), in order: the session the sender is (the
bearer's, the ACP host's, the UI's interrupt's; refused with a warning
when it's another thread's); the session whose resume id is the hook's
harness session id (an open one first); the thread's newest open session,
one with a turn running first (logged as a warning — every process oxplow
starts names its session); none (an agent oxplow didn't start: recorded
with the thread's anchors only).

**One transaction per envelope** writes the state the hook changes and
the `agent.*` events that record it, anchored to the thread's stream, the
session, its open turn and the thread's single open effort
(`activity_anchors_tx`). **Attribution is a best effort, by design:**
several agents, the person and outside processes can change a worktree at
once, so no rule ties every change to the right work exactly. The anchor
stays thread-scoped (the thread's own open effort), because that says
more than the stream would even when it's sometimes wrong.

- **Prompt** → `agent.prompt.submitted` (`reprompt: true` inside an open
  turn), its text in `event_content` (`oxplow_read` on `event:<id>#prompt`); with
  no turn open it opens one (`agent.turn.started`). A subagent's hand-back
  logs no prompt, opens a turn with none if needed and finishes the
  subagent.
- **PreToolUse** → `agent.tool.requested` with the policy's `decision`;
  **PostToolUse** → `agent.tool.finished`. Paths, detail, `ok` and
  `exit_code` are computed here against the thread's worktree; input and
  output go to `event_content` by hash. The harness's `tool_use_id` is
  the dedupe key, so a re-posted hook logs once.
- **Stop / Interrupt** close the session's open turns
  (`agent.turn.ended@2`, with Claude's `transcript_path`), then the
  turn-end snapshot ("Snapshot tracking").
- **Status.** A status the hook sets (Running, Idle / AwaitingUser,
  Stopped) is logged as `agent.status.changed` when it differs from the
  newest logged one, read in the same transaction (`last_status_tx`). The
  log is the status: a restarted daemon still knows a thread waits on the
  person. `HookIngestService::set_status` (ACP permission cards) is the
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
closes the turns the previous process left open as interrupted, logs
`agent.session.started` again (`resumed: true` for the resume id) and sets
Idle, so a turn that died without a Stop reads idle after a restart, not
stalled. The ACP client posts the same `SessionStart`. **`SessionEnd`**
closes the turns that session left open as interrupted ("session ended";
an exit mid-turn sends no Stop), status Stopped, and leaves other
sessions' turns alone; it logs `agent.session.ended`, and `reason: clear`
of the resume id blanks it (`/clear` starts a fresh session with no HTTP
hook). A close passes the body's `transcript_path` so an interrupted
turn's tokens are counted. A PTY's exit ingests a `SessionEnd` naming its
session (`TerminalSessionRegistry::ingest_exits_into`), once — Codex posts
none of its own.

**PreToolUse cost.** `agent_policy::may_refuse` is false for anything but
an edit, so `pre_tool_check` returns before any database read or git stat
for reads, searches, shell, MCP and subagents. The HTTP round trip still
fires for every tool (Claude's matcher is `"*"`).

**Side-band steps are best-effort by design.** Reactors that decorate a
call (collection observations, wiki attribution) try and warn: a coverage
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

**The one rule:** the guard keeps agents off worktree files, nothing
more. A non-writer thread may not change its own stream's worktree, and
no thread may change another stream's. Everything else is open to every
thread: files outside every stream, the shared wiki (through
`oxplow.knowledge.write_page`), and oxplow's own objects — work items,
efforts, notes, comments. A task's state and an effort are records, not
claims on the worktree, so a research or review thread can say what it's
working on. The bus's `WriteGate` applies to `Write` commands only;
`Record` commands skip it ([commands.md](./commands.md) "The pipeline").
**An agent may open a sibling agent session** in its own stream
(`oxplow.agent_session.open`): the row makes the UI attach and start its
process, and nothing types into it; its close is destructive and a
proposal.

- **Pure rules** in `crates/oxplow-runtime/src/policy.rs`:
  `decide_tool(ToolIntent{label, kind, paths}, PolicyFacts)` returns
  `Allow` or `Deny { layer, reason }`. Paths are normalized first
  (`normalize_path`: `..` resolved, the deepest existing ancestor
  canonicalized), so a `..` or a symlink can't spell a guarded path as an
  outside one. With several paths, the first refused wins.
- **I/O** in `crates/oxplow-app/src/agent_policy.rs` (`Services.agent_policy`):
  `check_tool(svc, thread, intent)` gathers the thread, its stream's
  worktree, the other streams' and the project dir; `check_command(thread,
  spec)` is the bus's agent gate — an agent runs a command only when its
  spec admits agents (`DenyLayer::Command`); `intent_of(tool)` makes an
  edit a worktree write of every file it names, and nothing else can be
  refused (`may_refuse`).
- **Transports only render.** The route's answer is a neutral
  `HookAnswer` (`Ack`, `Deny { reason }`, `Context { event, text }`)
  rendered by the hook's harness: the session's (`IngestOutcome.harness`),
  else the default harness's. Every built-in renders Claude Code's shape
  (`hookSpecificOutput`, `shared::render`), which Codex's command hooks
  and opencode's bridge speak too. An ACP agent gets a permission reject
  or an fs error.

### Write guard

Non-writer threads share the writer's worktree (same checkout, separate
agent sessions); their writing would corrupt the writer's work in
progress.

- **Edits** (`kind: edit` — Claude's `Write`/`Edit`/`MultiEdit`/`NotebookEdit`,
  Codex's and opencode's `apply_patch`, an ACP agent's edit, delete or
  move) from a non-writer thread into its own stream's worktree (the
  project's `.oxplow/` included) are denied; the reason is
  `write_guard::read_only_reason` (`crates/oxplow-runtime/src/write_guard.rs`)
  and names the path and `oxplow.thread.promote` (which a person or the
  agent may run; the current writer joins the queue). An absolute path
  outside the thread's worktree and every other stream's is allowed
  (`~/.claude/plans/foo.md`).
- **Another stream's worktree** (the primary checkout included) is denied
  for every thread, writer or not.
- **Wiki pages are written by command.** A direct write to the primary
  project's `.oxplow/wiki/` is denied for every thread
  (`write_guard::wiki_page_reason`); pages go through
  `knowledge.write_page`.
- **Shell isn't classified.** A non-writer's shell commands aren't
  checked; the session context's Access line says the thread is
  read-only.
- MCP tools are never refused here: their writes are commands, gated on
  the bus.

### Stop hook

A Stop is never refused ([work-tracking.md](./work-tracking.md) "No
gates"). The ingest closes the turn, keeps `last_assistant_message` as the
turn's `answer` (each harness's source in "Per-harness delivery";
`hook_ingest::LAST_ASSISTANT_MESSAGE`), takes the turn-end snapshot, sets
the status — `AwaitingUser` when the final message ends in a question,
keeping an `awaiting_user` the turn set — and acks `{}`. Without
`oxplow.effort.report`, an effort's summary is its last turn's answer.

### Subagents

**Who in a session acted.** A subagent (Claude Code's `Agent`/`Task`, a
Codex spawned agent, an opencode child session) is not a session: it has
no bearer and no process oxplow knows. It is who made a call inside its
parent's session: `ToolUse.subagent { id, kind? }`, on
`agent.tool.requested@3` / `agent.tool.finished@3` and
`v_tool_call.subagent_id`. The harness fills it when it can (Claude's and
Codex's `agent_id` / `agent_type`, `shared::subagent_of`); else the ingest
**brackets** it: a call landing while the session's open turn has a
`subagent` call requested and not finished is that call's subagent.
`SubagentStart` / `SubagentStop` log `agent.subagent.started@1` /
`.finished@1` (dedupe `subagent:<session>:<id>:<phase>`), read by
`AgentHarness::subagent`. A Claude Code **background** subagent outlives
its parent's turn — its calls arrive after the parent's Stop — and hands
its report back as a prompt (`<agent-message from="<id>">…`), which
`AgentHarness::prompt` reads as `Prompt::Handback`. Status keeps a
session working while a subagent it started runs. A provider harness
answers both as its `prompt` and `subagent` verbs (feature `subagents`).
Recorded in `fixtures/claude/subagent.jsonl` and replayed
(`a_recorded_claude_code_subagent_is_attributed`).

Subagent tool calls reach the hooks like any other, so their edits and
test runs land in the thread's effort with nothing from them: there is
no dispatch protocol or subagent skill. A subagent can leave a finding in
a thread note (`oxplow.knowledge.add_note`, read back from
`v_thread_note`).

**What to pick up next** is the foundation lens `oxplow-foundation/work-next`
(the tool `oxplow_work_next` with `thread: "thread:thr3"`) over `v_work_item`, whichever
list is active: an epic with its ready descendants, or every ready
non-epic item, open `blocks` links holding an item back; empty with
nothing ready or no list. The bundled `work-next` command calls that tool and
moves the item it picks to `in_progress`, so the effort links to it.
Claude Code's `TaskCreate` is a within-turn planner and never mirrors
oxplow's items.

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
`elicitation_dialog`; ACP permission cards log it through `set_status`.

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
  closes as interrupted and the status is Stopped, which clears open
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
  ACP host); both must go quiet. Since no hook arrives to re-derive,
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
per turn and per effort is known without git. The store and its
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
  runs a `turn_end` take anchored to the turn, its thread and its open
  effort, and `agent_turn.snapshot_id` records it. **What the turn
  changed is `agent_turn.start_snapshot_id → snapshot_id`**, the start
  recorded when the turn opened — not the take's op parent, which other
  takes during the turn (an effort's close, a commit, another thread's
  turn end) move. One take per Stop, for the newest turn it closed
  (`AgentTurnStore::close` reports whether it closed anything), after the
  status is set so the UI isn't held on running.
- **The budget.** The Stop waits at most `snapshotTurnBudgetMs` (default
  2000, min 100; an agent may set it). A slower take is never aborted: it
  finishes in the background and its op and `snapshot.taken` record
  `over_budget`. The clock starts before the per-stream take lock.
- **Turn events.** `agent.turn.started@1` / `agent.turn.ended@2 {
  outcome: completed | interrupted | restart }`, in the same transaction
  as the `agent_turn` row.
- **Quiet period.** Human edits between turns get their own snapshot: a
  watcher change arms a deadline (`DEFAULT_QUIET_PERIOD`, 3 s, debounced);
  when it fires with paths dirty and no turn open on any of the stream's
  threads (`stream_has_open_turn`, the registry's `OpenTurnProbe`), a
  `quiet` take records them. While a turn is open it yields to the turn's
  own take.
- **Effort brackets.** An effort's open takes an `effort_start` snapshot
  (`effort.start_snapshot_id`), its close an `effort_end` one
  (`end_snapshot_id`); an unchanged tree records an op on the current
  snapshot instead of a new row. Agents never flush explicitly.
- **Boot and HEAD.** The primary stream's startup sweep
  (`enqueue_startup_diff`, then a `startup` take) records what changed
  while oxplow was down; a git-refs change runs a `git_refs` take, then on
  a clean tree a `head_moved` re-stamp (`vcs.head.moved@1`).

### Per-effort write log

An effort's files (`effort_file`, [data-model.md](./data-model.md)) are
recorded as the work happens, never declared.

- **Claimed: edits.** The `effort.claim` reactor
  (`tool_call_reactors.rs`, on `agent.tool.finished`) claims every file
  an edit names for the event's **effort anchor**, so a claim landing
  after that effort closed still goes to it; with no anchor the thread's
  open effort takes it. Paths are already relative to the thread's
  worktree; an absolute path (outside it) is never claimed. `record_file`
  is idempotent on `(effort_id, path)` with `source = 'claimed'` and
  replaces an observation. The same event feeds `tool_call.project` (the
  `agent_tool_call` row) and `wiki.attribution` (an edit of an indexed
  `.oxplow/wiki/<slug>.md` marks the page touched by the thread).
- **Observed: everything else a turn changed** (shell, codegen,
  formatters). The `effort.observe` consumer (`effort_observation.rs`,
  after `effort.policy`, on `thread.checkpoint`) content-diffs the turn's
  start → end snapshots (`diff_snapshots`, so an edit-then-revert doesn't
  count) and records each changed path on the effort holding the turn,
  unless another thread's overlapping effort claimed it
  (`effort_store::observe_files_tx`). A path two threads changed and
  neither claimed is observed by both; changes between turns (the
  person's own) belong to no effort. Nothing is reconciled at close.
- **Paths the project never snapshots belong to no effort.**
  `EffortService::claimable_paths` drops what the stream's
  `WorkspaceFilter` excludes (`generated.exclude` and `.gitignore`).
- **A run is the effort its causing tool call was in.** Tests, analysis
  and coverage are recorded whether or not an effort is open; the capture
  is stamped with `CollectionService::resolve_owner` (the tool event's
  effort anchor, else the thread's open effort) as
  `metric_capture.effort_id`, and an effort that adopts a turn restamps
  its captures. `oxplow.test.record_run` is only for counts oxplow
  couldn't parse.

Claims and observations carry the agent session where one is known
([work-tracking.md](./work-tracking.md) "Efforts"). Claims an agent makes
about its work are a different thing: it states the ones it can back in
`oxplow.effort.report`'s `claims` (`v_claim.provenance = 'recorded'`),
and the bundled `infer-reasoning` effect asks a model, when an effort
finishes, for the decisions the agent made without recording them and the
claims its answers made (`inferred`; the `reasoning` job, skipped while no
model serves the `reasoning` profile). Decisions an agent records itself
are `oxplow.effort.record_decision`.

### Token usage capture

**OTEL (Claude Code, Codex).** The control plane's `/v1/metrics` and
`/v1/logs` OTLP receiver (`handle_otlp_metrics`, behind the session's
bearer) hands each export to `otlp_ingest.rs`, which asks every
registered harness's `token_readings` once per export and logs one
`agent.tokens.reported` event anchored to the turn it measured; the
`token_usage.otlp` consumer turns it into `oxplow.tokens` facts (cache
kinds on `oxplow.cache_tokens` / `oxplow.cache_usage`; tokens only, never
dollars). Claude Code's counter is `claude_code.token.usage`; Codex's
counts ride its `response.completed` log event (or the
`codex.turn.token_usage` metric). Details: [metrics.md](./metrics.md)
"OTEL token tracking".

**Per-turn rows (the transcript).** For a harness with `transcript`
(Claude Code), the `token_usage.turns` reactor (`TurnTokensConsumer`,
`crates/oxplow-app/src/token_usage.rs`) reacts to `agent.turn.ended@2`
with `TokenUsageService::on_stop`; nothing is parsed in the hook. It reads
the per-session cursor (`agent_token_cursor`), takes only the complete
lines past it, and the harness's `turns(tail)` splits them into one
`Turn` per agent turn: the person's prompt that opened it plus the summed
usage and `model` of the assistant messages answering it (tool-result
user messages fold into the current turn; a message counts once by its
id). Each turn becomes one `agent_token_usage` row (provenance
`observed`) on the effort the oxplow turn ran in (its anchor, else the
thread's open one), and the cursor advances in the same transaction
(`record_batch`), so a redelivered turn never reads the same bytes twice.
**Bootstrap:** with no cursor yet (a fresh daemon, or the first Stop on a
long transcript) it seeds the cursor at the last complete line and
records nothing, rather than lumping the whole history into one row; the
turn in flight at that first Stop goes uncounted. An ACP agent's counts
come on the Stop body instead (`TURN_USAGE_KEY`) and are recorded as one
row keyed by the event (`agent_token_usage.cause`).

**Prompt capture is observation only.** The prompt text is read from the
transcript the agent wrote — what the person typed into the real CLI.
There is no second input box and no "send prompt" tool. Everything reads
the rows through `v_token_usage`: the bundled `task-tokens` lens
(`work_item.detail.body` slot), `thread-tokens` (`thread.plan.header`)
and the usage lenses. Tables: [data-model.md](./data-model.md).

### Collection command and skill

The `configure` command (`crates/oxplow-agent-text/assets/configure.md`;
`/oxplow:configure` in Claude Code) has the agent instrument the
project's test tooling to write standard-format reports at stable paths,
then records the `testing:` block in `.oxplow/project.yaml` and one report
collector per report in the project's extension
(`oxplow/extensions/project/extension.yaml`). The `oxplow-collection`
skill loads when running tests, on questions about coverage, and on
`configure`; it tells the agent never to parse or report coverage itself,
because oxplow parses the report. The ingestion side is
[collection.md](./collection.md). When the PostToolUse path sees a test
run whose configured report wasn't refreshed, it leaves a one-shot nudge
steering the agent to the report-emitting command
([collection.md](./collection.md) "Report-less-run nudge").

### Nudge persistence

The nudges — the report-less-run nudge and any advisory hit (kind
`<extension>/<advisory id>`), whatever its trigger — are written by pump
reactors (`collection`, `advisories.post_tool` in
`post_tool_reactors.rs`; prompt and turn-end advisories too) to the
`agent_nudge` table (`crates/oxplow-db/src/agent_nudge_store.rs`) with
the kind, message, trigger, turn and **cause** (the event, so a
redelivered event can't fire the same kind twice). **The persisted nudge
is the delivery:** `post_tool_context` settles the two post-tool reactors
(up to 2.5 s), then takes the thread's undelivered nudges
(`take_for_agent`, which stamps `delivered_at`; oxplow's own kinds before
advisories; at most `NUDGE_BUDGET`, 2000 characters, per hook), and
`prompt_context` takes them the same way. One that lands after its hook
answered reaches the agent on the thread's next tool call or prompt.
Nothing is taken for a session whose harness lacks the hook's channel:
its nudges stay undelivered rather than being stamped and lost. A plan's
role-change note wins its call; nudges wait for the next. **One-shot marks
are durable** (`claim_once`, per effort or per thread): the report-less
nudge fires once per effort, an advisory once per its `once_per` scope,
across restarts. A person reads what oxplow told the agent in the bundled
`effort-nudges` lens ("Agent Nudges", `effort.review.details` slot).

### Observe conformance

What core records of a harness's session is checked the same way for
every harness: `oxplow_app::observe_conformance::suite(svc, &Expect {
harness, thread, edited, tokens })` reads the thread's events and checks
the canonical stream — the session started under the harness's key
(`agent.session.started@2`), a prompt opened a turn, the edit was
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
- **The real harnesses, replayed.** One session each of Claude Code,
  Codex and opencode, recorded with `OXPLOW_HOOK_DEBUG`, is checked in at
  `crates/oxplow-harnesses/fixtures/<harness>/hooks.jsonl` (`{event,
  payload}` per line; paths scrubbed to `<ROOT>`, `<HOME>`, `<TMP>`;
  `claude/subagent.jsonl` is a background-subagent session).
  `tests/replayed_harnesses.rs` posts each in order to a live route as a
  session of its harness, its own mapping and answers running, and the
  suite checks the record (tokens aren't replayed). When an agent's hook
  shapes change (Codex renamed its shell tool `Bash` and moved its patch
  to `command`), record a new session, filter it by its `session_id`, and
  replace the fixture: the replay is what notices.

## Pitfalls

- **The write guard refuses edits from a non-writer thread.** An agent
  reporting "permission denied" on a write in a non-writer thread is the
  guard working: promote the thread (`oxplow.thread.promote`) or work in
  the writer thread.
- **Filing work doesn't start an idle agent.** Someone has to prompt it
  ("What we can't do from oxplow hooks").
- **oxplow never commits on its own.** No commit at Stop, no commit
  points. Commits, pushes and checkouts are the `oxplow.vcs.*` commands
  (a person's in the app, an agent's through `run_command`) or plain
  `git`. Shipped oxplow takes no position on whether an agent should
  commit; a person says so in their own instructions.
- **Codex's and opencode's hooks are not Claude's.** Codex has no
  `SessionEnd`, `Notification` or subagent hooks; opencode has no
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

- [work-tracking.md](./work-tracking.md) — efforts, policies and hints
  built on this record.
- [commands.md](./commands.md) — the command bus every agent write goes
  through.
- [data-model.md](./data-model.md) — the tables and models the agent
  reads.
- [providers.md](./providers.md) — a harness as a provider process.
- [terminal.md](./terminal.md) — the PTY pane and resume outcomes.
- [ipc-and-stores.md](./ipc-and-stores.md) — adding an operation to both
  surfaces.
