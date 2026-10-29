# Agent execution model

How oxplow and a coding agent (Claude Code, Codex, opencode) meet: how a session is launched, what every agent is told, the
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
or an agent signed in with a plan, never is). Nothing implements,
holds or calls it: `source_guards::nothing_in_core_drives_an_agent` fails
on production code outside `drive.rs` that names it, so the first driver
is a decision, not a drift.

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
- A chat-library prompt is built only by `chat/human_prompt.rs`
  `compose`, from a person's submit ("The chat machinery (library)");
  nothing in production submits one.
- Drag-and-drop and "Add to agent context" (`agent-input-bus.ts`) are a
  person's gesture: the visible pane pastes the text.
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
- `chat/guard_tests.rs` pins the chat library's prompt path: no production
  code calls `submit_human_prompt` ("The chat machinery (library)").

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
| Drive | oxplow prompting the agent | `drive::Drive`, an interface nothing implements |

The agent's MCP tools ("The agent's tools") are the one channel the agent
itself starts.

## What every agent is told (the text contract)

Every agent session, whatever its harness, is told the same things; a
harness only delivers them.

| Piece | What | Where it comes from |
|---|---|---|
| Preamble | How to work in oxplow: nothing to file, end with the question, reading the errors a person saw, the references oxplow puts in a prompt (`@<path>`, `[oxplow uri <uri>]`, `[oxplow <work item uri>: …]`, `[oxplow page …]`) | `oxplow_agent_text::WORKING_IN_OXPLOW` (`crates/oxplow-agent-text/assets/working-in-oxplow.md`) |
| Launch prompt | The preamble; the session context only for a harness without `prompt_context`; the thread's and stream's custom prompts; `agentPromptAppend`; the testing hint (`testing.agentHint`); the skill index only for a harness without `runtime_text` | `agent_prompt::assemble_system_prompt(config, stream, thread, around, harness, skills)`, given the session's declared harness (`crates/oxplow-app/src/agent_prompt.rs`) |
| Skills | Core's (`oxplow-runtime`, `-wiki-capture`, `-mermaid`, `-collection`, `-metrics`, `-extension`, `-codebase`) and the active extensions' | `AgentText` (`oxplow_agent_text::core_text` + `capabilities::agent_text`) |
| Slash commands | Core's (`review-comments`, `configure`, `new-metric`) and the active extensions' | Same |
| Session context | Stream, worktree, branch, thread, the other live agents in the worktree, the count of open handoffs to the thread | Prompt-time context |
| Handoffs | Work another thread handed this one, once, at the first prompt that takes it ("Handoffs and native messaging") | Prompt-time context |
| Decisions | The open effort's recorded ones | Prompt-time context |
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
`oxplow-wiki-capture`; waiting on the person: `oxplow-runtime`;
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
`SessionStart` never arrives) and on `SessionStart`, so the next
prompt carries the session context and the decisions again. The baselines
are in memory: a daemon restart costs at most one repeated block.

A compaction is also logged, by every harness, as `agent.context.compacted@2`
(`thread`, `turn` when one is open, `trigger`, `summary`): Claude Code's
`PostCompact` (`trigger`, and `compact_summary`, kept as event content and
read as `event:<id>#summary`), Codex's finished `contextCompaction` item
(through the chat host; it names neither), and opencode's
`session.compacted` (through the bridge; it names neither). The log is all
that is done for opencode, whose bridge posts no `PreCompact`. Codex's
compaction also resets the session's baselines (`ChatHost::compacted`),
and the context is injected again once no turn runs. Thread Activity
counts a turn's compactions.

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
| Hooks | HTTP hooks (`hooks.json`) | None: the thread's notifications, in-process (`ServicesChatHost`) | A JS bridge plugin posting Claude-shaped bodies |
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
its own the same way, under its namespace ([extensions.md](./extensions.md)
"Agents: the MCP surface"). With the harness's namespace on top the agent
calls `mcp__oxplow__run_command`, `mcp__oxplow__oxplow_query_sql`. No other
spelling is accepted.

Every tool foundation declares is named below
(`every_foundation_tool_is_in_the_agent_model_doc` pins it).

