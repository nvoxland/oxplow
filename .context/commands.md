# Commands: the one write path

What this doc covers: the command bus — what a command declares, who
may run it, the pipeline every run goes through, the audit and undo,
and how a new command is added. Built in
P1.8–P1.10 (tsk410–412): the bus, `config.*`, and the generic MCP
surface (`list_commands` / `run_command`) with caller identity.

## What a command is

A command is a typed operation identified by its **command id**,
`<namespace>.<area>.<verb>` in snake_case (`oxplow.work_item.transition`,
`oxplow.config.set`, `acme_pr.issue.close`; `CommandSpec::validate_id`).
The id is the machine name; what a person reads is the command's label.
**An input field named for one resource takes a bare id** (`{"thread":
"thr3"}`, `"stream": "str1"`, `"effort": "eff12"`, `"comment": "cmt4"`,
`"note": "not7"`, `"claim": 7`; an integer id is a JSON number; "Bare ids"
in [resources.md](./resources.md)), parsed by `commands::util::bare_id`
(it replaced `ref_id`) — a URI is refused at its field, saying the id it
names. A field that can name anything (`uri`, `target`, `subject`, a work
item) still takes a URI. A result names the records it made the same way
(`effort.open` returns `{"effort": "eff12"}`), so it feeds the next call.
`CommandError::Proposed` carries the integer `proposal` and `supersedes`. (The
work-item interface keeps its own `thread` and `move { to: { thread } }`
ids: they cross the provider protocol. Hints and file
snapshots have no resource yet and are named by their ids.)
The **namespace** is its owner's, so two extensions' areas never collide:

- `oxplow` — **reserved** for oxplow's own commands: core's, and those of
  the extensions that ship with oxplow (`oxplow-bundled` declares
  `namespace: oxplow`). They share it, collisions checked per id, so a
  command moving between core and a shipped extension keeps its id.
- An extension's declared `namespace:` (default: its name with `-` →
  `_`) — that extension's alone, its external providers' commands included (one
  id for every instance; the input says which runs it).

The registry records each command's **source** (`core`,
`extension:<name>`; `CommandBus::source_of`) and each
namespace's **holder** (`namespace_owner`: `oxplow`, or the one source
holding it). `register` is core's (under `oxplow`); `register_namespace(ns,
source, commands)` registers a source's commands all-or-nothing. Duplicate
IDs within the submitted batch are rejected before mutation. For replacement,
`replace_namespace` delegates to `replace_namespaces`: an atomic list of
`NamespaceCommands` (namespace, source, prepared commands). Each source appears
once. Every replaced source is removed from a cloned registry before any new
batch is validated, so participating sources can exchange namespaces, IDs or
shortcuts. Unmentioned sources remain intact. The final registry swaps under
one write lock; any rejection preserves every prior command and holder. An
empty command set removes its source. Commands already resolved keep their Arc
handlers. This boundary covers the command registry, not SQL models or the other
extension registries. `prepare_namespaces` validates a candidate off the active
registry lock; `publish_namespaces` accepts it only on the exact registry used
for preparation. Every successful registration/replacement/removal advances an
allocation identity retained by candidates, avoiding revision wraparound and ABA.
An intervening mutation or publication on another bus returns Unavailable and
leaves the active registry intact. Publication remains an in-memory operation,
not a DB commit boundary.

The native generation coordinator can reserve a checked command candidate and
its extension owner metadata before SQL commit, then apply the replacement while
retaining both locks through the remaining generation maps. Until the first generation is
activated (boot's `initialize`), command publication keeps its optimistic
checks. The command bus, SQL gateway, AI service, capability registry and
provider registry are built holding the project's activation boundary (a part
built alone, as the CLI's checks do, has its own, which nothing has
activated), so a pin is always available.
`CommandBus::with_activation` sets the ingress boundary: a pin is
attached for command lookup and retained through admission, policy,
confirmation and the run of a `Tx` handler, including prepared effect commands,
compositions and post-commit projections. An `External` handler runs after the
pin is released (`Flow::External` / `run_external`): it makes its own host
calls, each pinning for itself, so an activation never waits for a long
collector run or a provider call. Its run carries no pin
(`RunContext::without_host_generation`). A standalone
bus retains its previous behavior. Production Services adopt the same boundary
as the SQL gateway and provider registry during boot.

Extension commands are staged together from one CatalogSnapshot and published
as one fenced registry candidate by the generation coordinator (activation) —
the only path that registers them. An enabled extension that fails to stage is
held back on its own (the generation names it in its problems); partially
parsed command lists never replace prior handlers. Namespace/shortcut
validation spans all sources. Disabled/removed sources are withdrawn in the
same candidate, and captured command contexts have a live owner check so
already-routed handlers cannot preserve that authority. A catalog change during
staging rejects publication and the owner stages again. Raw filesystem
snapshots remain authoring inputs. `unregister_source` removes one source's (an extension disabled, an
external provider stopped), freeing a namespace left empty. Migration V26 rewrote
history (audit, proposals, `command.*` payloads) from the old two-part
ids. A command declares (`oxplow_domain::commands::CommandSpec`):

| Field | Meaning |
|---|---|
| `input_schema` | JSON Schema; the bus validates the input first and names the failing field, plus what the schema accepts there (`InputValidator::check`: an object's fields, required first, for an unknown or missing one; the choices for a bad enum value and that field's `description`), and quotes the schema's example input, so a caller fixes it in one more call. The example is JSON Schema's own `examples` keyword, from the command's declaration (`example:` in its manifest, kept over its operation's schema by `Op::command`; a script command's first example that runs when it declares none; `oxplow.command.sequence`'s in `compose.rs`). Every command an agent is offered has one that fits (`every_command_an_agent_is_offered_has_an_example_that_fits`) |
| `job` | a model's tool call inside one of its extension's jobs may run it (`job: true`; `false` unless said, and only on a command with its extension's own handler — `Op::command` refuses it over a scope's operation, so none of core's commands takes a job). Who else may run a command isn't the command's to say: **anyone** — a person, an agent, a lens acting for either. oxplow puts no wall between a person and their agent; what an agent may call is its harness's to decide (its permission mode) |
| `confirm` | `Never`, `Always`, or `Destructive` — a **person's** run asks them first (the second click). An agent's run doesn't ask (`commands::asks`); an effect's is kept as a proposal; a job's or oxplow's is refused. A command that only reads (`View` / `Read`) may not ask (`Command::new` / `with_confirm_for` refuse it — so a read or view scope's command can't declare `confirm`): it runs unrecorded, so nothing would resolve its proposal |
| `undoable` | the handler returns an inverse call that `undo` applies |
| `lifecycle` | `Stable` / `Experimental` |
| `atomicity` | `Tx` (handler runs inside the bus's transaction), `External` (against a system the bus doesn't own — a VCS, a process, a work list), or `Dispatch` — a composite, which its calls decide ("Composition") |
| `access` | `oxplow_domain::Access`: `View`, `Read`, `Record` or `Write` (the default); a scope operation's command takes its scope's. `View` (changes only what the person sees) and `Read` are unrecorded (`Access::reads_only`): no audit row or `command.executed`, so a polling agent doesn't fill the log, and a closed thread can still run one (`oxplow.config.list_keys`, `oxplow.config.get`). `Record` and `Write` are audited (`Access::records`). A `Record` changes oxplow's own records (`work_item.*`, `effort.*`) and is open to any thread; a `Write` is refused to an agent whose thread is closed or missing (`Access::needs_thread`) |
| `unrecorded` | top-level input fields its audit row leaves out, kept as their size (`{ "omitted_bytes": n }`; an op's `with_unrecorded` — `oxplow.file.save`'s `content` — or an extension command's `unrecorded: [field…]`, each a property of its input or `*`, checked at load): still audited, the log not grown by every file saved. A lens form's `secret` field must be one of them |
| `needs` | the **scopes** its handler calls (`sql.read`; below — always available, enforced per call) and the capabilities, or their features (`work_items.comments`), it needs active — the grammar lenses use. Unmet, it isn't offered and doesn't run (pipeline step 0). `oxplow.work_item.create` / `update` / `transition` need `work_items`; `link`, `comment` and `delete` need their feature |

`Actor` is who runs it: `Human`, `Agent { thread_id, stream_id }`,
`Lens { lens_id, on_behalf_of }`, `System`. Its `source()` (`human`,
`agent:thr3`, `lens:acme/x` — an event source, not a URI) is what the event and audit record.

`Actor::Job` is host-derived from a validated job/attempt/fence identity
(source `job:<id>:<attempt>:<fence>`): what a job's model tool calls run
as ([extensions.md](./extensions.md) "Jobs"). It carries the thread and
stream the job was asked in, for the gates, and is confined to that
stream. It cannot confirm or create a proposal: a command that asks is
refused. A job runs only a command that says `job: true`, which an
extension declares on the read-only commands its jobs name as `tools`
(`commands::job_refusal`); every child a script composes is re-checked,
and none of core's takes a job. A lens wrapping a Job keeps Job admission and its confinement. A
`Write` command from a gated actor without a thread is refused.

## Offering a command to a person

A command says how a person meets it with an optional `ui`
(`oxplow_domain::CommandUi`; agents ignore it; a manifest command's `ui:` — oxplow's own in `extensions/oxplow-foundation`):

| Field | Meaning |
|---|---|
| `label` | what a person reads (`Pull Changes`, `New Task…`) |
| `group` | what search lists it under (`Git`, `Tasks`) |
| `keywords` | more words search matches it by |
| `about` | the resource it acts on (a resource name, `oxplow/work_item`, `acme/pr`): offered on a URI of it, its page and rows; absent, it needs no URI and **search** offers it |
| `input` | the input it runs with; a string exactly `{{stream}}` / `{{thread}}` (bound to the stream's / thread's bare id, `str1`), `{{uri}}` (the page's URI) or `{{uri.id}}` (its key; only with an `about`) binds from where it runs, and one with nothing to bind there makes it unavailable there. A binding is the whole string, never spliced into text (`UI_INPUT_BINDINGS`) |
| `form` | what gathers its input (so no `input`): a page's URI (`oxplow:oxplow/page?page=new-task`) or one of the window's own forms (`WINDOW_FORMS`: `new-thread`, the Navigator's inline form; `commit`, the Files page's commit dialog) — choosing it opens that instead of running it; offered about a URI (`about`), a page form opens with `uri=<uri>` (`tabs/pageRefs.withPageParams`) so a `pages:` lens form can prefill `defaults: { uri: "{{param.uri}}" }`; a window form takes none |
| `open_after` | a URI to open once it ran, taking only `{{result.<field>}}` from its result (`oxplow:oxplow/page?page=custom-dashboard&id={{result.id}}`, `oxplow:oxplow/agent_session?id={{result.id}}`); a whole `{{result.<field>}}` is allowed when the result field is itself the URI (`{{result.page}}`); bound by the server (RPC `command_open_after`, `CommandUi::open_after_for`) — a field the result lacks opens nothing and says so |
| `background` | it runs as a background task (kind `vcs` for `oxplow.vcs.*`, else `command`), its failure an op error |
| `shortcut` | the key that runs it, `Ctrl/Cmd+S` / `Ctrl/Cmd+Shift+N` (`keybindings.ts` `offerForShortcut`). Several commands may share a key, told apart by `when` (VS Code's way): the key runs the one whose `when` holds now, oxplow's own first. Two on one key under the same `when` always collide, so the second is refused where it registers (`Invalid` at `/ui/shortcut`) |
| `while_typing` | its shortcut runs while the person types in a field too (Save, Find, Quick Open); otherwise typing keeps it (New Task) |
| `menu` | its place in the menu bar: `{ bar: file \| edit, group?, order }` (`MENU_BARS`, `menuBar.ts`) — groups (`1_project`, `2_save`) sorted by name, a separator between them, `order` within one |
| `when` | when it's offered — in search, the menu bar (greyed when false) and by its shortcut — in VS Code's when-clause syntax over the window's context keys (below, "When a command is offered"); absent, always |

**A `ui` is checked where it registers** (`CommandUi::problem`, from
`Command::new` for every command and from an extension's loader, so a
manifest's mistake is its `file:line` error): the label, `about`'s
syntax, each `input` binding (whole, known, `uri` only with `about`),
`form` (a URI or a window form, and no `input` beside it), `open_after`
(a URI, only `{{result.<field>}}`) and `menu.bar` — `Invalid` at
`/ui/<field>`, never a menu entry that silently doesn't work. Every
template — a lens action's `{{param.x}}` / `{{row.x}}`, `ui.input`,
`open_after` — is read by one tokenizer, `oxplow_domain::template`
(`placeholders`, `whole_placeholder`, `splice`); the window binds
`ui.input` by exact match on the four names the check let through, and
parses nothing.

RPC `list_person_commands` lists what a person is offered: what they may
run now (`needs` active) that has a `ui`. The desktop keeps one
listing (`personCommandsStore.ts`, reloaded when extensions or config
change) and turns it into **offers** with `commandOffers.ts` (pure:
binding, `when`, run / form / background, and the page the server binds
for open-after).
Everything a person reaches a command by comes from the offers:

- **search** lists the ref-less ones that can run now;
- **the menu bar** (`menuBar.ts`: the OS's on macOS, the in-window
  `Menubar` elsewhere) is the offers with a `ui.menu` place, in order —
  the project commands among them — beside only the native Edit roles
  (Undo … Select All, done by the OS's responder chain: WKWebView only
  delivers ⌘C/⌘V with them in the menu; File ▸ Open Recent's submenu is
  built from the recents list and runs `projects.write` `open`);
- **shortcuts** are the offers' `ui.shortcut`s.

**When a command is offered** (`ui.when`; `oxplow_domain::when`, the
window's `when.ts`). VS Code's when-clause syntax: `!`, `&&` (binding
tighter than `||`), `||`, parentheses; `==` / `!=` against `true`,
`false` or a string (quoted, or a bare word — `pageKind == git-dashboard`);
`=~` against a regular expression literal (`/^work/i`, flags `i` `m`
`s`). A key alone holds when it is `true` or a non-empty string; a key
the window doesn't hold is false, equal to nothing (but `false`) and
unmatched. `true` and `false` alone are the literals: `when: "false"`
is a command never offered on its own (search, the menu bar, a shortcut),
declared with a `ui` for its label and `open_after` because lens actions
run it (`oxplow.file.create`). VS Code's `<` `<=` `>` `>=` and `in` / `not in` parse but are
refused as not supported yet: no context key holds a number or a list.
The window publishes its **context keys** (`CONTEXT_KEYS`, the one list):
`enabledAgents` (the enabled agent harnesses' keys, comma-joined in
priority order; a command for one matches `enabledAgents =~
/(^|,)claude(,|$)/`), `fileDirty`, `fileShown`, `pageKind` (the kind of the page shown),
`shellAvailable` (the app, not a browser window), `streamKind`
(`primary` / `worktree`), `streamShown`, `threadShown`, `vcsEnabled`.
A command's `when` is checked where it registers — it parses, names only
these keys, and compares each to a value of its kind (`true`/`false` for
a boolean key; a string or a regular expression for a string key) — so a
typo is a refusal (`Invalid` at `/ui/when`), never a silently hidden
command; the window only evaluates. Both read the same cases
(`crates/oxplow-domain/fixtures/when_cases.json`). A new key goes in
`CONTEXT_KEYS` and the window's `whenContext` (`App.tsx`) together.

Save, Find, Quick Open and New Lens with Your Agent are the window's own
commands (below, "Where a command runs"); New Thread and Commit are
`oxplow.thread.create` / `oxplow.vcs.commit` with a window `form`. There
is no other command list in the app.

## Scopes

Command composers reject every `tree.read` declaration, including qualified
file/directory bounds. They execute inside a database transaction; captured
tree reads belong to effect or collector contexts outside that transaction.

What a handler may do beyond composing other commands is a **scope**:
an OAuth-style permission named `<resource>.<action>`, declared by core
with an **access** (`oxplow_domain::scope`: `SCOPES`;
`oxplow_domain::Access`), and listed in the command's `needs`. (Not a
*capability*: that word is a swappable slot — `work_items`,
`snapshots` — a project picks an implementation of.)
A scope also says whether it **spends** the person's money (`Scope::spends`,
`v_scope.spends`): a model call on their provider key. Access stays about what
a run changes in oxplow's records, so `ai.compute` is `Read`; a tool over a
spending scope (the command needs it, or is its operation) just isn't hinted
read-only (`agent_tools::spends`), so the client asks first.
Every command — core's and any extension's — reaches them the same way,
so oxplow's own commands can do nothing an extension's can't.

Composed children capture their own RunContext during routing, alongside the
retained command handler. A registered contribution factory supplies that child's
identity, package version, generation and declared authority; a command without
one inherits the caller in a new child context. Nested composers run pure under
their own contribution, transactional leaves use transactional mode, and external
steps use their own context for provider invocation. Cancellation and host live
guards remain inherited. Check authority before composition and again before
each child executes; a later registry replacement cannot relabel a retained
handler. Native scope operations also include their implicit backing scope in
the captured declaration. This prevents namespace services from mistaking the
parent composer's extension for the child command's owner. Parent audit and event
atomicity are unchanged; this does not implement generation-wide reader pinning.

The native `state.read/snapshot` operation reads its owning contribution's private
namespace from the command's connection. Its scope/area input contains no owner
selector and its read access leaves no payload in command audit or SQL models.
`StateHost` also supplies a trusted write helper for an already-audited TxCtx,
with live checks and CAS savepoint rollback. There is no public state write
operation yet: private input/result/proposal handling must precede that surface.
See [extensions.md](./extensions.md) "Private state host boundary".

| Access | Does | Audited | An agent's run |
|---|---|---|---|
| `view` | changes only what the person sees (open a page) | no | any thread |
| `read` | reads (`sql.read`) | no | any thread |
| `record` | changes oxplow's own records | yes | any thread |
| `write` | changes files, git, processes, the configuration | yes | needs the actor's thread, and that thread open |

**The agent's reads are read scopes.** Every read an agent has beyond
`oxplow_query_sql`'s models is an operation of a read scope (`Access::Read`,
`Host::Daemon`, an `External` handler over live state), in
`crates/oxplow-app/src/commands/agent_reads/`, declared in
`oxplow-foundation` as `oxplow.<area>.<verb>` with an `example:`: `records.read/read` (`oxplow.records.read`, a
URI's content), `sql.read/query` (`oxplow.sql.query`), `search.read/search`
(`oxplow.search.find`), `pages.read/open` (`oxplow.page.open`),
`agent_text.read/skill` (`oxplow.skill.read`), `lenses.read/run`
(`oxplow.lens.run`: the lens's text, or the `LensRun` with `format: json`),
`vcs.read/{diff,blame,log,status,branches}` (`oxplow.vcs.log` pages history
from any revision, past what `v_commit` indexes), `lsp.read/*` (`oxplow.code.*`),
`changes.read/ensure` (`oxplow.change.ensure`: it writes only the derived
change cache, so it stays a read), `knowledge.read/{comments,drift}`,
`extensions.check/{validate,preview,review}` (a review clones the
candidate into `.oxplow/tmp`; it changes no record),
`collectors.preview/preview`, and `ai.compute/{decide,summarize}` (recorded
calls whose caller is the asking thread). `lenses.write/run_action`
(`oxplow.lens.run_action`) is a `record` that composes, running as the
lens's actor. A read runs unrecorded, for any thread, and the operation
reads the stream it names, else the actor's own
(`agent_reads::stream_or_actors`). The `v_scope` live model lists every
scope (id, access, host, summary). The MCP tools that carried these
reads are thin calls to the commands (`OxplowMcp::read_command`).

A command's access is the strongest among the scopes it needs
(`strongest_access`): a `view` or a `read` runs unrecorded, a `record` or
a `write` is audited. The access says what a run changes, not where its
handler runs: that is the operation's handler — in the bus's transaction (`Tx`) or against a system
it doesn't own (`External`, pinned by `the_external_commands_are_the_reviewed_ones`).
They don't follow each other: `config.write`'s `set` is `Tx` (the record
commits with the run; the file is written after it commits), while
`providers.sync` and `test_runs.write` are `record` and `External`
(each commits in its own service's transaction).
Two more scopes are services a script uses through builtins rather than
`scope(…)` — `tree.read` and `ai.compute` ([extensions.md](./extensions.md)
"Services"). A command's script may read SQL or its owning extension's private state: it
composes, so no model calls, and it decides what runs rather than doing it, so no
tree (`scope_calls::check_command_need`, at load; an effect or a collector reads it). The callable services are
**`sql.read`** and **`state.read`**; state takes `{scope, area}` and returns
null or `{stamp, values}` on the routing connection, with identity and live
authority checked by StateHost. It never enters the semantic layer. SQL takes `{ sql, params? }`, one read-only
statement over the published models (`v_*`), its `:name`s bound from
`params`, at most `SQL_READ_ROW_CAP` (1000) rows, answered as a list of
row objects, read through the SQL gateway (`SqlGateway::run_in`, on
the run's activation pin) by every script host alike — a command's
composer, an effect's or policy's script, a dry run and a provider's
`host/call` — so `metric_grid()` and `metric_findings()` answer there as
in a lens. Each read is its own snapshot; a script's reads don't share
one. More than 1000 result
rows fails with an error; it never hands a script a silently truncated
list. Exactly 1000 rows is allowed. Aggregate in SQL to calculate across
all matching source rows, or explicitly narrow the query with a filter
or `LIMIT` when a subset is intended. A script that wants a large result
a slice at a time asks for it: `page: { limit, offset }` (limit at most
1000) answers `{ rows, more, next_offset?, warning? }`, the `warning`
set when the SQL has no `ORDER BY` (semantic-layer.md "Completeness and
coverage"). The same `scope_calls::Calls`
guard serves commands, effects, effort policies, dry runs and provider
`host/call`. Fixture `answers` stand in for the scope's answer: a row
list, or the page object when the call asks for a `page`.

Completeness here is **of the query's result**, not collection coverage
or freshness. An explicit `LIMIT` is part of the authored query; the
scope cannot infer whether that subset answers the author's question.
The row-only scope does not expose the gateway's model-change timestamps
or prove that every source has been collected.

A Starlark handler calls one with `scope(id, args)`
(`oxplow_script::scope`). The script runs in a script worker process
(`extension_commands::run_command_script` → `oxplow_script::workers`,
[collection.md](./collection.md) "Containing scripts"); the call crosses
the worker's pipe to the thread that runs the handler (which owns the
transaction) and is answered there — that time is left out of the
script's budget, not its ceiling. A script past its budget has its
worker killed and the transaction released. `crate::scope_calls::Calls` serves it: a
scope that isn't in the command's `needs` is **refused** (the
script fails with the reason), one that is is counted and answered; a
busy database retries the whole run. A dry run (an extension's
`examples`) answers from the example's `answers` (per scope, in
call order) or through the SQL gateway.

**Where a command runs** (VS Code's model: commands run where their
handler lives). A scope says its host (`Host`): the **daemon**
(oxplow's records, the repository, the project's files), the project's
**window** (`tabs.write`: open, close, focus a tab; `editor.write`:
save; `window.show`: find, quick open, and `navigate` — `oxplow.window.navigate { uri, stream? }`, open to an agent, takes the person to a URI: the window opens and focuses its page in the thread it belongs to (`apps/desktop/src/tabs/navigation.ts` plans it: a file's editor, a work item, effort, lens, wiki page, directory or a `page` URI), switches to `stream` first when it is named, and a bare stream or thread URI selects it; it returns `{ opened, stream }`; `agent_input.write`: draft, a
person's) or the **app shell** (`projects.write`: create, open — New /
Open Project; the window reaches the shell, so the window hosts its
handlers too). A shell command isn't recorded: there's no app-scope bus
or log yet — one comes when something there is worth recording or an
agent needs it. A command
backed by a window scope is `View` (not recorded, no
transaction) and its spec carries the operation (`CommandSpec::op`):

- **The window's own runs stay in it** — `clientHost.ts` `runLocally`:
  the window does it with its own handler, nothing sent to the daemon.
- **A run on the daemon** (an agent's through MCP, a script's) reaches
  the window through `client_host::ClientHost`: an
  `OxplowEvent::ClientCall { id, client, thread_id, actor, scope, op, input }`
  addressed to one window — the last registered that hosts the
  scope — which does it and answers `answer_client_call { client,
  id, result | error }` (another window's answer is ignored); the run
  waits (15 s), its answer the run's result. A window mints an id when it
  starts and says it's open — and what it hosts — with
  `register_client_host { client, scopes }` then and on reconnect,
  and `unregister_client_host` when it closes (`pagehide`), which refuses
  its unanswered calls at once. With no window, one that doesn't host it,
  closes or doesn't answer, the run is `Unavailable`; a window that
  didn't answer is forgotten until it registers again, so the next call
  is refused at once rather than waiting out another timeout.
- **What the window runs to answer a call runs as the caller.** A
  window handler that writes through the daemon — Save writing
  `oxplow.file.save` — calls `run_command_for_call { client, call, id,
  input }`, which runs as the waiting call's actor (`ClientHost::caller`;
  only the window the call went to, only while it waits), never as the
  person at the window: an agent's Save is the agent's write, under its
  write gate and in its own stream (`file.save` refuses anyone but a
  person outside their own stream), audited to it.
- **An agent acts only in its own thread**: the call carries the agent's
  thread and the window acts in that thread's tabs — opening a file there
  reads it into its stream's session — never switching the thread or
  stream the person sees (an agent with no thread is refused). A person's
  call acts in the thread shown. Every call carries its actor.
- One handler per command; something that should happen when one runs
  reacts to an event, not a second handler.

**Operations** (`commands/ops.rs`). A record or write scope's
native behavior comes as **operations**: `bookmarks.write` has `set` and
`remove`. Each `Op` carries what only Rust can supply — the input schema
of the type its handler reads, the handler (`Tx` / `External` /
`Dispatch`), whether it returns an inverse, a per-input confirmation, a
check before the transaction — and is added to the bus (`add_op`). A
manifest command backed by one (`scope:` + `op:`) gets the
operation's schema, undo and atomicity, its access from the scope's,
and
the scope in its `needs`; the manifest says the rest. oxplow's own
are declared in **`oxplow-foundation`** — a shipped extension that is
**required** (`BundledExtension::required`: it can't be disabled). Its
commands are staged and published with each generation like any
extension's, from its active deployment (the shipped one, or one deployed
while developing oxplow), so deploying it with a new command registers
that command at once; nothing runs a command before the first activation
publishes. Each runs as its extension's contribution (an
extension-scoped operation — configuration, state, jobs — finds its owner
that way), so foundation's command runs are `oxplow-foundation`'s
`v_extension_run` rows like any extension's. An extension may declare a command over the same operation in
its own namespace. A script reaches record and write operations only by
composing commands.

**An operation bounds only confirmation.** Anyone may run any
command: a person, an agent, or a lens acting for either. There is no
invoker list, and the manifest has no `invokers:` key (the loader refuses
it). The operation bounds only how much a person is asked: an `Op` says
the least it is confirmed (`confirm_at_least`: `Destructive` for a
delete, a discard, a merge; `Always` for an install), and `Op::command`
refuses a declaration that asks less (`Invalid` at `/confirm`), from
`oxplow-foundation` and from any other extension alike. A declaration may
ask more. A command's `job: true` lets a durable job's model call it; it
is allowed only on a command with its extension's own handler (`entry:`),
never on one over a scope's operation.

**The per-run trace.** Every run carries a `RunContext`
([extensions.md](./extensions.md) "The run context") whose trace counts
its scope calls. A `Tx` run's attempt keeps its own (`TxCtx::trace`,
fresh per attempt, shared by the runs nested in it). The routing pass
composes a composite once, through the gateway, as a pure child of the run,
and what that child called becomes the plan's scopes, which the run
records; steps add each step's. An `External` call's is its
`Invocation::run`, a child of the command's run: a provider's
`host/call`s during it count there — its own command's (`run_op`) and a
capability verb's, which the `work_item.<verb>` run hands its verbs in
`VerbCall::run` ([providers.md](./providers.md) "The protocol"). Its
summary — per scope, how many calls (`{"sql.read": 2}`) — is written on
the run's audit row (`command_audit.scopes_json`, V27, named so in V34;
NULL when it used none). Later: each call's detail (what it touched),
sampled or on by setting, and approvals by scope.

## The pipeline (`crates/oxplow-app/src/commands/mod.rs`)

The bus is split by concern: `mod.rs` the types, the registry and the
pipeline (`run_prepared`, the read path); `record.rs` a
run's audit row and events (`record_tx`, `audit_only`,
`record_external`, the transaction's `Abort`); `undo.rs` undo and the
claims that keep two undos or approvals from both applying;
`proposals.rs` proposals (`unconfirmed`, the dry run, approve,
decline); `steps.rs` composition — the composed tree, routing, steps;
`ops.rs` the scopes' operations; `util.rs` what handlers
share; `tests.rs` the bus's own tests.

`CommandBus::run(actor, name, input, confirmed)`:

First the **actor is resolved**, once (`CommandBus::resolved`): an agent
— or a lens acting for one — whose transport carried its thread but not
its stream runs with its thread's stream, so "the caller's stream" is
`actor.stream_id()` in every handler, one answer; an agent claiming a
thread that doesn't exist is `Denied` (audited). An agent with no thread
has no stream, and a handler's own-stream check refuses it rather than
skipping.

0. **Offered** — what's active now (`with_capabilities`: the capability
   registry and the config, `capabilities::Active::refusal`) must meet the
   spec's `needs`, and a command an implementation owns runs only while
   that one is active: a provider instance's namespace (`<instance>.*`).
   Refused
   → `Invalid` naming what it needs (audited), before validation, for
   every actor; `list` leaves it out the same way.
1. **Validate** the input against the schema → `Invalid { field, message }`
   (audited as `invalid`).
2. **A job's tools** — a job (or a lens it pressed) runs only a command
   that says `job: true` → `Denied` (audited; `job_refusal`). Anyone else
   may run every command.
3. **Agent policy** — for a thread-gated run (an agent, an effect, a
   job, or a lens acting for one, nested runs included),
   `AgentPolicy::check_command(thread, spec, may_write)`
   (`agent_policy.rs`) → `Denied` (audited). A `Write`
   command needs an open thread: the bus asks its write gate
   (`Services` wires `Thread::is_open()`), so a closed thread can read
   but not change state. A `Record` command skips that refusal: oxplow's own records
   (tasks, efforts) aren't a claim on the worktree.
   **Pre-check** (tsk1010): a command may carry a
   `Precheck` (`Command::with_precheck`) — an async check of the input
   that needs more than a transaction can do (resolving a metric query
   through the metric engine). It runs here, once the actor is admitted
   and before any transaction: the command's own for a direct call; for a
   composite, every composed call's, collected while routing
   (`steps.rs` `Router`), its refusal placed at `/calls/<i>/input…`. A
   refusal is audited `invalid` and nothing runs.
4. **Confirmation** — what a run asks is `commands::asks`: the command's
   `confirm` (or its per-input `confirm_for`), except an agent-driven run
   (an agent, or a lens acting for one), which asks nothing — what an
   agent may call is its harness's to decide. When it asks and the call
   isn't `confirmed`, **nothing runs and no audit row is written**: a
   person (or the system) gets `NeedsConfirmation { preview }`; an
   effect's run is kept as a **proposal** and gets `Proposed { proposal,
   preview }` (see "Proposals", P6b.A3). `confirmed` stays a person's
   word (`Actor::may_confirm`): an agent's run is never confirmed, so a
   handler's own consent check still asks for a person. The answer
   rides into the handler as `TxCtx::confirmed` (and step 3's gate as
   `TxCtx::may_write`, "the thread is open"): a handler that learns only while running that a
   confirmation is needed — a composite whose child asks — raises
   `NeedsConfirmation` itself, and the bus treats it as this step would
   (rolled back, nothing audited; a person asked, an agent's run
   proposed) (P6b.A1). An `External` handler learns it the same way:
   `Invocation::confirmed` is the person's confirmation, and a handler
   that finds it needs one (a deploy that adds permissions) raises
   `NeedsConfirmation` while it is false; the proposal's approval runs it
   again confirmed. `Preview::details` carries what the person is asked
   about beyond the command, one line each (the permissions a deploy
   adds): over IPC as `IpcError::details` (`IpcCallError.details`), shown
   in `CommandConfirm` and the proposal's card, and in the error's text an
   agent reads. Only a plain call is proposed: an undo that needs a
   person and isn't theirs is `Denied` (a proposal is a plain call and
   would lose the row it undoes); an agent's undo asks nothing, as its
   runs don't.
5. **Run and record in one transaction**: the handler, a `command_audit`
   row (`crates/oxplow-db/src/command_audit_store.rs`: actor — an
   agent's session too, `session_id`, V40 — input,
   outcome, the handler's `result` — V114, so a run's answer, such as a
   merge's conflicts, stays readable after the fact — inverse, and the
   scopes it called, "Scopes"), `command.executed@4` in the event log
   pointing at the audit row, the handler's domain events (with `cause` =
   the executed event, and the actor's thread/stream/agent session —
   `Actor::anchors()` —
   filled into any anchor the handler left empty), and the audit row's
   `event_id`. A handler failure
   rolls all of it back and is audited as `error` in a transaction of its
   own. A handler that found **nothing to change** and wrote nothing
   returns `HandlerOutput::unchanged` (tsk861): a plain call then leaves
   no record — no audit row, no `command.executed`, `audit_id: None` —
   like a read (`oxplow.knowledge.relocate_comment` re-run on a settled
   document is the case). The bus enforces it (tsk901): an unchanged call's
   transaction is **rolled back**, so nothing it wrote lands unaudited,
   and one that also returns events or an inverse is refused (audited as
   `error`). An undo, an approval or an effect's reaction is recorded
   regardless: its row must be marked (`an_undo_or_approval_that_changes_
   nothing_is_still_recorded`). Only `Tx` handlers may say it; an
   `External` run is always recorded.
6. Post-commit: wake the event pump.

**Retention.** A refused run's audit row (`invalid`, `denied` — input
that didn't fit, a caller turned away) wrote nothing and nothing points
at it, so the daily retention sweep deletes those older than
`event_retention::REFUSED_AUDIT_DAYS` (30) through the partial index
`command_audit_refused` (V33); a confused or polling agent makes many.
Runs that did something (`ok`, `error`) stay — undo, approvals, effect
runs and `command.executed` name them.

**Reading it.** `v_command_audit` (command, actor kind and id, outcome,
error, time, thread, session) is the agent-facing read of the audit. The
bundled Agent Interface page (Activity) counts the agents' runs by
command with the share invalid, denied or failed, and opens a command for
its most common errors, so a command an agent keeps getting wrong shows.
The refused rows' 30-day retention is why that page looks back 30 days.

A command that only reads (`View` / `Read`) stops after step 4: its handler runs in
`Database::read` — a snapshot that is always rolled back, so a write it
makes never lands (nothing would audit it; tsk513) — and the outcome has
`audit_id: None`, `event_id: None`.

`CommandBus::undo(actor, audit_id, confirmed)` loads the row, refuses a
run that didn't complete, was already undone or has no inverse, runs the
inverse through the same pipeline, and marks the row `undone_by` the new
run. The original row is claimed **with** the inverse run
— marked `undone_by` inside the run's transaction for a `Tx` inverse,
or claimed (`undone_by = 0`, pending) before an `External` inverse and
released if it fails — so two concurrent undos can't both apply it
(tsk437 review). The one that loses answers `Invalid` ("audit row N was
already undone", `lost_race`) and leaves **no audit row**: it didn't
fail, it was beaten to it. An approval's lost race is the same.

**Agent rules follow the agent.** An `Actor::Lens { on_behalf_of }` —
or an `Actor::Component { component, on_behalf_of }`, a custom component
acting for whoever uses it ("Custom components") — whose chain ends at an
agent (`Actor::is_agent_driven`) gets the agent policy and can never
confirm, exactly like the agent: its run asks nothing. **A lens grants
no power:** it acts with the rights of whoever pressed it (`Actor::presser`).
Its run of a command that asks is asked first when a person pressed it,
and runs without asking when an agent pressed it.

**Only a person confirms; an effect runs with an agent's rights**
(P8.D8). `Actor::may_confirm()` is true for `Human` and a lens acting for
one — never an agent, an effect or `System` (step 4 drops a `confirmed`
from anyone else). An agent's run asks nothing. An
`Actor::Effect { effect: "<extension>/<id>", thread_id, stream_id }`'s
run that asks becomes a proposal; `System` is asked (`NeedsConfirmation`).
An effect is held to an agent's gates, and it carries the
triggering event's thread and stream (`CommandBus::run_effect` takes the
event's anchors), so an agent's gates apply to it there
(`Actor::gated_thread`): the open-thread gate on that thread (a `Write`
command reacting to a closed thread's event is refused; an effect
whose event carries no thread — a project-level fact such as a
contribution's health — acts for no thread and writes into no worktree, so
the gate doesn't apply to it: `Actor::is_anchorless_effect`, step 3's
"requires the actor's thread" refusal skips it; its approval and each
command's confirmation still hold), and VCS
commands on that stream only (`Actor::confined_to_own_stream`; an event
with no stream reaches no worktree). Its source is
`effect:<extension>/<id>`, and only a
`Human` approves or declines a proposal. The audit and events say so:
`actor_kind = 'effect'`, carried by `command.executed@4` (whose
`ActorKind` also has `job`) and `command.proposed@3` (whose `ActorKindV2`
has no `job`: a job can't leave a proposal). The log holds only these
versions (V70 rewrote older rows). An effect's reaction runs
through `CommandBus::run_effect` (P8.D10) with `RunOrigin::Effect`, the
fourth origin beside a call, an undo and an approval: like those, its
record (`effect_run` + `effect.result@5`) lands in the run's transaction,
or is claimed before a step outside it (`claim`), and its
`command.executed` is caused by the triggering event (extensions.md
"Effects").

**An `External` run whose recording fails** (its effects already
committed in the service's own transaction) is reported as done and
unrecorded — `audit_id: None`, logged at error level — never as an
error, which would claim the change didn't happen.

A command whose confirmation depends on the input sets
`Command::with_confirm_for(fn(&input) -> Confirm)`; the bus consults it
in step 4. A handler that needs the UI to refetch returns
`HandlerOutput::after_commit`, which the bus runs once the transaction
has committed — the in-memory `OxplowEvent` broadcast belongs there,
never inside the handler.

**A `Tx` handler must be pure** (tsk442): it can run more than once,
because `Database::transaction` retries its closure on SQLITE_BUSY. Only
database writes on the given connection belong inside; anything outside
the database (a file, in-memory state) goes in `after_commit`, which must
tolerate failure since the run is already recorded. `oxplow.config.set` is the
model: the handler validates and computes before/after; `after_commit`
re-applies the key to the config as it is then, writes project.yaml and
swaps memory under one lock, so concurrent sets of different keys both
survive.

## Composition: `oxplow.command.sequence`, the composed tree and steps (P6b.A1, tsk713)

A composite is a command made of other commands' calls: a
`Handler::Compose` (`commands/compose.rs`) whose **composer** says, for
an input, which calls to run (`Composition { calls, result, events }` —
`events` are the composite's own, appended after the children's and
caused by its `command.executed`; on the steps path only when every step
landed; an extension command's declared types, P8.D4). It is the
one mechanism: `oxplow.command.sequence { calls: [{ name, input }] }` composes
its input's calls; an extension's own command (P6b.B2) composes what its
script returns over its `input` rows. Its atomicity is `Dispatch` — its
calls decide where it runs, so composites are pinned
(`CommandBus::composite_commands`, `the_composite_commands_are_the_reviewed_ones`).
Once the actor is admitted (steps 2–3), the bus **routes** it
(`CommandBus::route_composite`, `commands/steps.rs` `Router`): it
**composes once** — on a blocking thread, before any transaction, its
reads through the SQL gateway (`compose::Composing`: the gateway, the
database and the runtime to block on; no connection is held while it
reads, so the metric engine's own reads can run) — and routes each call — a `Tx`
command stays in the transaction; an `External` one (every work list's
`work_item.*` verbs, oxplow's own tasks' included) leaves it; a nested
composite is composed in turn and must route inside (its steps would
otherwise land outside the run they're a step of), and a call that only
reads (`View` / `Read`) is refused (a composite composes commands that write). Each call's needs
(step 0, refused at `/calls/<i>/name` — `commands::offered`), input (at
`/calls/<i>/input/…`, a nested call's at its full path) and precheck
are taken there, the composite's own precheck first. What it routed is
a `Composed` tree — each composite call's own composition beside it —
and **that is what runs**: the composer is never asked again, so the
calls checked are the calls run (an extension's script runs once; a
composer that would answer differently later isn't asked). Every call
inside → **one run in one transaction**, below; any outside →
**steps** (next section).

**In one transaction** (`steps::run_composed`, the `Tx` handler the bus
makes from the tree — the same calls on every attempt of the
transaction): first a pass that writes nothing over the whole tree —
each call's **own** agent policy (with the parent's
`may_write`) and `confirm`, a nested composite's calls' too — so a
composite never widens what its children allow; a call anywhere in the
tree that asks makes the parent ask (`Preview { command: <parent>,
destructive }`) unless the run was confirmed. Then every handler runs on
the parent's transaction one level deeper (`TxCtx.depth`), with its own
captured child RunContext; routing refuses a
composite more than `MAX_NESTING` (8) deep ("does a command compose
itself?") instead of recursing until the stack overflows. The parent has
the **one audit row** and `command.executed` (children are not audited
separately — a child row would be an independently undoable unit
fighting the parent's inverse); its `result` is `{ result, children: [{
name, result }] }` — no child's `input` (the caller sent it) or `inverse`
(the parent's is the undo): echoing both cost an agent filing twenty
tasks 23k characters back; the children's events ride out on the
parent's `HandlerOutput.events`, so they are caused by the parent's
`command.executed`; their `after_commit`s chain in order. The inverse is
the inverses of the children **that changed something**, reversed, as a
`oxplow.command.sequence` — or none when one of those has none (it can't
be undone in part) — so `undo` needs nothing new, and a child whose
inverse asks makes the undo ask. A child that says `unchanged` (an
`oxplow.config.set` to the value already there) has nothing to undo and
doesn't veto the rest; one that says so but returns events or an inverse
is refused; a composite whose every child changed nothing, with no events
of its own, is `unchanged` itself and leaves no record. A child's `Busy`
propagates and the bus retries the whole parent (handlers are pure).
A `Tx` handler doesn't compose by hand: composing is a composite's, so
it reads through the gateway outside the transaction.

**As steps** (P7 review, tsk713; `CommandBus::run_steps`): a composite
with a call outside the transaction can't be all-or-nothing, so its
calls run as steps. (1) **Checked first** (routed as above, then): every
step's — and every composite step's calls' — own agent
policy and `confirm`, before anything runs — a refusal is the run's, audited, and
nothing ran; a step that asks, or a composite step whose call asks, makes
the run ask once (`NeedsConfirmation` with the calls in
the preview), and an effect's run is a proposal **with no dry run**
(nothing outside the transaction runs before a person decides). (2)
**In order**, each landing as it runs: a step inside oxplow in its own
transaction (its events caused by the run's `command.executed`, whose id
is fixed first), an external one through its system; the first failure
stops the run, and the steps before it **stand**. (3) **Recorded once**:
one audit row and one `command.executed` with `result { result,
children: [the landed steps], failed?: { name, input, error } }` and the
landed steps' events, caused by it. A run that stopped partway is
recorded as an `error` with what landed, and returns `Failed` naming the
failed step and the ones that stand; a run that failed at its first
step is audited like any failure. **Not undoable** (no inverse): a
system outside the bus can't be rolled back with it, so a person
reverses a landed step by hand (each child keeps its own `inverse` in
the result).

## Proposals: a run that needs a person who isn't there (P6b.A3)

Two runs are kept for a person instead of being refused
(`CommandBus::unconfirmed`): an **effect's** run of a command that asks
(an effect is extension code reacting to an event, not a person), and a
run whose handler asks for **consent to code** — a deploy that adds
permissions or a build command — whoever ran it, an agent included: the
person approves what the code may do. An agent's own run of a command
that asks is never one: it runs.

1. **Dry run** — a `Tx` handler runs with `confirmed: true` in a write
   transaction that is always rolled back (`Database::rehearse`); its
   `result` is what it would have done (`oxplow.config.set`: `{ key, before,
   after }`; a composite: its `children`). Its events and `after_commit`
   are dropped, so nothing reaches a file. An `External` handler never
   runs: its proposal has no dry run. A dry run that fails is the run's
   failure (audited like one), not a proposal.
2. **Kept** — one transaction inserts the `command_proposal` row
   ([data-model.md](./data-model.md); `proposal_key` supersedes a pending
   proposal for the same config key or the same command and input —
   whichever thread proposed it — and `insert_tx` returns which) and
   logs `command.proposed@3 { proposal, command, actor_kind, actor_id?,
   destructive, supersedes? }` with the actor's anchors, `supersedes`
   naming the replaced proposals so the replacement is never silent.
3. **`Proposed { proposal: "proposal:N", preview, supersedes }`** — IPC
   code `PROPOSED`; MCP `run_command` turns it into a *successful*
   result `{ kind: "proposed", proposal, message }` (`proposed_message`: it waits
   for the person in this thread and in Alerts, and on the setting's
   row; "It replaces proposal:M" when it did; tell the person; don't run
   it again; its `decision` is in `v_command_proposal`). Other MCP
   tools that run a command report the same message as an error. The
   desktop shows a thread's pending proposals where the conversation is
   (P9.A3: the Answers strip, and `useThreadProposals`; usability.md
   "Agent proposals").

**Deciding** is a person's (`Actor::Human`; an agent, a lens or the
system is `Denied`): a proposal is consent to code that isn't theirs or
an effect's run, the one place a person, not their agent, answers. A
proposal is decided once (`Invalid` after):

- **`CommandBus::approve(actor, id)`** runs the proposal's command as the
  person, confirmed, through the whole pipeline (`RunOrigin::Approval`).
  A `Tx` run marks the row approved with its `audit_id` and logs
  `command.approved@2 { proposal, command, audit_id }` (caused by its
  `command.executed`) in the run's own transaction; an `External` run
  claims the row first (approved, no audit row: `claim_tx`), names its
  audit row when recorded (`finish_claim_tx`) and releases the claim if
  it fails (`release_claim_tx`) — the same pattern as an `External`
  undo's claim. A run that fails leaves the proposal pending; a
  concurrent approval that loses answers `Invalid` ("proposal:N was
  already decided") with no audit row.
- **`CommandBus::decline(actor, id)`** marks it declined and logs
  `command.declined@2`; nothing runs, no audit row.

A custom component's frame invokes a command through UI RPC
`invoke_component_command` — as the lens acting for the person, only a
command its component declares ([extensions.md](./extensions.md),
"Custom components").

UI RPC **`decide_proposal { proposal, approve }`** (`ui` in surface
parity; desktop `decideProposal`) returns the approving run's outcome, or
`null` for a decline. A proposal's dry run is a snapshot at proposal
time; the approval re-runs for real and the audit row records what
happened.

## `config.*` and the key registry

`.oxplow/project.yaml`'s vocabulary is one registry,
`oxplow_config::keys` (`config_keys()`): the JSON Schema generated from
the file's own shape (`RawConfig`, `deny_unknown_fields`), whose
top-level properties are the keys, each with its field doc and value
schema. A `oxplow.config.set` / `oxplow.config.unset` writes **only its key** into the
file (`write_project_key`, tsk1028): that key's block — its line at column
0 through the next key, under its own comments — is replaced, removed or
appended, and every other line stays byte for byte as the person wrote it
(flow style, comments, keys equal to their default such as
`metricRetentionDays: 0`). The edit is checked to change that key and nothing
else; a file whose YAML ties keys together (anchors) is refused, to be
edited by hand. The whole-file re-render it replaced lost comments and
formatting and dropped default-valued keys. Adding a field
to `RawConfig` makes it managed, documented and settable at once.

**A person's layer.** `oxplow.config.set` / `oxplow.config.unset` take `layer: project`
(the default) or `layer: personal`: `.oxplow/personal.yaml`, a person's
own layer over the project's, which git ignores (`.oxplow/.gitignore`
keeps only `project.yaml`). It holds only `oxplow_config::PERSONAL_KEYS`
(`activeProviders`, a person's choice of implementations, and
`newThreadSession`, what a new thread starts with — `ask`, `none` or an
agent — which has no project counterpart and so no `project.yaml`
schema; the layer's loader validates it); another key there is refused,
by the command and by the loader. `config.changed@2` names the `layer`; `@1`
payloads were all the project's (V14 moved them). `v_config_value`
shows the layer as its own `personal.<key>` row, origin `personal`.
`set_zones` is gone (tsk392): `zones` is just a key.

**Every key has an entry** (P6.H1): `oxplow_config::config_entries`
gives each key its value — the file's, or the default's — and whether the
file sets it; `render_project_config` is the set ones, and a test holds
the entries to exactly the schema's keys. The single-agent `agent` key is
gone: `agents` is the one key (an old file's `agent:` is refused as an
unknown field). Every entry's YAML comes from `oxplow_config::to_yaml`,
the one bridge from a config type to YAML, by way of JSON text: serde_json's
`arbitrary_precision` makes a number inside a `serde_json::Value` (an
extension instance's `config`) a private struct under `serde_yaml::to_value`,
which once wrote `{$serde_json::private::Number: '5'}` into the committed
file (P6 review, tsk599); `keys.rs`'s `yaml_to_json` is the same bridge
the other way.

**Settings is a view** (`crates/oxplow-app/src/effective_config.rs`, read
as the live model `v_config_value`; the desktop reads it through
`configValues.ts`'s `readSettings` / `settingValue`): one setting `{ key,
value, origin, extension?, doc, schema }` per project key (`project` when the file
sets it, else `default` with the default's value), per AI role
(`ai.roles.<role>`: `project` when the project overrides it, `global`
from `ai.yaml`, `default` when unbound), and per metric and
dimension from the global manifests (`global`) and enabled extensions
(`extension`). `SettingsPage` lists them grouped and searchable
(`pages/settingsModel.ts`), each with **Ask the Agent to Change This**
(a prompt naming the key, its doc and its value, inserted, never sent) —
which the agent sets with `oxplow.config.set`, unasked: **no key is a
person's alone** (`v_config_key` / `v_config_value` v2 dropped
`human_only`; V104 dropped the column). Direct controls remain for the
settings with an area of their own (agents, AI, language servers,
extensions, integrations); `set_snapshot_retention_days`
and `set_snapshot_max_file_bytes` went with their editors. The view
re-reads on `ConfigChanged`, the renderer's one signal for every config
write: a `oxplow.config.set`, and `oxplow.ai.set_role` (a role's binding is an
`ai.roles.<role>` row), which emits it too (P6 review, tsk607).

**The backend reacts on the pump, never the bus** (P7.B6,
`config_reactors.rs`): `config.changed@2` consumers, each filtered to its
keys — `config.workspace_filter` (`generated`), `config.extensions`
(`extensions`: the extension catalog's change signal),
`config.providers` (`extensionInstances`, `activeProviders`: the provider
registry reconciles) and `config.metrics` (any key: the metric catalog
reseeds). One that reads the whole config first waits for
`oxplow.config.set`'s after-commit swap (`applied`: the key no longer reads its
`before`, woken by `Services.config_applied`, at most 5 s), since the
pump can see the commit before the swap.

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent, the launcher for the human.

## `Tx` and `External`

A `Tx` handler is `Fn(&TxCtx, Value) -> HandlerOutput` and composes
into the bus's transaction, so its writes, the audit and the events
commit or roll back together — the target shape. `TxCtx { conn, actor,
events }` carries an `EventCtx` whose `source` is the actor's and whose
`cause` is the run's `command.executed` id — fixed before the handler
runs — so store cores the handler calls (`set_status_tx`,
`effort_store::start_tx`) log their own events as this run's. Those
events precede `command.executed` in `seq` (it's appended after the
handler, with the audit); `HandlerOutput.events` follow it. A handler
must stay pure: `Database::transaction` retries it on SQLITE_BUSY — and
a handler that hits one itself returns `CommandError::Busy` (the
`From<DomainError::Busy>`; map SQLite errors with `oxplow_db::map_sql_err`),
which the bus turns back into a retry; only a busy that outlasts the
retries reaches the caller, as `Busy` (RPC `BUSY`). A system outside
oxplow that couldn't answer just now (an external provider's `Internal`, a timeout,
a rate limit, with its wait) is `Unavailable { message, retry_after_ms }`
(RPC `UNAVAILABLE`) — the one failure an effect sends again by itself
(tsk914); everything else that fails is `Failed`. An **`External`**
handler (P5.A1) is an async call against a system the bus doesn't own —
a VCS, an external provider's process, a collector script, the lens files on disk —
whose state can't join the bus's transaction; the bus audits it after it returns (a failure to
record is logged, never reported as the run failing); its audit row
holds the handler's `result` like a `Tx` run's. It is
`Fn(Invocation, Value)`: `Invocation { run, idempotency_key }` (its
actor is `run.actor()`; the run is a child of the command's), the key
set for a step of an effect's reaction (`effect_step_key`:
`effect:<effect>:<event id>:<index>:<hash of the call>`, the same on
every attempt at it — P10) and `None` otherwise; an external provider's
handlers pass it to the provider process ([providers.md](./providers.md)
"Idempotency"). A keyed step's events take dedupe keys from it
(`<key>:event:<i>`, `same_events_once`) and `record_tx` appends handler
events uniquely, so a step that landed and is re-sent on a retry — its
system answering again, events included — logs them once (tsk912). It is the right
kind for exactly those commands, not a shortcut: `CommandBus::
external_commands()` is pinned by `the_external_commands_are_the_reviewed_ones`,
and adding one means naming its system in the summary. The
`work_item.*` verbs are `External` for every list, oxplow's own tasks
included: each runs on the item's list through its `WorkItemVerbs`
([work-items.md](./work-items.md)). Registering a handler whose kind
disagrees with the spec's `atomicity` is refused, as is a second command
of the same name.

**A run's projections are delivered before it returns.** Once a run
lands (one transaction, an outside call recorded, or a composite's
steps), `CommandBus::landed` runs the event pump's sync consumers — the
projections: a work list's records into `work_item`, the page refs —
before the caller gets the outcome (`EventPump::deliver_projections`),
then wakes the pump's loops. So whoever ran a write reads it back at
once, whichever list holds it.

## Commands so far

Each `oxplow.*` row below is declared in `extensions/oxplow-foundation`
(or, where it says so, `oxplow-bundled`) over an operation of the scope
its area falls under (`threads.write`, `vcs.write`, …; `SCOPES`); "Handler" describes that operation.

| Command | Handler | Notes |
|---|---|---|
| `<extension namespace>.<name>` (an enabled extension's `commands:`) | `Dispatch`: the extension's Starlark script composes core commands — once, when the bus routes the run — run as any composite's (`extension_commands.rs`, P6b.B2; "Composition") | declared confirm / access, undoable, `Experimental`; registered while the extension is enabled (primary worktree). See [extensions.md](./extensions.md) → "Commands" |
| `oxplow.review.accept`, `oxplow.review.request_changes` (oxplow-bundled, P7.C5) | extension composites of `oxplow.work_item.comment` + `oxplow.work_item.transition` (to `done` / `todo`) on an effort's work item — as steps, every list's verbs running outside the transaction (not undoable); an effort without a work item is refused by name; accept refuses unreviewed claims or inferred decisions unless `force` | `confirm: always` (a person's run asks; an agent's runs without asking). See [extensions.md](./extensions.md) → "oxplow-bundled" |
| `oxplow.command.sequence { calls: [{ name, input }] }` | `Dispatch` — a composite (`commands/compose.rs`, P6b.A1) | `Write`, `Confirm::Never` — the children decide. Each child's own policy and confirmation; one audit row for the parent with the children in `result`. Every call in oxplow's records → one transaction (`steps::run_composed`), undoable as the reversed children that changed something; a call outside it (a `work_item.*` verb, any list's) → steps, in order, not undoable. The one composition mechanism (an extension's command runs on it). See "Composition" |
| `oxplow.work_item.transition { uri, to, native_state? }` | `External` (`commands/work_item.rs`): the item's list's `transition` verb (oxplow's tasks: `oxplow_tasks::verbs::transition_tx`, in a transaction of their own) | `Record`; undoable (the inverse restores the prior canonical and native state; an external inverse is renamed to `oxplow.work_item.transition` so undo dispatches again). `to` is a canonical state, `native_state` the provider's own and must map to it (oxplow: its status; `archived` with `done` or `canceled`). For every provider core logs `work_item.state_changed` when the state moved, with the audit, caused by `command.executed` (the interface's events: [work-items.md](./work-items.md)), which the effort policy reacts to ([work-tracking.md](./work-tracking.md)). |
| `oxplow.work_item.create { title, body?, parent_uri?, state?, native_state?, native?, thread? }` | `External`: the active list's `create` (oxplow's tasks: `oxplow_tasks::verbs::create_tx`) | not undoable (that would be deleting an item). Always files on the active tracker, which must be running. `thread`: absent, an agent's own; a person's without one, the backlog. oxplow: `native { priority? }`; the row at the end of its list (`next_sort_index_tx`), core's `work_item.created@2 { work_item, state }` and `work_item.state_changed`, caused by the run; an agent's task is authored `agent`. The result has the item's `uri` and `link_warnings`: the body's `[[…]]` links that don't resolve, checked like a note's, for every list (`link_check::LinkDeps::item_warnings`, on a read of its own). |
| `oxplow.work_item.update { uri, title?, body?, parent_uri?, state?, native_state?, native? }` | `External`: the item's list's `update` (oxplow's tasks: `oxplow_tasks::verbs::update_tx`) | undoable (the inverse restores exactly the fields and state given). oxplow: fields and status commit together — core logs `work_item.edited@2 { work_item, fields }` for the fields the input set, then the status move with everything `oxplow.work_item.transition` implies; `native { priority? }` (a thread change is `oxplow.work_item.move`). A refused run writes nothing. An update that sets a body carries its `link_warnings`, as `oxplow.work_item.create`'s does. |
| `oxplow.work_item.delete { uri }` | `External`: the item's list's `delete` (oxplow's tasks: `oxplow_tasks::verbs::delete_tx`) | `Destructive` (asks first); not undoable; only on a provider declaring `delete`. oxplow: marks the task deleted; its record says so, which clears its row and its page refs; core logs `work_item.deleted@2`, caused by the run. The UI's Delete (a right-click menu item, a page's inline confirm) is the confirmation. |
| `oxplow.work_item.reorder { uri, before?, after? }` / `oxplow.work_item.move { uri, to: "backlog" \| { thread }, before?, after? }` | `External`: the item's list's `reorder` / `move` (oxplow's tasks: `oxplow_tasks::verbs::reorder_tx` / `move_tx`) | Only on a list declaring `ordering` / `lists`. Undoable — the inverse puts the item back next to the neighbour it had. `reorder` places an item before or after another of its own list (neither: at its end); `move` takes it to a thread's list or the backlog (at the end, or next to an item there), renumbering that list's `sort_index`. Logs core's `work_item.edited@2` (`list` and `rank`, or `rank`). An anchor from another list, both anchors, or an unknown thread is refused. |
| `oxplow.work_item.link { uri, target, link_type }` / `oxplow.work_item.unlink` (same input) / `oxplow.work_item.comment { uri, body }` | `External`: the item's list's `link` / `unlink` / `comment` (oxplow's tasks: `oxplow_tasks::verbs::{link_tx, unlink_tx, comment_tx}`) | `link` and `unlink` undo each other, `comment` isn't undoable; only with the provider's `links` / `comments`. A target that is an item must be the same provider's; any other must be a ref of a registered kind (`check_link_targets`, against the running vocabulary) and may dangle; every list with links takes `about`. oxplow: a link type of its own list (with `about`; a link belongs to its item, not a thread), or a note on the task (its page refs restated from the interface), logging core's `work_item.linked@2` / `work_item.unlinked@1` / `work_item.commented@2` caused by the run. |
| `oxplow.effort.open { thread?, work_item?, title?, adopt_since? }` / `oxplow.effort.close { effort \| thread, as_of?, summary?, reason? }` / `oxplow.effort.link { effort \| thread, work_item }` / `oxplow.effort.update { effort, title }` | `Tx` over `effort_store::start_tx` / `finish_tx` / `link_tx` / `retitle_tx` (`commands/effort.rs`) | `Record`: an effort is a record, not a claim on the worktree, so any thread may run them (an agent only in its own stream). `open` needs no work item; the thread's open effort closes first (`switch`), and `adopt_since` adopts the thread's un-efforted activity back to there, never before its previous effort ended. `close` names the effort or the thread's open one; `as_of` closes it at a past point and releases what came after; `reason` (`commit` / `switch`) records why, else it's the caller's (`agent`, `person`, an effect's `system`). `link` (`null` unlinks) and `update` (`null` clears the title) undo to what was there. See [work-tracking.md](./work-tracking.md) |
| `oxplow.effort.record_decision { thread?, work_item?, question, choice, alternatives?, confidence?, why? }` | `Tx` over `reasoning_store::record_decision_tx` (`commands/reasoning.rs`, P8.A7) | `Record`, `Confirm::Never`, not undoable. An agent's lands on its own thread whatever thread it names (another is refused); a person names one. It attaches to `work_item`'s open effort — refused when the item is another stream's work — else the thread's newest open effort. Read back as `v_decision`. An agent states claims it can back in `oxplow.effort.report`; a model reads the rest in its answers (`replace_inferred_claims`) |
| `oxplow.effort.report { thread?, summary?, impacts?, claims? }` | `External` (`commands/effort_report.rs`): it checks the summary's links against the worktree | `Record`, not undoable; optional. An agent reports only on its own thread; a person names one. It settles the effort lifecycle first, then records the summary, impacts and claims (at most 8 `{ statement, kind, evidence? }`, provenance `recorded`, replacing the effort's unreviewed recorded ones; a person's verdict stays) in one transaction on the thread's open effort, else its latest (refused at `/thread` when it has none) — never opening, closing or creating one — and returns `{ effort, link_warnings }`. Files and runs are observed, never reported, and without a summary `v_effort.summary` is the effort's last turn's final message ([work-tracking.md](./work-tracking.md)) |
| `oxplow.test.record_run { thread?, command, duration_ms?, passed?, failed?, total? }` | `External` over the `CollectionService` (`commands/test_runs.rs`): the capture commits in the collector's own transaction | `Record`, not undoable. An agent's goes on its own thread whatever it names; a person names one. The `asserted` run whose counts oxplow couldn't parse; it is the thread's open effort's. A report oxplow parses itself is a report collector's, run by hand with `oxplow.collector.sync`. See [collection.md](./collection.md) |
| `oxplow.extension.deploy { name, stream? }`, `rollback { name, to? }`, `enable { name }`, `disable { name }`, `remove { name }`, `share { name }`, `unshare { name }` | `External` (`commands/extension_deploy.rs`): copies a source into `.oxplow/runtime/extensions/<name>/<hash>/`, publishes the live set, and edits `.oxplow/personal.yaml`, `.oxplow/project.yaml` and `.oxplow/.gitignore` | `Write`, not undoable. `deploy` returns once the deployment runs; it fails, keeping the old one, when the extension does not prepare. `enable` / `disable` write the person's lists and are what Settings → Extensions calls; `remove` forgets deployments, state and settings and never deletes the source; `share` / `unshare` un-ignore a source and move its name to or from the team's `extensions:` list. See [extensions.md](./extensions.md) → "Sources and deployments" |
| `oxplow.snapshot.restore_file { file_snapshot }` | `External` over `snapshot_files::SnapshotFiles` (`commands/snapshot.rs`, P8.A9) | `Write`, `Confirm::Destructive` (it overwrites the file at its path), not undoable: a person's run asks first. Writes into the row's stream's worktree — refused when that stream is archived or gone, never written into another checkout — and announces the stream's `WorkspaceChanged` |
| `oxplow.lsp.install_server { package }`, `oxplow.lsp.remove_server { package }` | `External` over the `LspInstallerService` (`commands/lsp.rs`, P8.A9): the network and `.oxplow/lsp/` | `Write`, `Confirm::Always`, not undoable: a person's run asks first. Each pushes `LspServersChanged` (the server list isn't a model). See [lsp.md](./lsp.md) |
| `oxplow.agent_session.open { thread, kind?, harness?, title?, resume? }`, `oxplow.agent_session.rename { session, title }`, `oxplow.agent_session.close { session }` | `Tx` over `agent_session_store` and `agent_stores::close_session_tx` (`commands/agent_session.rs`) | `Record` (`agent_sessions.write`). **The command is the slot's lifecycle; the RPC is the process's.** `open` only inserts the row — it never starts a process and never prompts (the UI starts the process when the tab mounts; nothing types into it, which the no-automation guards keep true for a session an agent opened); `harness` is checked against the registered harnesses and the project's enabled ones, the default is `HarnessRegistry::default`, `kind` defaults to `terminal`, and `action` is refused (not built). `resume` (a saved conversation's native id) binds the new slot to it as `chosen` in the same transaction — refused for a conversation open in another slot, and denied to an agent, which may open a session but not choose its conversation. Undone by closing it. `rename` undoes to the old title. `close` asks no one and is undone by **`oxplow.agent_session.reopen { session }`**: the slot closes, its open turns end interrupted and it logs `stopped` in one transaction, and its process stops after commit (`agent_sessions::SessionProcesses::kill`); `reopen` opens a slot a person closed again (refused for one its thread's or stream's close took, or on a closed thread), and its process resumes the saved conversation when its tab mounts. An agent acts only on sessions in its own stream. `terminate_terminal_session` / `chat_close_session` stop a process and leave the slot open. Sessions are named by URI (`oxplow:oxplow/agent_session?id=ses3`). **`stop` / `start` / `start_fresh`** are a session's controls: `stop` sets `stopped_by_user_at` and stops the process after commit (undone by `start`); `start` clears it (the UI then spawns, resuming the saved conversation; undone by `stop`); `start_fresh` forgets the saved conversation too, the only way a session gets a new one — `Confirm::Destructive`, not undoable |
| `oxplow.thread.create { stream, title, from? }`, `oxplow.thread.rename { thread, title }`, `oxplow.thread.set_prompt { thread, prompt? }`, `oxplow.thread.mark { thread, mark? }`, `oxplow.thread.close { thread }`, `oxplow.thread.reopen { thread }` | `Tx` over `thread_store::{get,list_for_stream,upsert}_tx` (`commands/thread.rs`, P8.A3) | `Record`, `Confirm::Never`. The thread state machine: a thread is open or closed, nothing more (`threads.closed_at`); a new thread is open at the end of the stream's order, with no agent session (`from: thread:<id>` forks it on the same stream, copying nothing agent-ish); `mark` sets or clears (`mark` absent) a colour on the thread's Navigator tab — `amber`, `red`, `green`, `blue` or `violet` — for grouping threads the person's way, changing nothing else (undo sets the previous mark back); `close` ↔ `reopen` undo each other (a close also closes the thread's open effort, `closed_by` `system`, in the same transaction; a reopen leaves it closed), and a close closes its agent sessions (`thread_closed`) in the same transaction and stops their processes after commit; `rename` / `set_prompt` / `reorder` undo to what they were (`reorder`'s `order` names every thread of the stream once, so its undo puts each back); `create` isn't undoable; a move to the state a thread is already in (marking what it already is, closing a closed thread) is `unchanged` and leaves no record. Threads and streams are named by URI (`oxplow:oxplow/thread?id=thr12`, `oxplow:oxplow/stream?id=str1`). An agent acts only on its own stream and closes and reopens only its own thread; `set_prompt`, `mark` and `reorder` are the caller's (a prompt steers an agent); a prompt empty after trimming clears it. Every open thread of a stream shares its worktree and any may change it. The desktop's thread helpers run these through `run_command`; the views re-read on `ModelsChanged` naming `v_thread` |
| `oxplow.stream.create { title, source, existing_branch?, new_branch?, start_point?, worktree? }`, `oxplow.stream.archive { stream, delete_worktree? }` | `External` over `StreamService` (`commands/stream.rs`, P8.A4) | `create` (agents set up parallel work on a branch — `source: existing` (a branch that exists) or `new` (forked from `start_point`); `source: worktree` adopts a worktree already on disk, and is refused unless the path is one of the repository's worktrees: "{path} isn't one of this repository's worktrees"); the worktree's folder is made from the title (lowercase words, hyphens), so the caller names none; `archive`; `Write`, not undoable. Create reaches the VCS and starts the stream's capture service, and its `ui.open_after` is the new stream (the New Stream page gives way to it); archive is `Confirm::Destructive` (the desktop's Remove dialog is the confirmation), refused for the primary or an unknown stream before anything changes (`StreamService::archivable`), refused while an agent runs in one of the stream's threads, closes each of its threads' open efforts (`closed_by` `system`) at an `effort_end` snapshot taken first — before the working copy can go (`SqliteEffortStore::close`) — takes its threads with it, purges its search rows, forgets its worktree route and stops its capture, and removes its working copy when `delete_worktree`. A worktree that vanishes is archived by the workspace watcher directly — an observation, not an intent |
| `oxplow.stream.rename { stream, title }`, `oxplow.stream.set_prompt { stream, prompt? }` | `Tx` over `stream_store::{get,upsert}_tx` (P8.A4) | `Record`, undoable to what they were. An agent renames only its own stream; the prompt (appended to every agent prompt in the stream) is the caller's to set; one that is empty after trimming clears it. The stream list re-reads on `ModelsChanged` naming `v_stream` |
| `oxplow.hint.dismiss` (`Tx`) | `commands/hint.rs` over `agent_nudge_store::dismiss_tx` | `Record`: dismisses a hint raised to the person (`v_agent_nudge`, audience `person`), stamping its `delivered_at`; dismissing one already dismissed is `unchanged` (no record), one never raised `Invalid`. Not undoable |
| `oxplow.bookmark.set`, `oxplow.bookmark.remove` (`Tx`) | `commands/bookmark.rs` over `bookmark_store::*_tx` | `Record` (bookmarks are the person's navigation). The viewer is `thread` (its stream implied) or `stream`; `set` moves a page's bookmark to its `scope`. Both undo to what the viewer saw before |
| `oxplow.dashboard.create`, `oxplow.dashboard.rename`, `oxplow.dashboard.delete`, `oxplow.dashboard.remove_item`, `oxplow.dashboard.reorder_items` (`Tx`); `oxplow.dashboard.add_item`, `oxplow.dashboard.update_item` (`External`) | `commands/dashboard.rs` over `dashboard_store::*_tx` (P8.A5) | `Record`; `delete` is `Confirm::Destructive`, not undoable. `create` isn't undoable; the rest undo (a removed tile comes back at its position). Adding or editing a tile is `External` because its SQL is checked by the semantic engine (async) first. See [dashboards.md](./dashboards.md) |
| `oxplow.knowledge.add_comment { stream, thread?, target, quote, selectors_json, context_chain?, referenced?, intent, body }`, `oxplow.knowledge.reply_comment { comment, body }`, `oxplow.knowledge.update_comment { comment, intent?, status?, quote? + selectors_json? }`, `oxplow.knowledge.delete_comment { comment }` | `Tx` over `comment_store::*_tx` (`commands/comment.rs`, P8.A6) | `Record`, logging the existing `knowledge.comment.*` events caused by the run. The author is the actor (`user` for a person, `agent`, the person a lens acts for) — never an input. An agent comments only in its own stream and thread. `update_comment` undoes to the intent/status/quote it replaced (a relink is `quote` + `selectors_json`, which anchors it — `orphaned: false` unless it says otherwise — and its undo restores the old quote, anchor and `orphaned`; a half-named relink is refused before anything is written); `add`/`reply` aren't undoable; `delete` is `Confirm::Destructive`. `oxplow.knowledge.relocate_comment { comment, selectors_json, orphaned }` (tsk861) is the renderer's re-anchor after it re-finds the quote — not undoable; one already where it was is `unchanged` and leaves no record. Views re-read on `ModelsChanged` naming `v_comment` |
| `oxplow.handoff.send { to_thread, body, work_item?, from_thread? }`, `oxplow.handoff.settle { handoff, state, note? }` (`Tx`); `oxplow.handoff.resolve { handoff, outcome, note? }` (a native composite) | `commands/handoff.rs` over `oxplow_db::handoff_store::{send_tx, settle_tx}`, scope `handoffs.write` | `Record`. `send`: from the actor's thread (an agent's own, an effect's event's, the one a person names) to another open thread, any stream's; the item must be on the active list (a loose id is canonicalised); logs `handoff.sent@2`; not undoable. `settle`: `taken` / `declined` by the recipient thread, `withdrawn` by the sender, each only while open; `open` again by whoever settled it; logs `handoff.settled@2`; undone by the state it replaced. `resolve`: checks the same rule against the run's actor before composing, then `oxplow.work_item.move` onto the recipient thread (taking an item not there yet, on a list with `lists`) and the settle; a take that moves an item is steps, with no undo. See [agent-model.md](./agent-model.md) "Handoffs and native messaging" |
| `oxplow.knowledge.add_note { thread?, body }`, `oxplow.knowledge.update_note { note, body }` | `Tx` over `oxplow_tasks::satellite::{add_thread_note_tx, update_note_tx}` (`commands/note.rs`, P8.A6) | `Record`, `Confirm::Never`, so a queued agent (one that may not write the worktree) still takes notes. An agent's note lands on its own thread whatever it names; a person names one. `update_note` (a subagent filling in the note it was handed) undoes to the previous body and is refused on another thread's note. The result is `{ note, link_warnings }` — each `[[…]]` that doesn't resolve |
| `oxplow.effort.verify_claim { claim, evidence? }` / `oxplow.effort.unverify_claim { claim, evidence? }`; `oxplow.effort.dismiss_claim { claim }` / `oxplow.effort.reopen_claim { claim }` | `Tx` over `claim.review` and `claim.evidence` (`commands/review.rs`, P7.C4) | a review of a claim: any caller; `Record`; undoable as pairs. Each acts on an unreviewed claim (verify, dismiss) or one in the state it undoes, refusing otherwise. `verify_claim` marks it `verified`, citing `evidence`, else what it already cites (a run the agent's answer named), else `reviewer` (the person checked it); `unverify_claim` makes it unreviewed again, citing `evidence` (what it cited before, which verify's inverse carries). `dismiss_claim` marks it `dismissed` (the agent didn't claim it, or didn't mean it); `reopen_claim` takes that back. Reviewed claims stay when the effort is read again. Log `effort.claim_verified@2 { claim, effort?, evidence? }` and `effort.claim_dismissed@2 { claim, effort?, dismissed }`. The claim is named by ref (`claim:7`), whatever effort it's in |
| `oxplow.effort.confirm_decision { decision }` / `oxplow.effort.dismiss_decision { decision }` / `oxplow.effort.reopen_decision { decision }` | `Tx` over `decision.provenance` (`commands/review.rs`, P7.C4) | the same reviewers. Confirm and dismiss move an `inferred` decision to `confirmed` / `dismissed` (a recorded one isn't reviewed); reopen moves either back to `inferred`, and is their inverse (and they are its). Logs `effort.decision_reviewed@2 { decision, effort?, outcome }` |
| `oxplow.effort.replace_inferred_claims { effort, claims }` | `Tx` over `reasoning_store::replace_claims_tx` (`commands/reasoning.rs`) | `Record`, not undoable. The writer of `inferred` claims: at most 8 `{ statement, kind, evidence? }` a model read in the effort's answers replace its unreviewed inferred claims; the agent's `recorded` ones (`oxplow.effort.report`) and verified or dismissed ones stay. Thread and work item are the effort's, never the input's. Whether each is backed is a review policy's call (oxplow-bundled's `claim_status`) |
| `oxplow.effort.replace_inferred_decisions { effort, decisions }` | `Tx` over `reasoning_store::replace_inferred_tx` (`commands/reasoning.rs`) | `Record`, not undoable. The only writer of provenance `inferred`: at most 8 `{ question, choice, alternatives?, why?, confidence? }` (absent confidence is `low`) replace the effort's inferred decisions; its recorded, confirmed and dismissed ones stay. Thread and work item are the effort's, never the input's. Inferred rows never reach the agent (`reasoning.rs` reads `recorded` only), so proposing one grants nothing. Logs `effort.decisions_inferred@2 { effort, work_item?, count }` |
| `oxplow.config.list_keys {}` / `oxplow.config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value |
| `oxplow.config.set { key, value }` / `oxplow.config.unset { key }` | `Tx`: validate against the key's schema, take the new document through the loader's own validation (`oxplow_config::keys::with_key`); after commit, write the file and swap the in-memory config | undoable (inverse restores the prior value or unsets); logs `config.changed@1 { key, before, after }` (what the backend's `config.*` reactors run on); `after_commit` swaps the in-memory config, wakes `config_applied` and broadcasts the renderer's `ConfigChanged`; anyone sets any key, unasked (no key is a person's alone, in the personal layer too: what an agent may change is its harness's to decide, and it could edit the file anyway) |
| `oxplow.metric.enable { keys, enabled }` | `Tx`: turns metrics on or off — computes the new `metrics:` list with `MetricsService::apply_metric_enabled` (a bundled code metric is off until a `use:` names it; a producer/extension metric is on until an `enabled: false` marker) and hands it to `oxplow.config.set`'s core; an unknown key (not in `metric_catalog`) is refused | undoable (restores the prior list); logs `config.changed@1 { key: metrics, … }`; the `config.metrics` reactor reseeds |
| `oxplow.metric.record { key, value, subject?, dims?, stream? }` | `Tx` over `fact_store::record_facts_tx` (`commands/metric.rs`, P4.8) | not undoable. An asserted fact on the metric's source measure, stamped to match its filter, anchored to the stream's latest snapshot; the fact and the audit commit together. A formula or `count` metric, an unknown key, or another stream (for an agent) is refused. After commit: clears the fact memo (the change loop announces `MetricSamplesChanged` for the measure) |
| `oxplow.metric.rebuild { force }` | `External` over `MetricsService::rebuild_baseline` | not undoable. Every snapshot fact collector's whole-tree baseline. It drives snapshot captures and collector scripts, which own their own transactions. (Running one collector is `oxplow.collector.sync`; `metric.run` is deleted.) |
| `oxplow.vcs.commit` / `oxplow.vcs.stage` / `oxplow.vcs.discard` / `oxplow.vcs.fetch` / `oxplow.vcs.pull` / `oxplow.vcs.push` / `oxplow.vcs.merge` / `oxplow.vcs.checkout_branch` / `oxplow.vcs.rename_branch` / `oxplow.vcs.delete_branch` / `oxplow.vcs.resolve_conflict`; `oxplow.git.rebase` / `oxplow.git.cherry_pick` / `oxplow.git.revert` / `oxplow.git.ignore` | `External` over the `Vcs` trait (`git.*`: the git provider's own ops) (`commands/vcs.rs`, P5.B6) | any caller's, a lens's too — an agent's on its own stream only (its terminal's `git` reaches only its own worktree; `vcs_op` refuses another stream), not undoable. Each takes the `stream` it acts on, by URI (`oxplow:oxplow/stream?id=str1`), resolved strictly (an unknown stream is refused, never the primary's). `oxplow.vcs.discard`, `oxplow.vcs.merge`, `oxplow.vcs.delete_branch`, `oxplow.git.rebase` and `oxplow.git.revert` are `Destructive` (confirmed); `oxplow.vcs.pull` and `oxplow.vcs.push` don't ask (routine, the person's own click). The spec is the one rule: the Git Dashboard arms its confirm from it (`SpecConfirm`, tsk898). The result — and the audit row's — is the VCS's `OpOutcome { success, log, conflicts, auto_resolved }` (`oxplow.vcs.commit`: `{ success, revision }`); an op that failed outright (a rejected push, an unreachable remote) is the run's `Failed`, git's log its message, audited `error` — never an `ok` row whose result says it failed — while one that stopped at conflicts answers with them (the repository is mid-merge for a person). Once a run is recorded (its `after_commit`; at once for one that failed, which may have touched the workspace too) the stream's `WorkspaceChanged` (and `VcsRefsChanged`; every stream's after a fetch, push, rename or delete) is announced. See [vcs.md](./vcs.md) |
| `oxplow.knowledge.write_page { slug, title?, body, verified_files?, removed_files? }` / `oxplow.knowledge.delete_page { slug }` / `oxplow.knowledge.link { page, target }` / `oxplow.knowledge.resync { slug }` / `oxplow.knowledge.verify_files { slug, refs? }` | `Tx` over `knowledge::write_page_tx` / `delete_page_tx` (`crates/oxplow-app/src/knowledge.rs`, P5.C3) | `Record` (any thread captures); not undoable; `delete_page` is Destructive. `verify_refs` re-pins the named files (every file the page cites when `refs` is omitted) against the page core records, through the same `write_page` path. Validates the slug and every `[[link]]` (refused, named), restates the `wiki_page` row and its pinned `page_ref` edges, logs `knowledge.page.written@2` / `knowledge.page.deleted@2` with the actor's anchors; writes (or removes) `.oxplow/wiki/<slug>.md` in the run; the UI re-reads on the row's `modelsChanged` (no wiki event of its own). See [knowledge.md](./knowledge.md) |
| `<provider>.<name>` (an enabled external provider's own declared commands — not its capability's verbs, which run as `work_item.<verb>`: the fake's `estimate`) | `External` over the provider process's `invoke` (`providers/registry.rs`, P5.D3) | registered while its instance is enabled (and removed when it stops), `Experimental`; `confirm`, `access` and `undoable` as the provider declares (its `inverse` becomes `<provider>.<command>`). The events its `invoke` returns are logged caused by the run — only types it declares, and a `work_item.recorded` only for its own items. See [providers.md](./providers.md) |
| `oxplow.collector.sync { collector, thread? }` | `External` over the collector's program or script (`collector_runner.rs`, P7.B3; was `source.sync`) | `Write`, not undoable. Runs a collector from the primary worktree and commits its rows, its `collector_run` row and `collector.synced@2` in one transaction (a failed run commits the failure). It never admits anything: a collector whose extension lacks a permission it needs (`process` for an exec one) is `Invalid`. Run by the system it's the `every:` schedule's run (`trigger: every`); by anyone else, `manual`. **The one manual run for every collector** (it replaced `source.sync` and `metric.run`): a fact collector (`facts:`, owner an extension, the project's own `project` among them) runs through the fact engine (`MetricsService::run_collector_by_key`) against the stream's latest snapshot, recording its capture, `collector_run` and `collector.synced@2` (the engine's own writes). Result `{ owner, id, rowCounts, facts }` — `rowCounts` for an entity collector, `facts` (count recorded) for a fact collector. A **report collector** (`records:`, tsk863) is read by the collection service in `thread` (an agent's own; a person names one): `{ owner, id, recorded: { status, records, run, … } }` |
| `oxplow.provider.sync { instance, collector? }` | `External` over the provider process's `read` (`providers/sync.rs`, P7.A3) | `Record` (it restates items, never claims), not undoable. Reads a running instance's collectors (all, or one — an undeclared one is `Invalid` at `/collector`) from their last checkpoints; the records land as `work_item.recorded@3`, each batch committed with its `$/state`. The one way to read: Settings → Integrations' Sync Now, an agent, the schedule (as the system) and the read an instance gets when it starts. Result `{ reads: [{ collector, records }] }`. See [providers.md](./providers.md) |
| `oxplow.file.save { stream, path, content }` / `oxplow.file.create { stream, path, content? }` / `oxplow.file.create_directory { stream, path }` / `oxplow.file.rename { stream, from, to }` / `oxplow.file.delete { stream, path }` | `External` over `WorkspaceFiles` (`commands/file.rs`, scope `files.write`): the worktree is the system that owns this state | a person in any stream, an agent or effect only in its own (`stream_of`); `delete` is `Confirm::Destructive` (a person's). Each logs what it did — `file.saved`, `file.created`, `file.directory_created`, `file.renamed`, `file.deleted` (subject `file:<path>`, the stream anchored) — which an effect `on:` reacts to and which the window follows with its open buffers (`useFileEvents`, `fileEvents.ts`: a rename moves them, a delete closes them, whoever ran it). `save` and `create` leave `content` out of the audit (`with_unrecorded`). The five bespoke workspace write IPCs are gone |
| `oxplow.capability.choose { capability, provider?, layer }` / `oxplow.replacement.choose { target, whose? }` | `Tx` config ops (`commands/config_commands.rs`, scope `config.write`): `choose_provider` edits that capability's entry in `activeProviders` of the project or personal layer (no provider unsets it), `choose_replacement` edits `replacements` | Settings → Integrations → Capabilities and Replaced components, as row actions of the `capabilities` and `replacements` lenses |
| `oxplow.ai.set_provider { id, kind, base_url?, key? }` / `oxplow.ai.remove_provider { id }` / `oxplow.ai.set_role { role, provider?, model?, restrict? }` / `oxplow.ai.test_provider { id, model }` | `External` over `AiService` (`commands/ai_settings.rs`, scopes `ai.settings`, and `ai.test` for the test, which `spends`): the settings are the user-global `ai.yaml` and the OS keychain, outside oxplow's database | Settings → AI as a form. `set_provider`'s `key` is a secret (`writeOnly: true` in its schema; "Secrets" below): kept out of the record, masked in the form; `remove_provider` is refused while a role uses the provider; `set_role` with neither provider nor model unbinds the role and announces `ConfigChanged`; `test_provider` returns `{ reply }`. The `save_ai_provider` / `remove_ai_provider` / `set_ai_role` / `test_ai_provider` IPCs are gone; `ai_settings` still reads |
| `oxplow.provider.check_instance { instance, config? }` / `set_instance { instance, enabled, config? }` / `add_instance { instance, provider, scope }` / `turn_off_here { instance }` / `remove_instance { instance }` (scope `providers.write`) and `oxplow.provider.set_credential { instance, name, value? }` / `begin_sign_in { instance, name, redirect_port }` / `complete_sign_in { instance, name, redirect }` / `cancel_sign_in { instance, name, sign_in }` (scope `credentials.write`) | `External` over `ProviderRegistry` (`commands/provider_instances.rs`): enabling an instance runs its program and its credentials are in the OS keychain. `set_credential`'s `value` is `writeOnly`, kept out of every record like `oxplow.collector.set_credential`'s. A destructive `remove_instance` is confirmed. Which actors may enable is the registry's (`Actor::Human` for a global instance's file) | Settings → Integrations (the `IntegrationsSection` until it is a foundation component); instances read as `v_provider_instance`, `v_provider_credential`, `v_provider_collector` |
| `oxplow.collector.set_credential { collector, name, value? }` / `oxplow.extension.set_secret { extension, reference?, value }` / `oxplow.extension.revoke_secret { extension, reference }` | `External` over the keychain (`commands/credentials.rs`, scope `credentials.write`; `set_secret` and `revoke_secret` over `ExtensionSecrets`, which also restarts a provider running on the secret) | Settings → Extensions as forms (`extensions-credentials`, `extensions-secrets`). `value` is a secret (`writeOnly`): kept out of the record, masked in the form, a blank `set_credential` value clears it. A credential must be one some collector of that extension declares (`Invalid` at `/name`); `set_secret` needs a current owner. Announce `CredentialChanged`; presence reads `v_collector_credential` and `v_extension_secret`. The `set_credential`, `save_extension_secret`, `extension_secret_is_set` and `revoke_extension_secret` IPCs are gone |
| `oxplow.agent.set_prompt_append { text }` | `Tx` (`commands/agent_settings.rs`, scope `config.write`) over `config_commands::change` | Writes `agentPromptAppend` (blank removes it): the Agents area's prompt form. The same one config write as `oxplow.config.set { key: agentPromptAppend }`, with a text field rather than a JSON value |
| `oxplow.agent.set_agents { agents }` / `oxplow.agent.set_setting { agent, key, value? }` | `Tx` (`commands/agent_settings.rs`, scope `config.write`) over `config_commands::change` | Settings → Agents: `set_agents` checks each key is a registered harness (`Invalid` at `/agents/<i>`, none at all at `/agents`) and writes `agents`; `set_setting` checks the harness declares `key` (`/key`, an unknown agent `/agent`) and rewrites `agentConfig` (a blank value clears it, an emptied entry goes). Both are the one config write: schema-checked, `config.changed@2`, undoable (the inverse is `oxplow.config.set`). The prompt addition is the key `agentPromptAppend` (`oxplow.config.set` / `unset`). The `set_agents` / `set_agent_setting` / `set_agent_prompt_append` IPCs are gone |
| `oxplow.file.mark_generated { entry, generated? }` | `Tx` (`commands/config_commands.rs`, scope `config.write`) over `config_commands::change` | The Files page's Mark / Unmark as Generated: adds `entry` (trimmed of `/`) to the `generated` key's sorted `exclude` list, or with `generated: false` takes it off; nothing marked removes the key. One config write: `config.changed@2`, undoable. The `set_generated` IPC is gone |
| `oxplow.delivery.retry { id }` / `oxplow.delivery.discard { id }` | `External` over the event pump (`commands/delivery.rs`, scope `deliveries.write`): the pump owns the dead letter (`v_event_dead_letter`) and a retry runs a consumer's handler again | `discard` is `Confirm::Destructive`. `retry` returns the letter (`retried`, or still `pending` with the new error); a letter that isn't pending, has no consumer or doesn't exist is `Invalid`. The `list_/retry_/discard_dead_letter` IPCs are gone |
| `oxplow.lsp.restart_server { stream, language }` | `External` over `LspSessionManager::restart` (`commands/lsp.rs`, scope `lsp.control`): the server is a process on this machine | tears the stream's session down and starts it again, replaying the open documents; pushes `LspServersChanged`. The `restart_lsp_server` IPC is gone |
| `oxplow.effect.retry { effect, event }` | `External` over `effect_triggers::run_reaction` (`commands/effect.rs`, P9.D4; the `effects.run` operation, its services filled at boot when the `effect.triggers` consumer registers) | the Alerts page's failed reactions; `Confirm::Always` (a person's run asks), not undoable. Has an extension's effect react again to an event its reaction to **failed** (its latest attempt, `v_effect_run.latest`), as the next attempt, run as the effect is now — enabled and deployed as it is, composing afresh. Asked every time: a failed attempt interrupted with a step outside oxplow under way may have landed it, and a retry sends it again. `Invalid` for a reaction that didn't fail or was never made, a disabled effect, an unknown effect or event. Result `{ effect, event, attempt, outcome, reason? }` — the retry's own outcome, which may be `failed` again. See [extensions.md](./extensions.md) "Effects" |
| `oxplow.effect.backfill { effect, from_seq? \| since?, to_seq? }` | `External` over `commands/effect.rs::backfill` (P9.D5) | `Confirm::Always`, not undoable. Has an effect react to the matching events in the range it never reacted to (those logged before it was enabled), oldest first, once each, as it is now — at most 200 a run. Each is an attempt with `origin: backfill`, under the same dedupe, loop guard and health as a live reaction; three failures in a row disable the effect and stop the run, and so does the effect being disabled (checked before each reaction). `Invalid` for an unknown or disabled effect, or both `from_seq` and `since`. Result `{ effect, planned, ran, skipped, proposed, failed, remaining, stopped? }` |
| `oxplow.effect.backfill_plan { effect, from_seq? \| since?, to_seq? }` | `Tx`, `Read` (P9.D5) | What `oxplow.effect.backfill` would react to: `{ effect, planned, from_seq, to_seq }`. The count a person is shown before running one (a confirmation carries a command's summary and input, not a computed count) |
| `oxplow.contribution.enable { extension, kind, contribution }` | `External` over `contribution_health.rs` and the provider registry (P7.C1; replaced `provider.enable`) | Not undoable. Enables a disabled contribution on this machine again — clearing an automatic disable (`contribution_health` back to `ok`, its count starting over). `kind` (`provider` \| `collector` \| `effect`) is part of its name — a provider and a collector may share an id (tsk721); there must be a failed one of that kind, or a provider instance the registry knows. A provider instance resets its backoff and reconciles, so it starts when `extensionInstances` enables it; a collector runs at its next trigger. Logs `contribution.enabled@1 { extension, contribution, kind }`; the result is its `contribution_health` row. See [providers.md](./providers.md) |
| `oxplow.contribution.enable_all { extension? }` | `External` over `contribution_health.rs` and the provider registry, like `enable` | Not undoable. Enables every contribution a failure policy disabled (`contribution_health.state = 'disabled'`) — `extension`'s, or every extension's — each exactly as `oxplow.contribution.enable` does (a provider instance resets and reconciles; a collector runs at its next trigger), logging one `contribution.enabled@1` apiece. Returns `{ enabled: [<contribution_health row>…] }`, empty when nothing was disabled. Settings → Extensions and Data → Collectors offer it as **Enable All Disabled (N)**. |
| `oxplow.lens.show { lens? , spec?, params? }` / `oxplow.lens.keep { answer \| spec, stream?, extension?, slug? }` | `External` (`commands/lens.rs`) | `show`: `Record` — checks the spec's query as `query_sql` does, metric functions resolved (`SqlGateway::check`), as its pre-check (so an agent can show a metric chart, called directly or composed), then its shape in the transaction that stores a `thread_answer` on the caller's thread (an agent may name only its own; a person any) with its open turn and effort; a named `lens` is the main worktree's, whatever the thread's stream, logs `lens.shown@2 { answer, thread, lens? }`; a refused query stores nothing. It returns `{ answer, title, text }`, the text being the answer's rendering once stored (`text_answer`: the main worktree's lenses in the answer's own thread's lens context, the one resolution `run_answer` uses too) — what `oxplow_show_lens` hands the agent; that's why it's `External`). `keep`: `Write`, not undoable — writes an answer, or a spec (Explore Data's Save as Lens; its shape checked as `show` checks one and its query through the SQL gateway — the metric functions rewritten, then the read contract — tsk987; in `stream`'s worktree — an agent may name only its own thread's stream, tsk988 — else the caller's thread's, else the primary's), as a lens in `my-lenses` or another extension's source — (an answer's params become defaults; `intent.origin` is the answer's thread, or the caller's), sets an answer's `kept_lens`, logs `lens.kept@3 { answer?, lens }`, and deploys the extension (`oxplow.extension.deploy`), then returns `{ lens, path }` once the lens is live; a failed deploy takes the lens file back out. `keep` writes lens files, which a `Tx` handler may not (the bus retries one on a busy database, and a retried file write strands the first): `keep` reads the answer in one transaction, claims it as kept in another (`claim_kept_tx`: only if it isn't yet, so of two keeps of one answer only one writes), writes the file, and gives the claim back when the write fails. The bus records each run and its events after the handler returns. See [extensions.md](./extensions.md) → "Thread answers" |
| `oxplow.ui.report_error { label, command?, message?, exit_code?, thread?, stream?, signal?, duration_ms?, stderr?, stdout? }` | `Tx` (`commands/ui.rs`, tsk1072) | `Record`, not undoable. The app's `recordOpError` runs it, fire-and-forget, for each op error; the Alerts page lists `v_op_error`, not a client copy. Logs `ui.op_failed@1` anchored to `thread` (and its stream), or, from no thread, to the `stream` on screen so its agent can read the output (not both: `Invalid` at `/stream`, tsk1079); stderr / stdout stored as its `output` body (`put_json_tx`, namespace `ui`); the agent reads them as `v_op_error` and `read_event_content(event_id, output)` |
| `oxplow.op_error.mark_seen { event }`, `oxplow.op_error.dismiss { event }`, `oxplow.op_error.dismiss_all {}` (`Tx`) | `commands/op_error.rs` over `oxplow_db::op_error_store` | `Record`, not undoable: the person opened a failed operation's output, or cleared it from the app's list. Each logs `ui.op_error.seen@1` / `ui.op_error.dismissed@1 { event }`, anchored as the failure and kept as long (the `ui` window), read back as `v_op_error.seen` / `dismissed` (dismissing is seeing). `event` that isn't a `ui.op_failed` event is `Invalid` at `/event`; one already in that state is `unchanged` (no record); `dismiss_all` logs one per failure not dismissed, `{ dismissed: n }` |
| `oxplow.metric.scaffold { name, language?, glob?, origin? }` | `Tx`, `Read` | the files of a code metric extension (`{ name, dir, files: [{ path, content }] }`: manifest, collector script, lens, fixture — the template `oxplow extension new metric` writes); writes nothing (the agent writes them, then `extension check` / `extension test`) |

The `work_item.*` commands are **every** provider's
([work-items.md](./work-items.md)): each names its item by its canonical
URI and is dispatched to the URI's provider; an unregistered provider is
refused naming the registered ones (`no work-items provider \`issues\`;
registered: oxplow`).

**Callers.** Every task edit or status change made for someone is a
`work_item.*` command run under its actor. The desktop runs them through
RPC `run_command` as `Actor::Human` (the typed task RPCs went in P6.E1b);
an agent runs them through MCP `run_command` as `Actor::Agent` with the
caller's verified thread and stream (`McpCaller` — see "MCP identity"),
so an anonymous connection can't file or change a task. P8.A10 deleted the MCP task-write tools (`create_task`,
`file_epic_with_children`, `update_task`, `complete_task`, `upsert_task`,
`transition_tasks`, `reorder_tasks`) and `oxplow_app::task_writes` with
them; an epic is a `oxplow.work_item.create` per row (children with
`parent_uri`), and closing is the `oxplow.command.sequence` above.
`oxplow_app::work_items::WorkItems`
is a typed client over the commands (the conformance suite uses it).
**Config is written only by `config.*`** (tsk515). The Settings page's
typed IPC setters (`set_agents`, `set_agent_prompt_append`,
`set_agent_model`)
each run `oxplow.config.set` / `oxplow.config.unset` as
`Actor::Human`, confirmed when the key asks — through `oxplow_rpc::commands::config::set_key`.
`config_service` only reads. `only_the_config_commands_write_project_yaml`
scans the crates for any other `write_project_key` call. A key whose
new value must reach something running reacts to the `config.changed`
event on the pump, reading the value from the event's `after` (the pump
can see the event before the after-commit swap): `generated` →
`config_reactors::WorkspaceFilterConsumer` updates the snapshot captures'
filter, so an agent's change applies like the person's.
**The person's way onto the bus** (P5.A1): RPC `run_command { id,
input, confirmed }` and `undo_command { audit_id, confirmed }`
(`oxplow_rpc::commands::bus`, `Actor::Human`; desktop `runCommand` /
`undoCommand` in `api.ts`). A call that needs confirmation comes back
`NEEDS_CONFIRMATION` and the UI asks, then calls again with `confirmed`.
A typed IPC setter is a convenience over one command; anything new the
UI writes goes through `run_command`. Parity: `both("run_command")`,
`ui("undo_command")`. A person's undo is offered where they ran it: a
command run through `personCommands` (a board move, a search offer)
that comes back undoable (its outcome's `inverse`) shows **Undo** on its
"done" toast, which runs `undoCommand(audit_id)` — the click is the
person's confirmation — and says "undone" or records the failure
(P11, tsk975). The thrown error keeps its IPC code
(`IpcCallError.code`, `needsConfirmation(e)` in `ipc-error.ts`), and
the UI asks with **`CommandConfirm`** (`components/CommandConfirm.tsx`:
the command's summary from RPC **`get_command { name }`**, destructive
ones marked; Run focused, Escape cancels). A command's `input_schema`
renders as a form with **`SchemaForm`** (`components/SchemaForm/`:
schemars' shapes — `$defs`/`$ref`, `Option<T>`, enums, nested objects,
string lists — else a JSON field; Enter submits, Escape resets, submit
disabled while a field has a problem; model in `schemaFormModel.ts`).
P6.B2.

`CommandError` → `McpError` mapping lives in `command_error` (oxplow-mcp):
invalid/denied/unknown are the caller's to fix, `Proposed` says the run
waits for a person (`run_command` returns it as a success instead),
`Failed` is internal.

## Exposure to agents (MCP)

Agents reach every command through two generic tools — `list_commands`
and `run_command { id, input }`. `list_commands` answers in two steps,
over the commands the calling agent may run and that are offered now
(step 0): with no arguments, an index of `{ id, summary }`; with `id`
(one command) or `prefix` (an area, `oxplow.work_item.`), each as the
agent needs it — `id`, `summary`, `input_schema`, its `example` input
(lifted out of the schema's `examples`, shown once), `confirm`,
`undoable`. The person-facing parts of a spec (`ui`, `op`, `atomicity`,
`needs`, `access`) never reach an agent. An id or prefix it doesn't offer
is refused naming the offered ids that end alike (`config.get` →
`oxplow.config.get`). Measured on our own sessions, agents guessed shapes
rather than read a listing of every full spec (164 KB): the index keeps
discovery cheap, the example makes the first call right, and a refusal
quotes the example. `run_command` answers with the outcome:
`result`, `audit_id`, `event_id`, `inverse?` — or, for an effect's run that asks,
`{ kind: "proposed", proposal, message }`). Every other tool is a
command or lens an extension declares with `tool:` ([extensions.md](./extensions.md)
"Agents: the MCP surface"): a named entry to a command, run through the
bus like `run_command`, hinted read-only by its access. Records are
written only through commands; there are no write tools of their own.

**Caller identity.** The control plane admits an MCP request only with an
agent session's bearer, minted at its launch, and puts that session's
`Principal` (session, thread, stream, harness) in the request's
extensions; `oxplow_mcp::McpCaller::from_parts` reads it from the
`http::request::Parts` rmcp attaches to each tool call
([agent-model.md](./agent-model.md) "Caller identity"). Nothing the
request says about itself — an `X-Oxplow-Thread` header, a `?thread=`
query — names the caller. A transport with no HTTP parts (stdio, a unit
test) is an anonymous agent: it may list, and `run_command` refuses it —
no run without an actor to audit it to. `OxplowMcp::verified_actor` still
resolves the thread, refusing one that no longer exists.
The wire test `crates/oxplow-control-plane/tests/mcp_wire.rs` proves the
headers reach `command.executed`'s `source = agent:thr…`.

## Adding a command

oxplow's own commands are declared in `extensions/oxplow-foundation`
over operations of the scopes ("Scopes" → "Operations"); `Services::new` adds the operations and each activation
registers the declarations from foundation's active deployment. Only `oxplow.command.sequence`, the
bus's composition primitive, is registered as a Rust command. A bus with
one area's operations (a test's, `oxplow-dev`'s) registers what's
declared over them with `register_declared`.

1. Define the input as a Rust struct with `JsonSchema`
   (`deny_unknown_fields`); the operation's `input_schema` is derived
   from it (`commands::util::schema`). A handler reaches for
   `commands::util` — `parse` its input, `invalid(field, …)` to refuse at
   a field, `sql` for a SQLite error (so a busy database retries the run
   instead of failing it), `failed` — never a private copy.
2. Write the handler — `Tx` unless it must call a pre-existing service.
   Return the inverse (a call of the command that undoes it) when it is
   undoable, and any domain events as typed envelopes
   (`Envelope::typed::<T>`).
3. Make it an operation of the scope it falls under
   (`Op::new(scope, name, schema, undoable, handler)`, plus
   `with_confirm_for` / `with_precheck`), give it the least it is
   confirmed (`confirm_at_least`), if any; add the scope to
   `SCOPES` if it's new (its access is the command's), and add the op
   in `Services::new` (`commands.add_op`).
4. Declare the command in `extensions/oxplow-foundation/extension.yaml`:
   `name: <area>.<verb>`, `summary`, `scope`, `op`,
   `confirm` (at least the operation's), `needs` (features), `ui`, `job: true`
   if a job's model may call it (only with an `entry:` of its own), and
   an `example:` input: realistic, records named by their URIs. An
   agent is shown it beside the schema, and a refused input quotes it.
5. Route the existing RPC/MCP entry points through `commands.run(...)`
   rather than calling the service directly, so the human's and the
   agent's runs are audited alike.
6. Tests: the input schema rejection names the field; the
   confirmation rules hold; the audit row and `command.executed` share
   the transaction; undo applies the inverse.

## Secrets

A secret is a kind of value. A command input field whose JSON Schema says
`writeOnly: true` is a secret (`oxplow.ai.set_provider`'s `key` is one).
Nothing else declares it: `commands::secret_fields` reads the schema.

- It is unrecorded by itself: the audit row and the events keep it out,
  as a `with_unrecorded` field is kept out ("Private operation inputs").
- Every `SchemaForm` masks it, and it never enters a draft.
- No model returns it: a read gives "set" or "not set", never the value.

## Private operation inputs

An operation's `with_unrecorded` fields are represented by omitted-byte metadata
in audit. The reserved `*` marker withholds the entire input, including non-object
inputs, and counts JSON bytes without allocating a serialized payload. Private
state CAS uses this for values, returns only a stamp and has no inverse payload.

Composition captures private descendants from the routed command tree. Its
invocation-local spec then withholds the entire parent input, since arbitrary
composers can derive a private child from any field. This never changes the live
registry, schema, grants or handler. Step success/failure metadata respects each
child's recording policy; private failure diagnostics are replaced with fixed
messages in stored audit/run history. Early failed composites conservatively
withhold their input before descendants are known. The caller still receives the
actual error.

Proposals retain replayable inputs in a public model. A private-input call or
composition needing confirmation is therefore refused before proposal storage
or dry run. A person can make a direct confirmed invocation; unconfirmed calls
can still run where ordinary command admission and grants permit them. Private
data read and intentionally returned/emitted by extension code is governed by
that extension's own explicit output contract; input redaction does not sanitize
arbitrary extension-authored outputs.

Declared jobs expose native `jobs.write/enqueue`, `jobs.write/cancel` and
`jobs.read/snapshot` operations for extension-owned command declarations. Intent
and provenance commit in the audited transaction; request content is
`unrecorded`, including parent composition/proposal handling. Model work is
performed later outside SQL under the job's extension's current permissions and exact registry
owner. `jobs.read` is also a read-only scope service for composers and workers;
it returns owned status/results and never the original request. See
[extensions.md](./extensions.md) for declaration/input shapes and lifecycle.