| Purpose | Tools |
|---|---|
| Read oxplow's data | `oxplow_catalog` (every model with its owner, what it holds and its columns; one model's column docs), `oxplow_query_sql` (one read-only statement over the `v_*` models, live models included — [semantic-layer.md](./semantic-layer.md) "Live models"; its result is also structured content, typed by its output schema), `oxplow_read { uri }`, `oxplow_search` (BM25 over every searchable kind — work items, comments, thread notes, wiki pages, an extension's — and file contents; each hit is a URI) |
| Code and revisions | `oxplow_vcs_log` (a stream's history from any revision, paged, past the index), `oxplow_vcs_status`, `oxplow_vcs_branches`, `oxplow_diff` (versions `working`, `snap:<id>`, `git:<rev>`; `since_fork`), `oxplow_vcs_blame` (through the `Vcs` provider; `working`, `snap:<id>` or `git:<rev>`), `oxplow_code_definition`, `oxplow_code_references`, `oxplow_code_hover`, `oxplow_code_call_hierarchy`, `oxplow_code_symbols` (a file's symbols from its language server, else oxplow's tree-sitter reading of the live file, saying which), `oxplow_code_workspace_symbols` (the editor's language servers, [lsp.md](./lsp.md)), `oxplow_ensure_change` (a change's analysis, then `v_change_*` through `oxplow_query_sql`) |
| Write | `run_command`, after `list_commands` |
| Ask a model | `oxplow_ai_decide` (typed questions about some text, on the `decide` role), `oxplow_ai_summarize` (on `summarize`): recorded computations paid with the person's key, as the asking thread ([ai-providers.md](./ai-providers.md)) |
| Pages | `oxplow_show_page` (shows the person a page beside the conversation — a page with params, or a `spec` of your own, written as a page file on `oxplow:temp` and shown as `oxplow:oxplow/page?page=temp/<id>`; returns its text and whether a window opened it), `oxplow_find_page` (the pages that answer a question, best first, each with its params; `about` a uri keeps the pages that show that kind of thing and binds its uri), `oxplow_run_page` (a page's text, each node under `[node <id>]`, or its loaders' rows; `page` or an inline `spec`), `oxplow_run_page_action` (a page's action as the page acting for you; a row action on one of the node's rows); `oxplow.page.describe { uri, text?, params? }` through `run_command` (what a page file is made of and where each node comes from; `uri` is the page's file on `oxplow:extensions`, `oxplow:temp` or the worktree; a file that is no page gives no rows) — [pages-and-tabs.md](./pages-and-tabs.md) "Page files" |
| Show the person | `oxplow_navigate` (takes them to any URI's page, switching stream when needed; by default the shown tab goes there, pushed; `replace` rewrites its entry, `new_tab` opens another) |
| Common questions, typed | foundation pages with `tool:`: `oxplow_thread_work`, `oxplow_work_next`, `oxplow_effort_summary`, `oxplow_catalog` (each runs `oxplow.page.run` on its page, its params the tool's input: `agent_tools::ToolTarget::Page`). Every other page is `oxplow_find_page` → `oxplow_run_page` (`effort-files`, `file-history`, `recent-commits`, `snapshots`, `thread-sessions`, `session-transcript`) |
| What the person has | `oxplow_list_tabs` (the tabs each of their windows shows, per thread, stream or project: selected, on screen, stale), `oxplow_get_open_page` (the page open in a thread in the window they were in last, the others' beside it, or one listed tab: `page` is its identity, `view` its `#`; the page's run as a tree with their params, and the focused node), `oxplow_list_comments` (their anchored comments, hydrated) |
| Wiki | `oxplow_wiki_ref_drift` (one stale ref's diff against its pin) |
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
Almost every read is a published model: an agent's thread, efforts,
work items, commits, snapshots, dashboards, diagnostics and errors are
`v_*` rows. A tool stays a tool only where a model can't be one: file and
revision content, live language-server answers, site search, a page's
rendering, a dry run.

**`oxplow_read { uri }`** (`oxplow_app::ref_read`) reads anything a URI names and
returns `{ uri, resource, row, related, content, truncated }`: the owning
model's row, `related` lists (a work item's links and comments, a
commit's or an effort's files), and `content` for the resources whose body no
model holds — a file at a revision (`oxplow:oxplow/file?path=src/a.rs` is the working tree;
`oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-40`; `rev=snap:<id>` a local-history snapshot),
an event's stored body (`oxplow:oxplow/event?id=<id>#part=input`, `part=output`, `part=prompt`, `part=summary`; the
caller's own stream's events only), a recorded model call's bodies
(`oxplow:oxplow/ai_call?id=<id>#part=request`, `part=response`; any call but one another agent's
thread asked for — an extension's, a collector's or a job's are the
project's), a page's definition (`oxplow:oxplow/page?page=<ext>/<slug>`). A file or page is
read in the caller's stream's worktree, or another's named by `stream`
(`str2`). Content stops at 64 KB with `truncated: true`; ask for
lines to read further. A captured file's row comes from
`v_snapshot_file`; restoring it is the destructive
`oxplow.snapshot.restore_file`, which writes
into that row's stream's worktree.

**Commands.** Every write is a command through `run_command { id, input
}`. `list_commands` with no arguments is an index of `{ id, summary }`;
with `id` or `prefix` (`oxplow.work_item.`) it gives each command's
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
the wiki (an `oxplow-page` block) or as a page. It returns the page's text.
`oxplow_run_page_action` presses a page's declared action as the page acting
for the caller, under the same rules as running the command directly.

**Server instructions** (`get_info`) are the one text every harness
shows its agent: what oxplow knows (`oxplow_query_sql` over `v_*`), that every
write is a command, and to answer "show me" with `oxplow_find_page` then `oxplow_show_page` (or
`show_page { spec }` for a one-off) rather than printing a table. They ship to users' projects, so they never name this
repo's docs.

**Tools read the interface.** No tool belongs to one implementation:
work-item reads (`oxplow_query_sql` over `v_work_item`, `oxplow_read` of a work item's
URI, the work-next page) go through whichever list is active (empty with
none), so the tool list never changes with a switch. What an
implementation owns is commands, offered only while it's active
(`capabilities::Active::refusal`).

**Comments.** `oxplow_list_comments({ id, scope?, status? })`: `id` is a thread
(`thr…`) or a stream (`str…`); `scope` is inferred from the prefix
(`resolve_comment_scope`; pass it only to assert the kind); `status` is
`all`, `open` or `needs_response`. Each row is an `EnrichedCommentThread
{ thread, primary, context_chain, referenced }` — the anchor target, the
page regions it sat inside (innermost first) and the URIs inside the
selection, each resolved by `oxplow_app::ref_resolver` — so the agent
gets what the highlighted thing is in one call. Replies and resolution
are `oxplow.knowledge.reply_comment` / `update_comment` (`comment: "cmt12"`);
a reply is authored by the actor and clears `needs_response`. oxplow
never prompts for comments: the person runs the `review-comments`
command.

**Code tools.** With no server for a language the error names the Mason
package and both fixes: `oxplow.lsp.install_server { package }`
or an `lsp.servers` entry in `.oxplow/project.yaml`. `v_lsp_server` says
what's configured, installed and running.

**The wiki and links.** Wiki reads are `oxplow_query_sql` over
`v_knowledge_page` / `v_knowledge_ref` (stale pins) and `v_page_ref` (the
backlinks graph, both directions), plus `oxplow_search` with `resources: ["oxplow/wiki"]`;
`oxplow_wiki_ref_drift({ slug, path })` returns one stale ref's unified diff
(`status` drifted, unchanged, not_a_ref, no_pin, binary or no_contents).
Writes are the `knowledge.*` commands ([knowledge.md](./knowledge.md)).
`oxplow.effort.report` and
`oxplow.work_item.create` / `update` check the `[[…]]` links in the text
they keep (`oxplow_app::link_check`) and return `link_warnings`, so the
agent fixes them in the same turn; `knowledge.write_page` refuses
instead. What counts as a link is `oxplow_domain::uri::classify_wikilinks`
([resources.md](./resources.md)).

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

**What a harness implements.** `AgentHarness` requires what every harness
answers: `id`, `title`, `launch`, `tool_use` (its calls in
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
and after commit runs `ChatManager::rename_native` → `Connection::rename`
on the observer connection; a late failure there is logged, the alias kept
(the result says `native_renamed`). The agent session page's **Rename…**
is a form over it (the title, and the box). The new name
returns through `thread/name/updated` into `native_title` like any other.
The UI reads each
harness's features from `list_agent_harnesses` (`HarnessListing.features`),
and its logo (`HarnessListing.icon`: the SVG its `implementations:` entry
names in `icon:`, as a data URI, null when it declares none — no harness id
is named in the UI; "Implementations" in [extensions.md](./extensions.md)):

| Harness (key) | Features |
|---|---|
| Claude Code (`claude`) | terminal, permission_prompts, resume, transcript, telemetry, runtime_text, subagents, prompt_context, tool_context, conversation_list, conversation_fork |
| Codex (`codex`) | terminal, permission_prompts, resume, runtime_text, subagents, conversation_list, conversation_fork, native_rename |
| opencode (`opencode`) | terminal, permission_prompts, resume, runtime_text, subagents, prompt_context, tool_context, conversation_list, conversation_fork |

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
enabled and the first declared is the default. `oxplow.agent.set_agents` and
`oxplow.agent_session.open` check keys against the registry. A harness's
own settings are `agentConfig: { <key>: { … } }`, what its `launch`
reads; it declares the ones a person sets (`AgentHarness::settings`:
opencode's `model`, Claude's `mcpServers` / `model` / `effort`, Codex's
`model` / `effort`; a blank value passes nothing, and a value the harness
refuses is its to refuse), and Settings → Agents renders a field for each,
saved by `oxplow.agent.set_setting`, which refuses an undeclared one (and
the order by `oxplow.agent.set_agents`, which refuses an unregistered key;
`.context/commands.md`). The UI
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
  `--model` / `--effort` come from `agentConfig.claude.model` / `.effort`
  (flags only; the person's `~/.claude/settings.json` is never written).
  Oxplow's server rides beside the
  person's own (local, project, user, plugin servers, claude.ai
  connectors), and `--strict-mcp-config` is added only by the
  `agentConfig.claude.mcpServers: strict` setting (`all` is the default;
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
  `agentConfig.codex.model` is the open request's `model` param and
  `.effort` its `config.model_reasoning_effort` (never the person's
  `~/.codex/config.toml`). Its skills (and
  commands, as skills) go to the worktree's `.agents/skills/`
  (`shared::write_worktree_skills`).
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
stream on another host (no `HOME` passed), reads as unknown.

What can be resumed is listed two ways, both reads of
`AgentHarness::conversations` (`live_datasets::harness_conversations`):
for an open session, the `agent-session-conversations` choices page over
`v_agent_conversation` (the page's Fork into New Session… form:
"Forking a conversation" in terminal.md); for a thread with no session in hand, the
`new-session-earlier` loader over `v_agent_conversation` (the New Session
page, "Earlier sessions"), which adds the stream's closed sessions and says per row whether taking it
reopens, resumes (`agent_session.open { resume }`), switches to a tab or
does nothing. Nothing is adopted until a person picks a row.

### The chat machinery (a library): tool mapping and transcript

**Content text is capped** at 64 KiB per block (`conn::MAX_TEXT`, `cap`,
`cap_value`). A connection turns its agent's traffic into
`chat/model.rs` types (`ToolCall`, `ToolCallPatch`, `ChatUpdate`,
`PermissionAsk` / `PermissionAnswer`); `chat/codex.rs` is the one
production connection.

**Gotcha: `serde_json` runs with `preserve_order` workspace-wide** (the
workspace `Cargo.toml` declares it; the ACP SDK used to switch it on by
feature unification, and removing the SDK changed the checked-in schemas'
key order until it was declared). JSON objects serialize in insertion
order. Anything that needs a stable text form builds it from a `BTreeMap`
(`metric_cube::dims_key` does), and hashes never hash JSON.

**`chat/mapping.rs` (pure)** maps tool kinds onto oxplow's vocabulary
(the kinds in `model.rs` are ACP's, kept because Codex's items map onto
them): edit, delete and move → `edit`; read → `read`; search →
`search`; execute → `shell` (`rawInput.command` / `cmd`, else the title);
fetch → `fetch`; an `mcp__…` title or name → `mcp`; other → `other`;
think and switch-mode record nothing. Paths come from `locations`, then
the diffs, then path-like `rawInput` keys (`file_path`, `path`,
`absolute_path`, `notebook_path`, `source`/`destination`,
`old_path`/`new_path`), deduplicated. The stored `content` is `rawInput`,
and once the call finishes `{is_error, …rawOutput}`, so a shell call's
`exit_code` survives.

**`chat/transcript.rs`** is the in-memory conversation (no table). Items:
user, agent, thought, tool (merged with its updates by call id), plan
(replaced within a turn), permission, policy-denied, bypass, error. Each
has a stable `id` and a `seq` bumped from one counter whenever it
changes; `since(seq)` returns new and changed items, which clients upsert
by `id`. Chunks of one kind coalesce into the trailing item;
`usage_update` is state (the context meter), not an item. The ring holds
2000 items (`DEFAULT_CAP`). The Assistant (`assistant/`,
[ai-providers.md](./ai-providers.md) "The Assistant") keeps its
conversation in the same `Transcript` and the same `ToolCall` and
permission items; it is not an agent session. A `ToolCall` carries the
`auditId` of the command run it made, when it made an audited one.

### The chat machinery (a library)

oxplow once opened ACP agents as chat sessions with their own view. That
product surface is gone, and so is the code that spoke ACP (the SDK, the
stdio client, the scripted ACP agent): nothing in `crates/oxplow-app/src/chat/`
speaks ACP. What remains is the **session actor** (`chat/session.rs`)
and the one connection in production, the **Codex observer**
(`chat/codex.rs`, "Codex: its TUI on a thread oxplow observes"). The
actor is written against the `Connection` trait (`chat/conn.rs`: open,
prompt, cancel, closed, inject, rename; and the `Incoming` messages a
connection feeds it), so another agent protocol is one more
implementation.

**Shape.** `Services.chat` (`chat/manager.rs`) holds each agent session's
command sender and a shared `SessionView` (status, transcript, stderr
tail), keyed by `AgentSessionId`. One actor task per session owns the
connection and sees updates, requests and turn end in the order the
connection sends them. Events for every session share one broadcast
channel (`ChatEvent`). `open_codex` reserves the session's slot under one
lock before connecting, so concurrent opens start one observer; `close`
marks the handle closed at once, and a closed session still winding down
is replaced and marked not current, so it records no Interrupt over its
successor. Closing a thread (`oxplow.thread.close`) closes its open
effort and, once committed, stops every open session's process through
`agent_sessions::SessionProcesses::kill`.

**Host.** `chat/host.rs` `ChatHost` is the seam (tests use a recording
double). `ServicesChatHost::new(svc, stream, session)` holds `Weak<Services>`
and the session it hosts, and records what a hooked turn records:
`SessionStart` plus the resume id on start; `UserPromptSubmit` on the
person's prompt; `PreToolUse` on every policy check; `PostToolUse` on each
finished call; `Stop` with the turn's counts (`TURN_USAGE_KEY`), its
last agent message (`LAST_ASSISTANT_MESSAGE`) and the model and effort
the observer reports (`turn_run`); `Interrupt` when the agent
goes away.

**Starting.** A connection's `open` starts the agent's session or
continues the saved one; what a continued session replays reaches the
actor before `open` returns and rebuilds the transcript while recording
nothing (no hooks, no tool rows, no bypass checks, file writes refused).
`Opened.preamble` is a system prompt the agent had no other way to take;
it rides ahead of the first prompt only.

**Prompts (the no-automation rule).** `chat/human_prompt.rs` `compose` is
the only way to make a `HumanPrompt`, and `Connection::prompt` takes
nothing else. The session calls it only for `Command::Prompt`, which only
`ChatManager::submit_human_prompt` sends; a second prompt while a turn runs
is `TurnInFlight`, never queued. The Codex connection refuses a prompt (the person
types in Codex's own TUI). The session context, advisories, decisions and
waiting nudges ride the person's prompt as a visible leading block
(behind a disclosure on the user item). `chat/guard_tests.rs` scans the
source: no production code starts a Codex turn (`"turn/start"`), context
reaches a Codex thread only from `codex.rs`, `compose(` is called only
from the session, and no production code calls `submit_human_prompt(`
outside `chat/manager.rs`.

**Permissions.** Each `Incoming::Permission` asks `AgentPolicy`. A
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
Stopped/Closed; the actor runs it on a clean exit and the manager task
again afterwards (a no-op once stopped), so a transport error still ends
the session.

**File requests and bypass.** `Incoming::WriteFile` / `ReadFile` (a
connection whose agent asks oxplow to touch files) are policy-checked; a
deny reaches the agent with its reason. oxplow reads and writes files for
an agent only inside the session's worktree (paths normalized first;
relative ones against the cwd). A write-kind call that completes after
the policy or a person rejected it gets a bypass banner; one that
completes with no one allowing it, and not every path written through
`WriteFile`, is checked afterward and bannered if the policy would deny
it. At turn end, open cards are cancelled and the Stop and tokens
recorded.

**Tests.** `chat/session_tests.rs` drives the actor with `FakeConn`, an
in-process `Connection` scripted by `fake:<step>` lines in a prompt (say,
think, edit, edit-anyway, bypass, fswrite, fsread, bash, tokens, wait,
crash), and with a Codex thread on a scripted app-server daemon
(`FakeAppServer`). `ChatManager::open_connection` (test-only) starts a
session on any `Connection`.

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
`open_terminal_session` opens the observer (`ChatManager::open_codex`,
which keeps one already running and answers its thread's id), then
spawns the session's PTY as any agent pane, running `attach` with the
thread id appended. The thread's id is the session's resume id.

**The observer.** The chat session's actor (`chat/session.rs`) over
Codex's `conn::Connection`, with no view. It:

- persists a new thread before answering (`thread/goal/set` then
  `thread/goal/clear`), so the TUI can resume it;
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

The session actor (`chat/session.rs`, `inject_context`) therefore builds
the context with `AgentContext::build_context` — the same "once, then on
change" baselines, the `<handoffs>` block, nudges and decisions a prompt
carries — and injects it (`Connection::inject`), only when the connection
says it can (`Connection::can_inject`: the thread runs no turn, read from
`turn/started` / `turn/completed`; and the daemon knows the method, else
the first refusal, "unknown variant", turns it off for the connection).
It does so at four moments: when the session opens, when a turn ends,
after a compaction (`ChatHost::compacted` resets the baselines; one inside
a turn waits for that turn's end), and **on a change**: the
`agent.context_changes` pump consumer (`context_watch.rs`) bumps
`Services::context_changes` when the log records a fact the context
reads — `handoff.sent` / `handoff.settled`, `agent.session.started` /
`.ended` (who else is in the worktree), `effort.opened` / `.closed` /
`.finished` / `.linked` / `.retitled` and the decision reviews, and the
`oxplow.effort.record_decision` command — and each session watching it
builds again. A build that changes nothing is `None` (the baselines), so
a no-op event injects nothing; a change during a turn waits for its end.
No polling. The launch prompt's `developerInstructions` leave the session
context out (the harness declares `prompt_context`); the first injection
carries it.

**The context basket.** What the person collected for the thread's
agents (`thread_context_item`, `v_thread_context`, the `oxplow.context.*`
commands) rides the next prompt of *any* agent in the thread as a
`<context-basket>` block after the handoffs (`agent_context::basket_block`):
each page as `[oxplow uri …]` with its label, each piece of text quoted
(up to `BASKET_TEXT_IN_CONTEXT` characters) with where it came from.
`commit` stamps the items it carried delivered to the session, which
empties the basket; a delivered item stays as a record. Claude Code and
opencode get it at prompt submit. Codex gets it by injection while no turn
runs (an `oxplow.context.add` run wakes `context_watch`), so with two
agents in a thread an idle Codex can take the basket before the agent the
person prompts. The basket is the person's: an agent's `context.*` run is
refused. Pinned by `the_context_basket_reaches_the_next_prompt_once`.

*The UI* (`apps/desktop/src/contextBasket.ts`, the window's selected thread
from `setSelectedThread`): **Add to Context** is on every page's ☰
(`menu-item-page-add-to-context`, beside Ask; `oxplow.context.add` with the
page's canonical ref and title; disabled with a reason when no thread is
selected) and on selections: an editor's and a diff's right-side selection
(**Add Selection to Context**, text plus the `#lines=` URI), a page row's
menu (**Add to Context**: the row's first linked page, else the row as
text) and the floating toolbar over a selection on a commentable page.
Each toasts with Undo. The **Context** dock panel (`@`, badge = waiting
items) lists what waits with × per item (`oxplow.context.remove`), Clear
(`oxplow.context.clear`) and a link to the **Context** page (foundation page
`context`: Waiting and Delivered (last 20) over `v_thread_context`, row
action Remove, Clear in the nav bar).

**Building takes nothing; delivery commits.** `build_context` returns a
`PendingContext` and changes no state: no baseline moves, no handoff or
nudge is stamped. `AgentContext::commit` records the blocks as the
session's baselines, stamps the handoffs it carried delivered (by id) and
marks its nudges, and runs only once the agent has the text: the hook
answer for Claude Code and opencode (`prompt_context` is build then
commit, the answer counting as the success), `conn.prompt` for an ACP
agent, the daemon's `{}` for an injection. A refused injection commits
nothing, so a handoff stays undelivered and the next chance carries it
whole.

Pinned by `context_is_injected_between_turns_never_during_one`,
`a_refused_inject_loses_nothing` and
`a_change_while_idle_is_injected_before_the_next_turn` (a scripted
daemon), `a_context_that_was_not_delivered_takes_nothing` and
`a_handoff_tells_the_context_watchers`, `a_recorded_inject_starts_no_turn_and_waits_for_none`
(`tests/fixtures/codex/inject.jsonl`, recorded on the live daemon),
`nothing_is_injected_into_a_running_turn`, and the source guard
`only_the_codex_connection_injects_context` (the drive guards stay:
`nothing_starts_a_codex_turn`).

**Pitfalls.** `/new` in the TUI starts a thread with the TUI's own
config, which oxplow doesn't bind: use Start Fresh.

**Collab agents are subagents** (feature `subagents`). A Codex thread can
spawn child agents, each its own thread on the daemon, and the daemon
sends this client the child threads' notifications too. The observer
(`chat/codex.rs`, `on_message`) maps what it sees; it never calls a collab
tool and never starts, interrupts or closes a child, and a child gets no
session slot, bearer or process of oxplow's:

- `subAgentActivity` `started` / `completed` / `interrupted` on the
  parent thread (`agentThreadId`, `agentPath`) become
  `agent.subagent.started@2` / `.finished@2` (`Incoming::Subagent` →
  `ChatHost::subagent` → a `SubagentStart` / `SubagentStop` hook), the
  subagent's id the child thread's id and its kind the `agentPath`
  (`/root/count_lines`). A `collabAgentToolCall`'s `agentsStates` do the
  same for the children it names (a terminal state finishes one, with its
  `message` as the result), once per child whichever item says it first.
  Recorded 2026-10-16 on 0.162.1: a spawn shows only as a
  `subAgentActivity` (no `spawnAgent` collab item); `wait` is a
  `collabAgentToolCall` with no receivers.
- A `collabAgentToolCall` is also the parent's own tool call, kind
  `subagent`, named by its tool (`wait`, `spawnAgent`, …).
- A child thread's **finished tool calls** (`item/completed` of a command,
  a file change, an MCP call, …) are recorded in the parent's session as the
  subagent's (`ToolUse.subagent`, `v_tool_call.subagent_id`), so its edits
  are claimed by the parent's effort. Only those: a child's messages,
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
  patch names (`shared::patch_paths`). An ACP agent's calls and a Codex
  thread's items are mapped by `chat::mapping` (Codex's onto ACP's tool
  kinds first, "Codex: its TUI on a thread oxplow observes").
- **Where.** The hook route maps a PreToolUse / PostToolUse body with the
  bearer's harness before the policy runs and hands the call to the
  ingest (`HookEnvelope.tool`); the ACP host does the same with its
  mapping. The ingest maps nothing: it runs in one transaction, and a
  harness's `tool_use` may be a provider's call. A body naming no tool
  records nothing.
- **What reads it.** The write guard (`edit`, every path), the status
  derivation (`subagent`, `ask`, `plan`), effort file claims (`edit`), test-run collection (`shell`, its `command`
  and `exit_code`), a turn's writing calls (`thread.checkpoint`, by
  `v_tool_call.kind`), the bundled struggle model, `v_context_read`, and
  `agent.tool.requested@3` / `agent.tool.finished@3`, which carry `tool`,
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
  (`OTEL_EXPORTER_OTLP_HEADERS`), opencode's `{env:…}` header, an ACP
  agent's `McpHttp` entry, and a Codex thread's `config.mcp_servers`
  header, sent over the daemon's socket. A bearer is never
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
session's context baselines, and `PostCompact` is logged (`agent.context.compacted`).

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
  (`agent.turn.ended@3`, with Claude's `transcript_path`), then the
  turn-end snapshot ("Snapshot tracking"). A Stop also sets the turn's
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
session (`TerminalSessionRegistry::ingest_exits_into`), once.

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

**The two rules:** no thread may change another stream's worktree, and
wiki pages are written by command. Everything else is open to every
thread: its own stream's worktree, files outside every stream, the shared
wiki (through `oxplow.knowledge.write_page`), and oxplow's own objects —
work items, efforts, notes, comments. The bus's `WriteGate` asks only that
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
  else the default harness's. Every built-in renders Claude Code's shape
  (`hookSpecificOutput`, `shared::render`), which opencode's bridge speaks
  too. A chat session's agent (ACP, Codex) gets a permission reject or an
  fs error.

### Isolation

An edit (`kind: edit` — Claude's `Write`/`Edit`/`MultiEdit`/`NotebookEdit`,
opencode's `apply_patch`, an ACP agent's or a Codex thread's edit, delete
or move) is refused in two places, and nowhere else
(`crates/oxplow-runtime/src/write_guard.rs`):

- **Another stream's worktree** (the primary checkout included), for
  every thread (`write_guard::foreign_worktree_reason`, naming the path
  and the tree). Its own stream's worktree, and an absolute path outside
  every stream's (`~/.claude/plans/foo.md`), are allowed.
- **Wiki pages are written by command.** A direct write to the primary
  project's `.oxplow/wiki/` is denied for every thread
  (`write_guard::wiki_page_reason`); pages go through
  `knowledge.write_page`.
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
the status — `AwaitingUser` when the final message ends in a question,
keeping an `awaiting_user` the turn set — and acks `{}`. Without
`oxplow.effort.report`, an effort's summary is its last turn's answer.

### Subagents

**Who in a session acted.** A subagent (Claude Code's `Agent`/`Task`, an
opencode child session, a Codex collab agent) is not a session: it has
no bearer and no process oxplow knows. It is who made a call inside its
parent's session: `ToolUse.subagent { id, kind? }`, on
`agent.tool.requested@3` / `agent.tool.finished@3` and
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
they fall, a subagent's hand-back marked (a turn with no prompt), each
row a turn URI. A turn URI opens it at that turn (`v_agent_turn
.transcript_page`, built with `uri('oxplow/page', …)`). Its action **Resume in a new session**
runs `oxplow.agent_session.open` with `resume`: a saved conversation of
the harness, bound `chosen` in the same transaction, refused when it is
open in another session. A person's: an agent may open a session but not
choose its conversation.

Subagent tool calls reach the hooks like any other, so their edits and
test runs land in the thread's effort with nothing from them: there is
no dispatch protocol or subagent skill.

**What to pick up next** is the foundation page `oxplow-foundation/work-next`
(the tool `oxplow_work_next` with `thread: "thr3"`) over `v_work_item`, whichever
list is active: an epic with its ready descendants, or every ready
non-epic item, open `blocks` links holding an item back; empty with
nothing ready or no list. The bundled `work-next` command calls that tool and
moves the item it picks to `in_progress`, so the effort links to it.
Claude Code's `TaskCreate` is a within-turn planner and never mirrors
oxplow's items.

### Native features: observed, set, left native

Each harness has interactive features of its own. oxplow **observes and
shows** what it can hear, **sets at launch** what a flag or open request
takes, and **leaves to the harness** the rest — never answering for the
person: a prompt or question is answered in the harness's own TUI, which
is in the tab already ("Key invariant"). The audit behind this table was
made against Claude Code 2.1.301, Codex 0.163 (daemon) / 0.162.1 (CLI)
and opencode 1.17.4, from their documentation and protocol schema;
*italic* is available from the harness but not used yet, — is not
available from outside.

| | Claude Code | Codex | opencode |
|---|---|---|---|
| Waiting on a permission | `Notification` `permission_prompt` / `agent_needs_input`, and the `PermissionRequest` hook (observed, answered `Ack`) → AwaitingUser | `thread/status/changed` `activeFlags` `waitingOnApproval` → AwaitingUser, the request's command or reason as detail; cleared by the flags or `serverRequest/resolved` | the bridge forwards `permission.asked` (a `Notification`) and `permission.replied` |
| Asked a question | `AskUserQuestion` PreToolUse → AwaitingUser with its question; a `?`-ending Stop | `waitingOnUserInput` with `requestUserInput`'s first question; a `?`-ending Stop | `question.asked` (an Ask `PreToolUse`); a `?`-ending Stop |
| Answering from oxplow | — (no interface into an interactive session; a cross-session message can't consent) | — (a request goes to the turn's client) | *HTTP `permission` / `question` reply* — left native |
| The plan | `ExitPlanMode` PreToolUse `tool_input.plan`, `planFilePath` → `agent.plan.proposed` | the `plan` item's text on `item/completed` → `agent.plan.proposed`; `turn/plan/updated` steps reach only the chat library | — (`plan_exit` carries no text, so none is recorded) |
| Model / effort | `agent_turn.model` / `effort`: the Stop's `effort.level` and the transcript tail's newest `message.model`; transcript `message.model` → `v_token_usage.model`; *`SessionStart.model`, `PostModelSwitch`* | `agent_turn.model` / `effort`: `Thread.model` / `reasoningEffort` at open, moved by `thread/settings/updated` | `agent_turn.model` / `effort`: the bridge's Stop carries the last assistant message's `modelID` and `variant` |
| Set at launch | `agentConfig.claude.model` → `--model`, `.effort` → `--effort` | `agentConfig.codex.model` → the open request's `model`, `.effort` → its `config.model_reasoning_effort` | `agentConfig.opencode.model` → `-m` |
| Context fill, compaction | `agent_turn.context_tokens` (window NULL): the transcript tail's newest assistant usage; `PostCompact` → `agent.context.compacted` (`trigger`, `compact_summary`); `PreCompact` resets the context baselines | `agent_turn.context_tokens` / `context_window`: `thread/tokenUsage/updated` `last.totalTokens` / `modelContextWindow`; the `contextCompaction` item → `agent.context.compacted` (resets nothing: no prompt channel) | `agent_turn.context_tokens` (window NULL): the Stop's `tokens`; `session.compacted` → `agent.context.compacted` |
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

### Handoffs and native messaging

Work moving between threads is a **record oxplow keeps, never a message
it delivers.** Without it the person carries a result from one terminal
to another, or agents use their harness's own messaging, which oxplow
doesn't see, which nobody acknowledges, and which wakes the recipient at
the sender's choice. A handoff gives that move one durable, visible fact,
and every transport stays native.

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

**The record** (`crates/oxplow-app/src/commands/handoff.rs`,
`oxplow_db::handoff_store`, `v_handoff`, URI `handoff:hnd<n>`):

- **From** a thread: an agent's own (naming another is refused), an
  effect's event's, or the one a person names; the session and the actor
  kind are the actor's, never an input. **To** an open thread, never a
  session (the thread outlives its processes), any stream's: a handoff
  is knowledge, not a write into that worktree. To the sender's own
  thread is refused ("a note is for that"). An optional **work item** on
  the active list (a loose id is canonicalised), and a Markdown **body**
  whose `[[…]]` links are `page_ref` edges, checked like a note's
  (`link_warnings`) and restated by the boot repair. Searchable
  (`v_search_handoff`, under the recipient's stream).
- **Commands**, all `Record` and open to every invoker (scope
  `handoffs.write`): `oxplow.handoff.send { to_thread, body, work_item?,
  from_thread? }` logs `handoff.sent@2`, not undoable.
  `oxplow.handoff.settle { handoff, state, note? }` is the record of how
  it ended — `taken` / `declined` by the recipient thread, `withdrawn` by
  the sender, each only while `open`, and `open` again by whoever settled
  it — logging `handoff.settled@2`, undone by the state it replaced.
  `oxplow.handoff.resolve { handoff, outcome: taken | declined, note? }`
  is how a recipient takes one: a native composite that checks the same
  rule (`may_settle`) against the run's actor before it composes, so a
  stranger's run moves nothing, then composes `oxplow.work_item.move`
  onto the recipient thread (when taking an item not there yet, on a
  list declaring `lists`) and the settle. Ownership stays the item's
  thread. A list's verbs run outside the transaction, so a take that
  moves an item runs as steps and has no undo; the recipient reopens it
  with `settle { state: open }`. A decline undoes.
- **Thread notes stay as they are.** They were the candidate carrier and
  don't fit: a note belongs to its writer's thread, has no recipient,
  state or delivery stamp, and the Explore hand-back relies on that.

**When the recipient sees it.** Its agent: at the start of its next
prompt. `AgentContext::build_context` reads the thread's open,
undelivered handoffs (`SqliteHandoffStore::peek_for_prompt`) and
`AgentContext::commit` stamps `delivered_at` and `delivered_to_session_id`
(`mark_delivered`, by id) once the agent has them and puts them in a `<handoffs>` block after the session
context: each one's URI, the sender's thread and session (or "the
person"), when, its item's quoted title, the body up to
`HANDOFF_BODY_IN_CONTEXT` characters with its URI for the rest, and how
to resolve it. Never with a tool call: a handoff is a unit of work, not
a nudge, and mustn't land in the middle of other work. The session
context counts the thread's open ones (`- **Open handoffs to this
thread:** N`, `agent_context::surroundings`), so a later session or a
restart learns of them without the bodies again. Codex gets the whole
block too, injected between turns. The take is two steps
(`peek_for_prompt`, then `mark_delivered` by id once the agent has the
text), so a delivery that fails stamps nothing.

The person: at once, through the foundation extension (core's own, so a
project with `oxplow-bundled` off still has it):

- **Hand Off Work…** is `oxplow.handoff.send`'s `ui` (`about: thread`),
  so it is on a thread's page and at the end of its Navigator menu. Its
  form is the `hand-off` page (`viz: form`, `from_thread` fixed to the
  thread), which picks the recipient and the item from lists (`choices`:
  the `handoff-recipients` and `handoff-items` pages) and takes the body
  as Markdown.
- The thread's Tasks page shows **Handoffs to This Thread** (Take,
  Decline… with a note) and **Handoffs From This Thread** (Withdraw) in
  its `thread.plan.header` strip, open ones first, settled ones for a
  week.
- The Navigator counts each thread's open inbound handoffs beside its
  title (`useOpenHandoffs`), the newest sender in the tooltip.
- oxplow-bundled's Waiting on Me lists an open handoff to a thread with
  no agent session open there: the person's move. Agent status is not
  touched; it stays derived from agent activity.

`tests-e2e/specs/work/handoff.spec.ts` drives it: hand off from the
thread's menu, see it on the other thread, take it, the item moves.

**Nothing wakes a model.** oxplow never opens an inbox socket, a channel,
a server prompt or `turn/start` for a handoff: those are Drive on an
interactive surface ("Key invariant").
`source_guards::a_handoff_reaches_an_agent_only_at_its_prompt` pins
that `peek_for_prompt` has one caller and that no production code names
`prompt_async`, Claude Code's channels, or its inbox socket (outside the
harness that scrubs it from a child's environment). A restart loses
nothing: the record and its stamps are in the database. Pinned by the
`commands::handoff` tests and
`a_handoff_reaches_the_recipients_next_prompt_once`.

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
The Claude harness's `prompt` reads it as `Prompt::PeerMessage { from }`
(the session name, else the socket), not the person's words, as it reads
a subagent's hand-back. The control plane passes it as
`HookEnvelope.peer`; the ingest opens a turn with an empty prompt when
none is open (none if one is), logs `agent.peer_message.received@2
{ thread, turn, from }` (no `agent.prompt.submitted`) and sets the
session running. `v_agent_turn.from_peer` names the sender of a turn it
opened, and Thread Activity titles that turn "Message from …". Pinned by
`a_peers_message_opens_a_turn_with_no_prompt`, `a_peers_message_is_not_a_persons_prompt`
and the recorded `claude/peer_message.jsonl`
(`a_recorded_claude_code_peer_message_opens_a_turn_of_its_own`). Codex's
collab agents are subagents ("Collab agents are subagents"). Observe
only: oxplow never sends a peer message and doesn't read Claude's team
mailboxes (not a documented interface).

Cross-thread ownership and duplicate work are visibility, not gates
([work-tracking.md](./work-tracking.md) "Cross-thread ownership").

### Validated against, and what pins each scenario

The session integration is checked against these versions: **Codex CLI
0.162.1** (the 0.163 daemon was probed for "Codex: its TUI on a thread
oxplow observes"), **Claude Code 2.1.301**, **opencode 1.17.4**; the
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
| No terminal input from restore, metadata, tab discovery or handoff | `terminal_input_has_single_production_caller` and `no-agent-input-automation.test.ts`: the one way bytes reach a PTY is the person's keystrokes |

Not yet covered: a recorded Claude hook carrying `session_title` (filed).

### Plans

A plan the agent puts to the person is an `agent.plan.proposed` event
(`{ thread, turn?, text, path? }`), anchored to the turn and effort open
when it came. The text sits in the payload, capped at 64 KiB, so a model
reads it. Claude Code's `ExitPlanMode` PreToolUse is mapped by the
harness (`claude_shaped_tool_use`: `ToolUse.plan` from `tool_input.plan`
and `planFilePath`; core never names the tool); ingest logs the event
beside the tool request, once per call id, and only for an allowed call.
Codex has no tool call: the observer maps a completed `plan` item (its
text is authoritative) to `Incoming::Plan`, which the chat host ingests as
a `PlanProposed` hook with `tool.plan`; that waits on no one. opencode's
`plan_exit` carries no text, so it records none.

`v_agent_plan` (thread, effort, turn, session, `proposed_at`, `text`,
`path`, and `event_id`, the event to read with `oxplow_read event:<id>`)
reads them; the bundled `effort-plan` (slot `oxplow-foundation/changes.sections`) and
`thread-plan` (slot `thread.plan.header`) pages show a plan, newest
first.

**Whether the person accepted the plan is not recorded.** No hook or
notification carries the dialog's answer; a later edit or a new prompt
only suggests it.

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
(a `Notification`) and `question.asked` (an Ask `PreToolUse`); the
`*.replied` events become a tool-less `PostToolUse` that clears the wait.
ACP permission cards log it through `set_status`.

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
- **Turn events.** `agent.turn.started@2` / `agent.turn.ended@3 {
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
  a clean tree a `head_moved` re-stamp (`vcs.head.moved@2`).

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

**OTEL (Claude Code).** The control plane's `/v1/metrics` OTLP
receiver (`handle_otlp_metrics`, behind the session's bearer) hands each export to `otlp_ingest.rs`, which asks every
registered harness's `token_readings` once per export and logs one
`agent.tokens.reported` event anchored to the turn it measured; the
`token_usage.otlp` consumer turns it into `oxplow.tokens` facts (cache
kinds on `oxplow.cache_tokens` / `oxplow.cache_usage`; tokens only, never
dollars). Claude Code's counter is `claude_code.token.usage`. A chat
session's tokens (ACP, Codex) ride its Stop instead ("Codex: an
app-server client"). Details: [metrics.md](./metrics.md)
"OTEL token tracking".

**Per-turn rows (the transcript).** For a harness with `transcript`
(Claude Code), the `token_usage.turns` reactor (`TurnTokensConsumer`,
`crates/oxplow-app/src/token_usage.rs`) reacts to `agent.turn.ended@3`
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
the rows through `v_token_usage`: the bundled `task-tokens` page
(`oxplow-foundation/work-item.sections` slot), `thread-tokens` (`thread.plan.header`)
and the usage pages. Tables: [data-model.md](./data-model.md).

### Collection command and skill

The `configure` command (`crates/oxplow-agent-text/assets/configure.md`;
`/oxplow:configure` in Claude Code) has the agent instrument the
project's test tooling to write standard-format reports at stable paths,
then records the `testing:` block in `.oxplow/project.yaml` and one report
collector per report in the project's extension
(`.oxplow/extensions/project/extension.yaml`). The `oxplow-collection`
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
its nudges stay undelivered rather than being stamped and lost. **One-shot marks
are durable** (`claim_once`, per effort or per thread): the report-less
nudge fires once per effort, an advisory once per its `once_per` scope,
across restarts. A person reads what oxplow told the agent in the bundled
`effort-nudges` page ("Agent Nudges", `oxplow-foundation/changes.sections` slot).

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
  `claude/subagent.jsonl` is a background-subagent session).
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
