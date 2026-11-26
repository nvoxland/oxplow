# Commands: the one write path

What this doc covers: the command bus — what a command declares, who
may run it, the pipeline every run goes through, the command run record and undo,
and how a new command is added. Built in
P1.8–P1.10: the bus, `config.*`, and the generic MCP
surface (`list_commands` / `run_command`) with caller identity.

## What a command is

Page authoring is also a command: `oxplow.extension.write_page` saves YAML
and optionally deploys it; `oxplow.extension.new_dashboard` creates an
empty grid page from a title and deploys it. Both enforce the primary
source rule before writing. Visual dashboard edits call `write_page`,
preserving YAML comments and order; they do not use the legacy dashboard
store commands. See [dashboards.md](./dashboards.md).

A command is a typed operation identified by its **command id**,
`<namespace>.<area>.<verb>` in snake_case (`oxtask.task.move`,
`oxplow.config.set`, `acme_pr.issue.close`; `CommandSpec::validate_id`).
The id is the machine name; what a person reads is the command's label.
**An input field named for one resource takes a bare id** (`{"thread":
"thr3"}`, `"stream": "str1"`, `"request": "req12"`,
`"note": "not7"`, `"snapshot": 7`; an integer id is a JSON number; "Bare ids"
in [resources.md](./resources.md)), parsed by `commands::util::bare_id`
(it replaced `ref_id`) — a URI is refused at its field, saying the id it
names. A field that can name anything (`uri`, `target`, `subject`, a task) still takes a URI. A result names the records it made the same way
(`thread.create` returns `{"thread": "thr3"}`), so it feeds the next call.
`CommandError::Proposed` carries the integer `proposal` and `supersedes`. (Hints and file
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
`replace_namespaces` takes an atomic list of
`NamespaceCommands` (namespace, source, prepared commands). Each source appears
once. Every replaced source is removed from a cloned registry before any new
batch is validated, so participating sources can exchange namespaces, IDs or
shortcuts. Unmentioned sources remain intact. The final registry swaps under
one write lock; any rejection preserves every prior command and holder. An
empty command set removes its source. Commands already resolved keep their Arc
handlers. This boundary covers the command registry, not SQL datasets or the other
extension registries. `prepare_namespaces` validates a candidate off the active
registry lock; `activate_namespaces` accepts it only on the exact registry used
for preparation. Every successful registration/replacement/removal advances an
allocation identity retained by candidates, avoiding revision wraparound and ABA.
An intervening mutation or activation on another bus returns Unavailable and
leaves the active registry intact. Activation remains an in-memory operation,
not a DB commit boundary.

The native generation coordinator can reserve a checked command candidate and
its extension owner metadata before SQL commit, then apply the replacement while
retaining both locks through the remaining generation maps. Until the first generation is
activated (boot's `initialize`), command activation keeps its optimistic
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

Extension commands are staged together from one CatalogSnapshot and activated
as one fenced registry candidate by the generation coordinator (activation) —
the only path that registers them. An enabled extension that fails to stage is
held back on its own (the generation names it in its problems); partially
parsed command lists never replace prior handlers. Namespace/shortcut
validation spans all sources. Disabled/removed sources are withdrawn in the
same candidate, and captured command contexts have a live owner check so
already-routed handlers cannot preserve that authority. A catalog change during
staging rejects activation and the owner stages again. Raw filesystem
snapshots remain authoring inputs. `unregister_source` removes one source's (an extension disabled, an
external provider stopped), freeing a namespace left empty. Migration V26 rewrote
history (command runs, proposals, `command.*` payloads) from the old two-part
ids. A command declares (`oxplow_domain::commands::CommandSpec`):

| Field | Meaning |
|---|---|
| `input_schema` | JSON Schema; the bus validates the input first and names the failing field, plus what the schema accepts there (`InputValidator::check`: an object's fields, required first, for an unknown or missing one; the choices for a bad enum value and that field's `description`), and quotes the schema's example input, so a caller fixes it in one more call. The example is JSON Schema's own `examples` keyword, from the command's declaration (`example:` in its manifest, kept over its operation's schema by `Op::command`; a script command's first example that runs when it declares none; `oxplow.command.sequence`'s in `compose.rs`). Every command an agent is offered has one that fits (`every_command_an_agent_is_offered_has_an_example_that_fits`) |
| `job` | a model's tool call inside one of its extension's jobs may run it (`job: true`; `false` unless said, and only on a command with its extension's own handler — `Op::command` refuses it over a scope's operation, so none of core's commands takes a job). Who else may run a command isn't the command's to say: **anyone** — a person, an agent, a page acting for either. oxplow puts no wall between a person and their agent; what an agent may call is its harness's to decide (its permission mode) |
| `confirm` | `Never`, `Always`, or `Destructive` — a **person's** run asks them first (the second click). An agent's run doesn't ask (`commands::asks`); an effect's is kept as a proposal; a job's or oxplow's is refused. A command that only reads (`View` / `Read`) may not ask (`Command::new` / `with_confirm_for` refuse it — so a read or view scope's command can't declare `confirm`): its detail has no execution event or proposal link |
| `undoable` | the handler returns an inverse call that `undo` applies |
| `lifecycle` | `Stable` / `Experimental` |
| `atomicity` | `Tx` (handler runs inside the bus's transaction), `External` (against a system the bus doesn't own — a VCS, a process, an outside system), or `Dispatch` — a composite, which its calls decide ("Composition") |
| `access` | `oxplow_domain::Access`: `View`, `Read`, `Record` or `Write` (the default); a scope operation's command takes its scope's. `View` (changes only what the person sees) and `Read` keep a run and detail row (`Access::reads_only`) with NULL input, result and inverse and no `command.executed`. A polling agent does not fill the event log, and a closed thread can still run one (`oxplow.config.list_keys`, `oxplow.config.get`). `Record` and `Write` are recorded (`Access::records`). A `Record` changes oxplow's own records (a store's `task.*`) and is open to any thread; a `Write` is refused to an agent whose thread is closed or missing (`Access::needs_thread`) |
| `unrecorded` | top-level input fields its command run row leaves out, kept as their size (`{ "omitted_bytes": n }`; an op's `with_unrecorded` — `oxplow.file.save`'s `content` — or an extension command's `unrecorded: [field…]`, each a property of its input or `*`, checked at load): still recorded, the log not grown by every file saved. A page form's `secret` field must be one of them |
| `needs` | the **scopes** its handler calls (`sql.read`; below — always available, enforced per call) and the capabilities, features or shared models (`model:oxplow/task_record`) it needs active — the grammar pages use. Unmet, it isn't offered and doesn't run (pipeline step 0). oxtask's `oxtask.task.*` commands need the task model |

`Actor` is who runs it: `Human`, `Agent { thread_id, stream_id }`,
`Page { page_id, on_behalf_of }`, `System`. Its `source()` (`human`,
`agent:thr3`, a page's URI `oxplow:oxplow/page?page=acme/x`) is what the event and command run record.

`Actor::Job` is host-derived from a validated job/attempt/fence identity
(source `job:<id>:<attempt>:<fence>`): what a job's model tool calls run
as ([extensions-jobs.md](./extensions-jobs.md) "Jobs"). It carries the thread and
stream the job was asked in, for the gates, and is confined to that
stream. It cannot confirm or create a proposal: a command that asks is
refused. A job runs only a command that says `job: true`, which an
extension declares on the read-only commands its jobs name as `tools`
(`commands::job_refusal`); every child a script composes is re-checked,
and none of core's takes a job. A page wrapping a Job keeps Job admission and its confinement. A
`Write` command from a gated actor without a thread is refused.

## Offering a command to a person

A command says how a person meets it with an optional `ui`
(`oxplow_domain::CommandUi`; agents ignore it; a manifest command's `ui:` — oxplow's own in `extensions/oxplow-foundation`):

| Field | Meaning |
|---|---|
| `label` | what a person reads (`Pull Changes`, `New Task…`) |
| `group` | what search lists it under (`Git`, `Tasks`) |
| `keywords` | more words search matches it by |
| `about` | the resource it acts on (a resource name, `oxplow/task`, `acme/pr`, or a web link's scheme, `https` / `http`, `WEB_SCHEMES`): offered on a URI of it, its page and rows (and on a link to it: a markdown link, a terminal's path or URL, a tab); absent, it needs no URI and **search** offers it. The object form `{ resource, accepts }` narrows it: `store` (the item's store), `url` (a web link's whole URL, a param pattern) or `path` (a file's or directory's worktree path, globs) — [extensions-slots.md](./extensions-slots.md) "The store filter". A command whose `group` is `Open In` joins a link's Open In ▸ |
| `about_when` | with `about`: a column of the dataset its resource resolves through (`ResourceDecl::resolve`, `v_agent_session`) that must be truthy on the ref's own row — not null, 0, `""` or false, a page action's `when:` rule exactly (`viz_actions::when_holds`) — for the command to be offered on that ref. A ref in another state is not offered it: hidden, not greyed (greyed is for `when`, the window's state). For a command whose op refuses some states (`agent_session.reopen` refuses an open session: `about_when: reopenable`); absent, every ref of the kind is offered it. Checked when the extensions load (below) |
| `about_if` | with `about` and an `entry:` script: a function of that script that decides, for each ref (or link) that passes `about.accepts`, whether the command is offered on it — called with `{ uri, kind, url?, path? }` (the URL of a web link, the worktree path of a file or directory), offered when it returns `True`. For logic a pattern can't say. The daemon runs it as the menu opens (`ref_commands::about_if_holds`), as the person, with the services the command `needs` (a `sql.read` answers `scope("sql.read", …)`, so it can read `v_vcs_remote`): the shared script pool, a 1 s budget, every such command of the menu at once, an answer kept 30 s per command, ref and script. A run that fails or runs too long is no, logged. Checked where the command loads (a name; the script defines it) |
| `input` | the input it runs with; a string exactly `{{stream}}` / `{{thread}}` (bound to the stream's / thread's bare id, `str1`), `{{uri}}` (the page's URI) or `{{uri.id}}` (its key; only with an `about`) binds from where it runs, and one with nothing to bind there makes it unavailable there. A binding is the whole string, never spliced into text (`UI_INPUT_BINDINGS`) |
| `form` | what gathers its input (so no `input`): a page's URI (`oxplow:oxplow/page?page=new-task`) or one of the window's own forms (`WINDOW_FORMS`: `new-thread`, the Navigator's inline form; `commit`, the Files page's commit dialog) — choosing it opens that instead of running it; offered about a URI (`about`), a page form opens with `uri=<uri>` (`tabs/pageRefs.withPageParams`) so a `pages:` page form can prefill `defaults: { uri: "{{param.uri}}" }`; a window form takes none |
| `open_after` | a URI to open once it ran, taking only `{{result.<field>}}` from its result (`oxplow:oxplow/page?page=custom-dashboard&id={{result.id}}`, `oxplow:oxplow/agent_session?id={{result.id}}`); a whole `{{result.<field>}}` is allowed when the result field is itself the URI (`{{result.page}}`); bound by the server (RPC `command_open_after`, `CommandUi::open_after_for`; an extension command's `{ result, children }` answer binds from its own `result`) — a field the result lacks opens nothing and says so |
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
(a URI, only `{{result.<field>}}`), `menu.bar` and `about_when` (only
with an `about`, one plain column name) — `Invalid` at
`/ui/<field>`, never a menu entry that silently doesn't work. That
`about_when` is a column of `about`'s resolving dataset needs the loaded
extensions, so it is checked once they are all loaded
(`extension_commands::check_about_when`, an error on the command's extension at
`/ui/about_when` naming the dataset's columns; a resource with no
resolving dataset, a file, has no row to test). Every
template — a page action's `{{param.x}}` / `{{row.x}}`, `ui.input`,
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
  built from the recents list and runs `projects.write` `open`; View ▸
  Sidebar Panels lists the rail's panels, checked when shown — pages-and-tabs.md
  "Left-nav panels");
- **shortcuts** are the offers' `ui.shortcut`s;
- **a ref's menus** — a page's ☰, a page row's ☰ (a table or
  card row, the Board's card) and the Navigator's thread menu — offer the
  commands about its kind (`ui.about`) that apply to it. The **daemon**
  says which: RPC `ref_commands { uri }` (`oxplow_app::commands::ref_commands`)
  returns the ids of the commands whose `about` is the ref's resource
  and whose `about_when` holds on its row, read by the same lookup a
  resource page's `content` loader uses (the resolving dataset's row by
  `uri`; a ref with no row offers none of the conditional commands). The
  desktop asks once, as the menu opens (`useRefOffers`, so a state that
  changed a moment ago is current; the menu opens with the answer), and
  runs no SQL of its own: `refOffers` only binds and runs the listed ids.
  The agent has no per-ref listing (`list_commands` is the index of all
  it may run; the op refuses a state itself), so nothing there changes.

**When a command is offered** (`ui.when`; `oxplow_domain::when`, the
window's `when.ts`). VS Code's when-clause syntax: `!`, `&&` (binding
tighter than `||`), `||`, parentheses; `==` / `!=` against `true`,
`false` or a string (quoted, or a bare word — `pageKind == git-dashboard`);
`=~` against a regular expression literal (`/^work/i`, flags `i` `m`
`s`). A key alone holds when it is `true` or a non-empty string; a key
the window doesn't hold is false, equal to nothing (but `false`) and
unmatched. `true` and `false` alone are the literals: `when: "false"`
is a command never offered on its own (search, the menu bar, a shortcut),
declared with a `ui` for its label and `open_after` because page actions
run it (`oxplow.file.create`). VS Code's `<` `<=` `>` `>=` and `in` / `not in` parse but are
refused as not supported yet: no context key holds a number or a list.
The window publishes its **context keys** (`CONTEXT_KEYS`, the one list):
`agentTerminalLive` (the page shown is an agent session whose pane has a
running process; Restart Terminal reads it, `useLiveTerminals`),
`enabledAgents` (the enabled agent harnesses' keys, comma-joined in
alphabetical order; a command for one matches `enabledAgents =~
/(^|,)claude(,|$)/`), `fileShown`, `pageDirty` (the page shown has unsaved edits: its Save would write them), `pageKind` (the kind of the page shown),
`shellAvailable` (the app, not a browser window), `streamKind`
(`primary` / `worktree`), `streamShown`, `threadShown`, `treeDirty` (the
shown stream's working tree has uncommitted changes: the files the
Git panel counts, `v_change_file` of its `working` change; the
window reads it in `treeDirty.ts`), `vcsEnabled`.
A command's `when` is checked where it registers — it parses, names only
these keys, and compares each to a value of its kind (`true`/`false` for
a boolean key; a string or a regular expression for a string key) — so a
typo is a refusal (`Invalid` at `/ui/when`), never a silently hidden
command; the window only evaluates. Both read the same cases
(`crates/oxplow-domain/fixtures/when_cases.json`). A new key goes in
`CONTEXT_KEYS` and the window's `whenContext` (`window/useWindowCommands.ts`) together.

A page's `open: <column>` action is not a command: it runs nothing on the
bus (no command run, no `command.executed`), so it appears in no command list and
needs no spec ([extensions-pages.md](./extensions-pages.md) "Actions").

Save, Find, Quick Open and New Page with Your Agent are the window's own
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
*capability*: that word is a swappable slot — `snapshots` —
a project picks an implementation of.)
A scope also says whether it **spends** the person's money (`Scope::spends`,
`v_scope.spends`): a model call on their provider key. Access stays about what
a run changes in oxplow's records, so `ai.compute` is `Read`; a tool over a
spending scope (the command needs it, or is its operation) just isn't hinted
read-only (`agent_tools::spends`), so the client asks first.
Every command — core's and any extension's — reaches them the same way,
so oxplow's own commands can do nothing an extension's can't.

Composed children capture their own RunContext during routing, alongside the
retained command handler. A registered contribution factory supplies that child's
identity, package version, generation and declared authority. A command without
a contribution factory gets a core command identity and inherits the caller's
authority in its child context. Nested composers run pure under
their own contribution, transactional leaves use transactional mode, and external
steps use their own context for provider invocation. Cancellation and host live
guards remain inherited. Check authority before composition and again before
each child executes; a later registry replacement cannot relabel a retained
handler. Native scope operations also include their implicit backing scope in
the captured declaration. This prevents namespace services from mistaking the
parent composer's extension for the child command's owner. Each child has its own
run and detail row. Its `parent_run_id` links to the sender's run when that run
is recorded.

The native `state.read/snapshot` operation reads its owning contribution's private
namespace from the command's connection. Its scope/area input contains no owner
selector and its read access leaves no payload in command runs or SQL datasets.
`StateHost` also supplies a trusted write helper for an already-recorded TxCtx,
with live checks and CAS savepoint rollback. There is no public state write
operation yet: private input/result/proposal handling must precede that surface.
See [extensions-manifest.md](./extensions-manifest.md) "Private state host boundary".

| Access | Does | Logs `command.executed` | An agent's run |
|---|---|---|---|
| `view` | changes only what the person sees (open a page) | no | any thread |
| `read` | reads (`sql.read`) | no | any thread |
| `record` | changes oxplow's own records | yes | any thread |
| `write` | changes files, git, processes, the configuration | yes | needs the actor's thread, and that thread open |

**The agent's reads are read scopes.** Every read an agent has beyond
`oxplow_query_sql`'s datasets is an operation of a read scope (`Access::Read`,
`Host::Daemon`, an `External` handler over live state), in
`crates/oxplow-app/src/commands/agent_reads/`, declared in
`oxplow-foundation` as `oxplow.<area>.<verb>` with an `example:`: `records.read/read` (`oxplow.records.read`, a
URI's content), `sql.read/query` (`oxplow.sql.query`), `search.read/search`
(`oxplow.search.find`), `pages.read/open` (`oxplow.page.open`),
`agent_text.read/skill` (`oxplow.skill.read`), `pages.read/run`
(`oxplow.page.run`: the page's text, or its loaders' rows and tree with `format: json`), `pages.read/describe` (`oxplow.page.describe { uri, text?, params?, stream_id? }`: what a page file is made of — `uri` is a file uri, resolved per filesystem: `oxplow:extensions` the catalog's page that file is (a `rev` other than the running deployment's: that deployment's file parsed on its own), `oxplow:temp` a view's file (`parse_inline_page`), `oxplow:local` a draft `parse_page` of `text` (else the worktree file), the extension and slug from `[.oxplow/]extensions/<ext>/pages/<slug>.yaml`; a file that is no page, or no YAML, is a null result = zero rows as a loader, never an error; it replaced `oxplow.page.definition`, with no alias, and has no `spec` input or `yaml` output),
`vcs.read/{diff,blame,log,status,branches}` (`oxplow.vcs.log` pages history
from any revision, past what `v_commit` indexes), `lsp.read/*` (`oxplow.code.*`),
`changes.read/ensure` (`oxplow.change.ensure`: it writes only the derived
change cache, so it stays a read), `knowledge.read` (none today),
`records.read/read` of a model row by an agent (it appends `record.read`
to the log itself — a read's returned events go nowhere — and stays a read;
`.context/models.md` "Reads of rows"),
`extensions.check/{validate,draft_status,review}` (`oxplow.extension.draft_status`: how a stream's source differs from what runs, `extension_deploy::source_diff`; a review clones the
candidate into `.oxplow/tmp`; it changes no record),
`collectors.preview/preview`, and `ai.compute/{decide,summarize}` (recorded
calls whose caller is the asking thread). `pages.write/run_action`
(`oxplow.page.run_action`) is a `record` that composes, running as the
page's actor. A read can run for any thread. The operation
reads the stream it names, else the actor's own
(`agent_reads::stream_or_actors`). The `v_scope` live dataset lists every
scope (id, access, host, summary); `v_command` lists every registered command (the registry `list_commands` reads) with its summary, scope, confirmation and undoability, under the URI `v_contribution` holds for an extension's own. The MCP tools that carried these
reads are thin calls to the commands (`OxplowMcp::read_command`).

A command's access is the strongest among the scopes it needs
(`strongest_access`). A `view` or `read` keeps a detail without input,
result or inverse. A `record` or `write` can keep all three. The
operation's handler decides where it runs — in the bus's transaction (`Tx`) or against a system
it doesn't own (`External`, pinned by `the_external_commands_are_the_reviewed_ones`).
They do not follow each other: `config.write`'s `set` is `External`
(the shared settings service writes the chosen level), while
`providers.sync` and `test_runs.write` are `record` and `External`
(each commits in its own service's transaction).
Two more scopes are services a script uses through builtins rather than
`scope(…)` — `tree.read` and `ai.compute` ([extensions-permissions.md](./extensions-permissions.md)
"Services"). A command's script may read SQL or its owning extension's private state: it
composes, so no model calls, and it decides what runs rather than doing it, so no
tree (`scope_calls::check_command_need`, at load; an effect or a collector reads it). The callable services are
**`sql.read`**, **`config.read`** and **`state.read`**; state takes `{scope, area}` and returns
null or `{stamp, values}` on the routing connection, with identity and live
authority checked by StateHost. It never enters the semantic layer. SQL takes `{ sql, params? }`, one read-only
statement over the published datasets (`v_*`), its `:name`s bound from
`params`, at most `SQL_READ_ROW_CAP` (1000) rows, answered as a list of
row objects, read through the SQL gateway (`SqlGateway::run_in`, on
the run's activation pin) by every script host alike — a command's
composer, an effect's or policy's script, a dry run and a provider's
`host/call` — so `metric_grid()` and `metric_findings()` answer there as
in a page. Each read is its own snapshot; a script's reads don't share
one. More than 1000 result
rows fails with an error; it never hands a script a silently truncated
list. Exactly 1000 rows is allowed. Aggregate in SQL to calculate across
all matching source rows, or explicitly narrow the query with a filter
or `LIMIT` when a subset is intended. A script that wants a large result
a slice at a time asks for it: `page: { limit, offset }` (limit at most
1000) answers `{ rows, more, next_offset?, warning? }`, the `warning`
set when the SQL has no `ORDER BY` (semantic-layer.md "Completeness and
coverage"). The same `scope_calls::Calls`
guard serves commands, effects, dry runs and provider
`host/call`. Fixture `answers` stand in for the scope's answer: a row
list, or the page object when the call asks for a `page`.

Completeness here is **of the query's result**, not collection coverage
or freshness. An explicit `LIMIT` is part of the authored query; the
scope cannot infer whether that subset answers the author's question.
The row-only scope does not expose the gateway's dataset-change timestamps
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
save; `window.show`: find, quick open, `open_url` and `copy_text` (`oxplow.window.open_url { url }`, the person's own browser, and `oxplow.window.copy_text { text }`, their clipboard: `ClientHost::person_op` refuses an agent, so only a person or what they run, an extension's Open In ▸ command, reaches past the window), and `navigate` — `oxplow.window.navigate { uri, stream? }`, open to an agent, takes the person to a URI: the window opens and focuses its page in the thread it belongs to (`apps/desktop/src/tabs/navigation.ts` plans it: a file's editor, a task, request, page, knowledge page, directory or a `page` URI), switches to `stream` first when it is named, and a bare stream or thread URI selects it; it returns `{ opened, stream }`; `agent_input.write`: draft, a
person's) or the **app shell** (`projects.write`: create, open — New /
Open Project; the window reaches the shell, so the window hosts its
handlers too). A shell command isn't recorded: there's no app-scope bus
or log yet — one comes when something there is worth recording or an
agent needs it. Window commands normally have `View` access. Restart
Terminal uses the person's `agent_terminal.write` scope because its
handler calls the PTY restart RPC. Window commands carry their operation
in `CommandSpec::op`.

- **The window's own runs stay in it** — `clientHost.ts` `runLocally`:
  the window does it with its own handler, nothing sent to the daemon.
- **A run on the daemon** (an agent's through MCP, a script's) reaches
  the window through `client_host::ClientHost`: an
  `OxplowEvent::ClientCall { id, client, thread_id, actor, scope, op, input }`
  addressed to one window that hosts the scope — the window of the device
  the run was asked from (its caller; a person's run has no other), else, for
  a run with no caller, the window the person used last
  (`ClientDevices::last_used`, `.context/remote-daemon.md` "Last used"),
  else the last registered — which does it and answers `answer_client_call { id,
  result | error }` (another window's answer is ignored); the run
  waits (15 s; 10 min for a call that waits on a person, such as a folder
  pick, `client_host::PERSON_TIMEOUT`), its answer the run's result. A window that misses the answer
  stops hosting and its event socket is closed (a normal close): a live page
  reconnects and registers again; a frozen one is away until it does. A
  person's run goes only to its own device's window, never another's. A window uses its own id for
  one page load (`uiClientId.ts`, the id its socket, tab reports and logs
  carry; a reload is a new window). The window is the request's own page
  load (its `Oxplow-Connection`,
  `.context/remote-daemon.md` "Who is asking"), and a request with none is
  refused. An agent's `oxplow.window.navigate` and `oxplow.page.show` may
  name a `client` (a device id or a connected device's name): the call goes to that device's window (`ClientHost::call_on`),
  or waits for it when it is away (`.context/remote-daemon.md` "An agent's
  tab on a device"). It says it's open — and what it hosts — with
  `register_client_host { scopes }` once it is ready to do a
  call as asked (`window/useWindowHost.ts` `windowReady`: its stream and thread loaded,
  the thread's first tabs opened, the extensions loaded; before that a
  navigate was refused, pushed aside by those first tabs, or drawn by
  nothing). Once ready it stays the host, and `<html data-oxplow-ready>`
  says so to whoever waits for the window (the browser suite). It
  registers again on reconnect,
  and `unregister_client_host` when it closes (`pagehide`), which refuses
  its unanswered calls at once. With no window, one that doesn't host it,
  closes or doesn't answer, the run is `Unavailable`; a window that
  didn't answer is forgotten until it registers again, so the next call
  is refused at once rather than waiting out another timeout.
- **What the window runs to answer a call runs as the caller.** A
  window handler that writes through the daemon — Save writing
  `oxplow.file.save` — calls `run_command_for_call { call, id,
  input }`, which runs as the waiting call's actor (`ClientHost::caller`;
  only the window the call went to, only while it waits), never as the
  person at the window: an agent's Save is the agent's write, under its
  write gate and in its own stream (`file.save` refuses anyone but a
  person outside their own stream), recorded to it.
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
commands are staged and activated with each generation like any
extension's, from its active deployment (the shipped one, or one deployed
while developing oxplow), so deploying it with a new command registers
that command at once; nothing runs a command before the first activation
completes. Each runs as its extension's contribution (an
extension-scoped operation — configuration, state, jobs — finds its owner
that way), so foundation's command runs are `oxplow-foundation`'s
`v_contribution_run` rows like any extension's. An extension may declare a command over the same operation in
its own namespace. A script reaches record and write operations only by
composing commands.

**An operation bounds only confirmation.** Anyone may run any
command: a person, an agent, or a page acting for either. There is no
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
([extensions-permissions.md](./extensions-permissions.md) "The run context") whose trace counts
its scope calls. A `Tx` run's attempt keeps its own (`TxCtx::trace`,
fresh per attempt, shared by the runs nested in it). The routing pass
composes a composite once, through the gateway, as a pure child of the run,
and what that child called becomes the plan's scopes, which the run
records; steps add each step's. An `External` call uses the trace on
`Invocation::run`: a provider's
`host/call`s during it count there — its own command's (`run_op`) and a
capability verb's, which the `task.<verb>` run hands its verbs in
`VerbCall::run` ([providers.md](./providers.md) "The protocol"). Its
summary — per scope, how many calls (`{"sql.read": 2}`) — is written on
the shared run row (`contribution_run.scopes_json`), along with permission
decisions. Later: each call's detail (what it touched),
sampled or on by setting, and approvals by scope.

## The pipeline (`crates/oxplow-app/src/commands/mod.rs`)

The bus is split by concern: `mod.rs` the types, the registry and the
pipeline (`run_prepared`, the read path); `record.rs` a
run's command run row and events (`record_tx`, `record_unapplied`,
`record_external`, the transaction's `Abort`); `undo.rs` undo and the
claims that keep two undos or approvals from both applying;
`proposals.rs` proposals (`unconfirmed`, the dry run, approve,
decline); `steps.rs` composition — the composed tree, routing, steps;
`ops.rs` the scopes' operations; `util.rs` what handlers
share; `tests.rs` the bus's own tests.

`CommandBus::run(actor, name, input, confirmed)`:

First the **actor is resolved**, once (`CommandBus::resolved`): an agent
— or a page acting for one — whose transport carried its thread but not
its stream runs with its thread's stream, so "the caller's stream" is
`actor.stream_id()` in every handler, one answer; an agent claiming a
thread that doesn't exist is `Denied` (recorded). An agent with no thread
has no stream, and a handler's own-stream check refuses it rather than
skipping.

0. **Offered** — what's active now (`with_capabilities`: the capability
   registry and the config, `capabilities::Active::refusal`) must meet the
   spec's `needs`, and a command an implementation owns runs only while
   that one is active: a provider instance's namespace (`<instance>.*`).
   Refused
   → `Invalid` naming what it needs (recorded), before validation, for
   every actor; `list` leaves it out the same way.
1. **Validate** the input against the schema → `Invalid { field, message }`
   (recorded as `invalid`).
2. **A job's tools** — a job (or a page it pressed) runs only a command
   that says `job: true` → `Denied` (recorded; `job_refusal`). Anyone else
   may run every command.
3. **Agent policy** — for a thread-gated run (an agent, an effect, a
   job, or a page acting for one, nested runs included),
   `AgentPolicy::check_command(thread, spec, may_write)`
   (`agent_policy.rs`) → `Denied` (recorded). A `Write`
   command needs an open thread: the bus asks its write gate
   (`Services` wires `Thread::is_open()`), so a closed thread can read
   but not change state. A `Record` command skips that refusal: oxplow's own records
   (tasks, notes) aren't a claim on the worktree.
   **Pre-check**: a command may carry a
   `Precheck` (`Command::with_precheck`) — an async check of the input
   that needs more than a transaction can do (resolving a metric query
   through the metric engine). It runs here, once the actor is admitted
   and before any transaction: the command's own for a direct call; for a
   composite, every composed call's, collected while routing
   (`steps.rs` `Router`), its refusal placed at `/calls/<i>/input…`. A
   refusal is recorded `invalid` and nothing runs.
4. **Confirmation** — what a run asks is `commands::asks`: the command's
   `confirm` (or its per-input `confirm_for`), except an agent-driven run
   (an agent, or a page acting for one), which asks nothing — what an
   agent may call is its harness's to decide. When it asks and the call
   isn't `confirmed`, **nothing runs and no command run row is written**: a
   person (or the system) gets `NeedsConfirmation { preview }`; an
   effect's run is kept as a **proposal** and gets `Proposed { proposal,
   preview }` (see "Proposals", P6b.A3). `confirmed` stays a person's
   word (`Actor::may_confirm`): an agent's run is never confirmed, so a
   handler's own consent check still asks for a person. The answer
   rides into the handler as `TxCtx::confirmed` (and step 3's gate as
   `TxCtx::may_write`, "the thread is open"): a handler that learns only while running that a
   confirmation is needed — a composite whose child asks — raises
   `NeedsConfirmation` itself, and the bus treats it as this step would
   (rolled back, nothing recorded; a person asked, an agent's run
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
5. **Run and record.** The bus writes a `contribution_run` row and a
   `command_run` detail row linked by `run_id`
   (`command_run_store::insert_tx`). The shared row holds the actor,
   anchors, timing, outcome, error, cost and authority. The detail row
   holds the command's input, result, inverse, event link and undo marker,
   and the device a person asked from (`device_id`): a run's context carries
   the request's caller (`RunContext::with_caller`; its children inherit it),
   and the RPC commands that run as the person (`run_command`, `undo_command`,
   `decide_proposal`, page actions and forms, component commands) set it
   from the request (`.context/remote-daemon.md` "Who is asking"). An agent's,
   an effect's or a job's run has none.
   For a `Tx` write, the handler, both rows, `command.executed@6` and the
   handler's effects commit together. The event's `command_run_id` names the
   detail row's integer ID. The handler's events take the executed event
   as their cause and inherit any missing thread and stream anchors.
   A handler failure rolls back its writes and gets its own record with
   outcome `failed`. Refused input and authority use `invalid` and
   `denied`.
6. Post-commit: wake the event pump.

Plain reads and unchanged calls still get a run and a detail row
(`ran_without_event`). Their `input_json`, `result_json` and `inverse_json`
are NULL. They emit no `command.executed` and return no `command_run_id` or
`event_id`. A `Tx` read runs in a snapshot that rolls back. A call that
returns `HandlerOutput::unchanged` also rolls back its transaction, then
the bus records its run separately. The bus rejects an unchanged result
that also carries events or an inverse. Undo, approval and effect
reactions keep their detail and event links even when nothing changes.
An external read also receives the command's run ID, so model calls can
link to it.

**Retention.** The sweep removes refused runs (`invalid` or `denied`)
after 30 days (`REFUSED_COMMAND_RUN_DAYS`). Failed runs and reads age out after
the 90-day `CONTRIBUTION_RUN_DAYS` window. Successful runs with recorded
input stay for undo, approvals and event links. Deleting a shared run
cascades to its command detail.

**Reading it.** `v_command_run` joins the command detail to the shared
run and exposes every recorded command, including reads and unchanged
calls. `changed = 1` means the run succeeded and recorded its input. The Command data
page filters on that field and a row opens Run details. The bundled
Agent Interface page counts completed agent runs, including reads, over the
selected date range and harness. It reports invalid, denied and failed runs
separately, and its Not OK rate includes every recorded outcome other than
`ok`. Command Errors groups recurring messages and links to shared Run details.

A command's run records the extension whose manifest declares it (through
its contribution, as `v_command` lists it); a native command no extension
declares records `core`, whatever its id. An external handler receives the
command's own run context so model calls and child operations can link to it.

`CommandBus::undo(actor, command_run_id, confirmed)` loads the command detail and its shared run. It refuses a
run that didn't complete, was already undone or has no inverse. It runs the
inverse through the same pipeline and sets `undone_by` to the new detail
row's ID. The original row is claimed **with** the inverse run
— marked `undone_by` inside the run's transaction for a `Tx` inverse,
or claimed before an `External` inverse and released if it fails. The
`UNDO_PENDING = 0` sentinel reserves `undone_by` before the inverse has
a command run ID. Those IDs start at 1. A concurrent undo that loses the
claim returns `Invalid` ("command run N was already undone", `lost_race`)
without recording a run. Approval claims use the same approach.

**Agent rules follow the agent.** An `Actor::Page { on_behalf_of }` —
or an `Actor::Component { component, on_behalf_of }`, a custom component
acting for whoever uses it ("Custom components") — whose chain ends at an
agent (`Actor::is_agent_driven`) gets the agent policy and can never
confirm, exactly like the agent: its run asks nothing. **A page grants
no power:** it acts with the rights of whoever pressed it (`Actor::presser`).
Its run of a command that asks is asked first when a person pressed it,
and runs without asking when an agent pressed it.

**Only a person confirms; an effect runs with an agent's rights**
(P8.D8). `Actor::may_confirm()` is true for `Human` and a page acting for
one — never an agent, an effect or `System` (step 4 drops a `confirmed`
from anyone else). An agent's run asks nothing. An
`Actor::Effect { effect: "<namespace>/<id>", thread_id, stream_id }`'s
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
`effect:<namespace>/<id>`, and only a
`Human` approves or declines a proposal. The command run and events say so:
`actor_kind = 'effect'`, carried by `command.executed@6` (whose
`ActorKind` also has `job`) and `command.proposed@4` (whose `ActorKindV2`
has no `job`: a job can't leave a proposal). V68 updates the stored command-executed payloads. An effect's reaction
runs
through `CommandBus::run_effect` (P8.D10) with `RunOrigin::Effect`, the
fourth origin beside a call, an undo and an approval: like those, its
effect attempt and shared run finish in the same transaction as its
`effect.result@5`. Before an external step, `claim` starts the shared
row and inserts the attempt together. A reaction awaiting a proposal
decision ends as `skipped` and records the proposal ID. Its
`command.executed` is caused by the triggering event. [Effects](./extensions-events.md)

**An `External` run whose recording fails** (its effects already
committed in the service's own transaction) is reported as done and
unrecorded — `command_run_id: None`, logged at error level — never as an
error, which would claim the change didn't happen.

A command whose confirmation depends on the input sets
`Command::with_confirm_for(fn(&input) -> Confirm)`; the bus consults it
in step 4. A handler that needs the UI to refetch returns
`HandlerOutput::after_commit`, which the bus runs once the transaction
has committed — the in-memory `OxplowEvent` broadcast belongs there,
never inside the handler.

**A `Tx` handler must be pure**: it can run more than once,
because `Database::transaction` retries its closure on SQLITE_BUSY. Only
database writes on the given connection belong inside; anything outside
the database (a file, in-memory state) goes in `after_commit`, which must
tolerate failure since the run is already recorded. `oxplow.config.set` is the
model: the handler validates and computes before/after; `after_commit`
re-applies the key to the config as it is then, writes project.yaml and
swaps memory under one lock, so concurrent sets of different keys both
survive.

## Composition: `oxplow.command.sequence`, the composed tree and steps (P6b.A1)

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
command stays in the transaction; an `External` one leaves it; a nested
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
inside → **one transaction**, below; any outside →
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
itself?") instead of recursing until the stack overflows. Each child has its own run and detail row in the transaction. The parent
owns `command.executed` and the combined inverse. Its `result` is `{ result, children: [{
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
of its own, is `unchanged` itself and records a detail without input, result or inverse. A child's `Busy`
propagates and the bus retries the whole parent (handlers are pure).
A `Tx` handler doesn't compose by hand: composing is a composite's, so
it reads through the gateway outside the transaction.

**As steps** (P7 review; `CommandBus::run_steps`): a composite
with a call outside the transaction can't be all-or-nothing, so its
calls run as steps. (1) **Checked first** (routed as above, then): every
step's — and every composite step's calls' — own agent
policy and `confirm`, before anything runs — a refusal is the run's, recorded, and
nothing ran; a step that asks, or a composite step whose call asks, makes
the run ask once (`NeedsConfirmation` with the calls in
the preview), and an effect's run is a proposal **with no dry run**
(nothing outside the transaction runs before a person decides). (2)
**In order**, each landing as it runs: a step inside oxplow in its own
transaction (its events caused by the run's `command.executed`, whose id
is fixed first), an external one through its system; the first failure
stops the run, and the steps before it **stand**. (3) **Recorded with their parent**:
each step has its own run and detail. The parent owns `command.executed` with `result { result,
children: [the landed steps], failed?: { name, input, error } }` and the
landed steps' events, caused by it. A run that stopped partway is
recorded as `failed` with what landed, and returns `Failed` naming the
failed step and the ones that stand; a run that failed at its first
step is recorded like any failure. **Not undoable by itself** (no
inverse of its own): a system outside the bus can't be rolled back with
it, so a person reverses a landed step by hand (each child keeps its own
`inverse` in the result) — unless the composer **said** how:
`Composition.inverse` (an extension script's `"inverse": { "commands":
[{ name, input }] }`) is checked while routing and, when every step
landed, is the run's inverse — recorded in the command run row and returned
beside the result, the same place and the same `undo` as any other. An
undo runs it through the bus as a run of its own (its policy, confirmation
and command run; as the person or agent who undoes, not the extension). **Checked
at routing, before anything runs** (`Router::route_inverse`): each call
names a registered command that writes, the composing contribution may
compose it (`commands:` permission), its needs are met and its input fits;
the host's own-script-only writes (`oxplow.extension_store.write`,
`oxplow.extension_files.write` / `delete`) are refused (an undo is not the
extension's script — name the extension's own command that makes the write);
several calls become an `oxplow.command.sequence`, so none may itself compose
(a composite whose steps leave the transaction can't be a step of one),
and a single call stays as it is. **A composition that stays in the
transaction takes no `inverse`**: it undoes itself (the reversed children),
so a script that returns both is refused at routing, naming the command —
silently preferring one would hide the other. A run that stops partway
keeps no inverse.

**`finish`** (`Compose.finish`, a `Finisher`): a composer may also have a
function that runs after every call landed, with each call's answer
(`StepAnswer { name, input, result }`) — an extension script's `finish`
(extensions-commands.md). In the transaction it runs in `steps::run_tree`
after the children, still inside it (`TxCtx.run`'s pure child; no reads: the
transaction holds the database), so its refusal rolls the run back; as steps it
runs in `CommandBus::finish_steps` (`spawn_blocking`, reading through the
`Reads` the routed `Composed` keeps), and its `inverse` goes through
`route_inverse` — the same check as routing's — before the run is recorded; a
failure there is recorded as an error with the steps that landed (they stand).

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
   failure (recorded like one), not a proposal.
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
   for the person in Alerts, and on the setting's
   row; "It replaces proposal:M" when it did; tell the person; don't run
   it again; its `decision` is in `v_command_proposal`). Other MCP
   tools that run a command report the same message as an error. The
   desktop shows a thread's pending proposals where the conversation is
   (P9.A3: the agent session page's `approvals` section above its
   terminal, narrowed to the thread; usability.md
   "Agent proposals").

**Deciding** is a person's (`Actor::Human`; an agent, a page or the
system is `Denied`): a proposal is consent to code that isn't theirs or
an effect's run, the one place a person, not their agent, answers. A
proposal is decided once (`Invalid` after):

- **`CommandBus::approve(actor, id)`** runs the proposal's command as the
  person, confirmed, through the whole pipeline (`RunOrigin::Approval`).
  A `Tx` run marks the row approved with its `command_run_id` and logs
  `command.approved@3 { proposal, command, command_run_id }` (caused by its
  `command.executed`) in the run's own transaction; an `External` run
  claims the row first (approved, no command run row: `claim_tx`), names its
  command run row when recorded (`finish_claim_tx`) and releases the claim if
  it fails (`release_claim_tx`) — the same pattern as an `External`
  undo's claim. A run that fails leaves the proposal pending (the window
  records the person's failed approval as an op error and keeps it on the
  card; its pending read keeps a claimed row, so the card isn't dropped
  mid-run); a concurrent approval that loses answers `Invalid`
  ("proposal:N was already decided") with no command run row.
- **`CommandBus::decline(actor, id)`** marks it declined and logs
  `command.declined@2`; nothing runs, no command run row.

A custom component's frame invokes a command through UI RPC
`invoke_component_command` — as the page acting for the person, only a
command its component declares ([extensions-components.md](./extensions-components.md),
"Custom components").

UI RPC **`decide_proposal { proposal, approve }`** (`ui` in surface
parity; desktop `decideProposal`) returns the approving run's outcome, or
`null` for a decline. A proposal's dry run is a snapshot at proposal
time; the approval re-runs for real and the command run row records what
happened.

## `config.*` and the key registry

Core and extensions share `oxplow_config::settings::SettingsRegistry` and
`SettingsStore`; [settings.md](./settings.md) is the contract. Core definitions
are generated by `core_settings`, and extension `configuration.schema` leaves
are registered by `extension_settings`. A key is fully qualified, such as
`oxplow.snapshots.retentionDays`, `oxplow.testing.command` or
`oxtask.tasks.workflow`. Fixed object fields become independent settings;
arrays and open maps are atomic values. Categories do not depend on ownership.

`oxplow.config.get { key, level? }` returns `{key, value, origin, isSet, secret}`.
Without `level` it resolves local → project → global → default. An explicit
level reads only that override. `oxplow.config.list_keys {}` returns registry
definitions with title, description, category, schema, default, secret and UI
metadata. Every setting supports all three levels.

`oxplow.config.set { key, value, level }` and
`oxplow.config.unset { key, level }` require an explicit `local`, `project` or
`global` level. YAML overrides are flat maps of fully qualified keys in
`.oxplow/local.yaml` (ignored), `.oxplow/project.yaml` (shared), and
`global_config_dir()/global.yaml` (`OXPLOW_HOME` applies). Writes validate each
changed value by its registered schema and its own core rule
(`core_settings::check`), then build the typed config the files would make
(`effective_values` + `config_from`, once) before mutation, so a write never
leaves the project unloadable; they preserve other keys and comments and
replace files atomically. Unsetting removes only that
level's override. There is no personal-only key list, nested legacy format,
legacy settings-file fallback or automatic migration.

Secret definitions route through the same service to secure storage. Local and
project keychain accounts include the project identity; global accounts do not.
No secret bytes enter YAML, command inputs/results in command runs, events or undo
payloads. Secret defaults are prohibited. Secret writes have no plaintext undo
payload; ordinary writes restore or unset the chosen override through canonical
`config.*` calls. Secure-store errors never fall back to files.

`scope("config.read", {key, level?})` gives a command script an authorized read
of the same service. A script composes `oxplow.config.set` per independent leaf;
it does not rewrite an extension's whole configuration or use revision stamps.
Pages and agents read settings from `v_config_value`, and SQL from
`config('/pointer')`. Extension private runtime state still uses `state.read` and
`state.write/compare_exchange`; settings do not use the state protocol.

**Settings is a registry inventory** (`effective_config.rs`, live dataset
`v_config_value` v5, desktop `viz/settingsModel.ts`). Each row has the fully qualified
key, redacted effective value, origin, owner, extension, title, category,
description (`doc`), schema, secret presence, restart requirement and UI hints.
Ordinary values have explicit local/project/global columns independent of the
effective origin; unavailable reads retain the row with unknown value/origin.
Metric and dimension definitions are not preferences and do not appear here.
The existing Settings areas keep their specialized controls; the overview groups
by registry category and searches key, title, description and public values.

**Backend reactions** use canonical keys in `config.changed`: workspace filters
watch `oxplow.generated.*`, provider reconciliation watches
`oxplow.extensionInstances` and `oxplow.activeProviders.*`, extension activation
watches extension definitions and enablement, and metric retention watches its
registered keys. The renderer's `ConfigChanged` refreshes live setting reads.

`CommandBus::list(actor)` is the specs that actor may run —
`list_commands` for the agent, the launcher for the human.

## `Tx` and `External`

A `Tx` handler is `Fn(&TxCtx, Value) -> HandlerOutput` and composes
into the bus's transaction, so its writes, the command run and the events
commit or roll back together — the target shape. `TxCtx { conn, actor,
events }` carries an `EventCtx` whose `source` is the actor's and whose
`cause` is the run's `command.executed` id — fixed before the handler
runs — so store cores the handler calls (`set_status_tx`, the task
stores' `_tx` cores) log their own events as this run's. Those
events precede `command.executed` in `seq` (it's appended after the
handler, with the command run); `HandlerOutput.events` follow it. A handler
must stay pure: `Database::transaction` retries it on SQLITE_BUSY — and
a handler that hits one itself returns `CommandError::Busy` (the
`From<DomainError::Busy>`; map SQLite errors with `oxplow_db::map_sql_err`),
which the bus turns back into a retry; only a busy that outlasts the
retries reaches the caller, as `Busy` (RPC `BUSY`). A system outside
oxplow that couldn't answer just now (an external provider's `Internal`, a timeout,
a rate limit, with its wait) is `Unavailable { message, retry_after_ms }`
(RPC `UNAVAILABLE`) — the one failure an effect sends again by itself;
everything else that fails is `Failed`. An **`External`**
handler (P5.A1) is an async call against a system the bus doesn't own —
a VCS, an external provider's process, a collector script, the page files on disk —
whose state can't join the bus's transaction; the bus records it after it returns (a failure to
record is logged, never reported as the run failing); its command run row
holds the handler's `result` like a `Tx` run's. It is
`Fn(Invocation, Value)`: `Invocation { run, idempotency_key }` (its
actor is `run.actor()`; the run is the command's), the key
set for a step of an effect's reaction (`effect_step_key`:
`effect:<effect>:<event id>:<index>:<hash of the call>`, the same on
every attempt at it — P10) and `None` otherwise; an external provider's
handlers pass it to the provider process ([providers.md](./providers.md)
"Idempotency"). A keyed step's events take dedupe keys from it
(`<key>:event:<i>`, `same_events_once`) and `record_tx` appends handler
events uniquely, so a step that landed and is re-sent on a retry — its
system answering again, events included — logs them once. It is the right
kind for exactly those commands, not a shortcut: `CommandBus::
external_commands()` is pinned by `the_external_commands_are_the_reviewed_ones`,
and adding one means naming its system in the summary. Registering a handler whose kind
disagrees with the spec's `atomicity` is refused, as is a second command
of the same name.

**A run's projections are delivered before it returns.** Once a run
lands (one transaction, an outside call recorded, or a composite's
steps), `CommandBus::landed` runs the event pump's sync consumers — the
projections: the page refs —
before the caller gets the outcome (`EventPump::deliver_projections`),
then wakes the pump's loops. So whoever ran a write reads it back at
once, whichever list holds it.

## Commands so far

Each `oxplow.*` row below is declared in `extensions/oxplow-foundation`
(or, where it says so, `oxplow-bundled`) over an operation of the scope
its area falls under (`threads.write`, `vcs.write`, …; `SCOPES`); "Handler" describes that operation.

| Command | Handler | Notes |
|---|---|---|
| `<extension namespace>.<name>` (an enabled extension's `commands:`) | `Dispatch`: the extension's Starlark script composes core commands — once, when the bus routes the run — run as any composite's (`extension_commands.rs`, P6b.B2; "Composition") | declared confirm / access, undoable, `Experimental`; registered while the extension is enabled (primary worktree). See [extensions-commands.md](./extensions-commands.md) → "Commands" |
| *(tasks)* | — | Core has no task commands: a task store's commands are its extension's (`oxtask.task.*`) and join bundled's task points (`.context/tasks.md`). |
| `oxplow.command.sequence { calls: [{ name, input }] }` | `Dispatch` — a composite (`commands/compose.rs`, P6b.A1) | `Write`, `Confirm::Never` — the children decide. Each child's own policy and confirmation; each command has its own run and detail. The parent's `result` lists its children. Every call in oxplow's records → one transaction (`steps::run_composed`), undoable as the reversed children that changed something; a call outside it (a `task.*` verb, any list's) → steps, in order, not undoable. The one composition mechanism (an extension's command runs on it). See "Composition" |
| `oxplow.test.record_run { thread?, command, duration_ms?, passed?, failed?, total? }` | `External` over the `CollectionService` (`commands/test_runs.rs`): the capture commits in the collector's own transaction | `Record`, not undoable. An agent's goes on its own thread whatever it names; a person names one. The `asserted` run whose counts oxplow couldn't parse; it belongs to the request of the turn that reported it. A report oxplow parses itself is a report collector's, run by hand with `oxplow.collector.sync`. See [collection.md](./collection.md) |
| `oxplow.extension.write_page { extension, page, spec, deploy? }` | `External` (`commands/extension_deploy.rs`, scope `pages.author`): writes `pages/<page>.yaml` (the YAML text in `spec`) into the extension's source in the primary checkout, scaffolding the manifest when the extension is new; `spec` must parse (`pages::schema::parse_page`) and the extension must load with it, else nothing stays written; a bundled extension and a caller in another stream (in-project source) are refused; `deploy: true` then deploys | `Write`, not undoable. Returns `{ page, path, deployed }`. What the app's Save as Page and dashboard edits call |
| `oxplow.extension.deploy { name }` (from the primary checkout; refused from another stream when the source is inside the project), `rollback { name, to? }`, `enable { name }`, `disable { name }`, `remove { name }`, `share { name }`, `unshare { name }` | `External` (`commands/extension_deploy.rs`): copies a source into `.oxplow/runtime/extensions/<name>/<hash>/`, publishes the live set, and edits `.oxplow/local.yaml`, `.oxplow/project.yaml` and `.oxplow/.gitignore` | `Write`, not undoable. `deploy` returns once the deployment runs; it fails, keeping the old one, when the extension does not prepare. `enable` / `disable` write a local `oxplow.extensions.<name>.enabled` boolean and are what the Extensions page calls; `remove` forgets deployments, state and settings and never deletes the source; `share` / `unshare` un-ignore a source and write or remove its project enabled override. See [extensions-deploy.md](./extensions-deploy.md) → "Sources and deployments" |
| `oxplow.snapshot.restore_file { file_snapshot }` | `External` over `snapshot_files::SnapshotFiles` (`commands/snapshot.rs`, P8.A9) | `Write`, `Confirm::Destructive` (it overwrites the file at its path), not undoable: a person's run asks first. Writes into the row's stream's worktree — refused when that stream is archived or gone, never written into another checkout — and announces the stream's `WorkspaceChanged` |
| `oxplow.lsp.install_server { package }`, `oxplow.lsp.remove_server { package }` | `External` over the `LspInstallerService` (`commands/lsp.rs`, P8.A9): the network and `.oxplow/lsp/` | `Write`, `Confirm::Always`, not undoable: a person's run asks first. Each pushes `LspServersChanged` (the server list isn't a model). See [lsp.md](./lsp.md) |
| `oxplow.agent_session.open { thread, kind?, harness?, title?, resume? }`, `oxplow.agent_session.rename { session, title }`, `oxplow.agent_session.close { session }` | `Tx` over `agent_session_store` and `agent_stores::close_session_tx` (`commands/agent_session.rs`) | `Record` (`agent_sessions.write`). **The command is the slot's lifecycle; the RPC is the process's.** `open` only inserts the row — it never starts a process and never prompts (the UI starts the process when the tab mounts; nothing types into it, which the no-automation guards keep true for a session an agent opened); `harness` is checked against the registered harnesses and the project's enabled ones, the default is `HarnessRegistry::default`, `kind` defaults to `terminal`, and `action` is refused (not built). `resume` (a saved conversation's native id) binds the new slot to it as `chosen` in the same transaction — refused for a conversation open in another slot, and denied to an agent, which may open a session but not choose its conversation. Undone by closing it. `rename` undoes to the old title. `close` asks no one and is undone by **`oxplow.agent_session.reopen { session }`**: the slot closes, its open turns end interrupted and it logs `stopped` in one transaction, and its process stops after commit (`agent_sessions::SessionProcesses::kill`); `reopen` opens a slot a person closed again (refused for one its thread's or stream's close took, or on a closed thread), and its process resumes the saved conversation when its tab mounts. An agent acts only on sessions in its own stream. `terminate_terminal_session` / `chat_close_session` stop a process and leave the slot open. Sessions are named by URI (`oxplow:oxplow/agent_session?id=ses3`). **`start_fresh`** is a session's one control besides closing it (a session has no stop that keeps its slot: closing ends it, Earlier sessions reopens it): it forgets the saved conversation, the only way a session gets a new one — `Confirm::Destructive`, not undoable |
| `oxplow.thread.create { stream, title, from? }`, `oxplow.thread.rename { thread, title }`, `oxplow.thread.set_prompt { thread, prompt? }`, `oxplow.thread.mark { thread, mark? }`, `oxplow.thread.close { thread }`, `oxplow.thread.reopen { thread }` | `Tx` over `thread_store::{get,list_for_stream,upsert}_tx` (`commands/thread.rs`, P8.A3) | `Record`, `Confirm::Never`. The thread state machine: a thread is open or closed, nothing more (`threads.closed_at`); a new thread is open at the end of the stream's order, with no agent session (`from: thread:<id>` forks it on the same stream, copying nothing agent-ish); `mark` sets or clears (`mark` absent) a colour on the thread's Navigator tab — `amber`, `red`, `green`, `blue` or `violet` — for grouping threads the person's way, changing nothing else (undo sets the previous mark back); `close` ↔ `reopen` undo each other and a close closes its agent sessions (`thread_closed`) in the same transaction and stops their processes after commit; `rename` / `set_prompt` / `reorder` undo to what they were (`reorder`'s `order` names every thread of the stream once, so its undo puts each back); `create` isn't undoable; a move to the state a thread is already in (marking what it already is, closing a closed thread) is `unchanged` and leaves no record. Threads and streams are named by URI (`oxplow:oxplow/thread?id=thr12`, `oxplow:oxplow/stream?id=str1`). An agent acts only on its own stream and closes and reopens only its own thread; `set_prompt`, `mark` and `reorder` are the caller's (a prompt steers an agent); a prompt empty after trimming clears it. Every open thread of a stream shares its worktree and any may change it. The desktop's thread helpers run these through `run_command`; the views re-read on `DatasetsChanged` naming `v_thread` |
| `oxplow.stream.create { title, source, existing_branch?, new_branch?, start_point?, worktree? }`, `oxplow.stream.archive { stream, delete_worktree? }` | `External` over `StreamService` (`commands/stream.rs`, P8.A4) | `create` (agents set up parallel work on a branch — `source: existing` (a branch that exists) or `new` (forked from `start_point`); `source: worktree` adopts a worktree already on disk, and is refused unless the path is one of the repository's worktrees: "{path} isn't one of this repository's worktrees"); the worktree's folder is made from the title (lowercase words, hyphens), so the caller names none; `archive`; `Write`, not undoable. Create reaches the VCS and starts the stream's capture service, and its `ui.open_after` is the new stream (the New Stream page gives way to it); archive is `Confirm::Destructive` (the desktop's Remove dialog is the confirmation), refused for the primary or an unknown stream before anything changes (`StreamService::archivable`), refused while an agent runs in one of the stream's threads, takes its threads with it, purges its search rows, forgets its worktree route and stops its capture, and removes its working copy when `delete_worktree`. A worktree that vanishes is archived by the workspace watcher directly — an observation, not an intent |
| `oxplow.stream.rename { stream, title }`, `oxplow.stream.set_prompt { stream, prompt? }` | `Tx` over `stream_store::{get,upsert}_tx` (P8.A4) | `Record`, undoable to what they were. An agent renames only its own stream; the prompt (appended to every agent prompt in the stream) is the caller's to set; one that is empty after trimming clears it. The stream list re-reads on `DatasetsChanged` naming `v_stream` |
| `oxplow.hint.dismiss` (`Tx`) | `commands/hint.rs` over `agent_nudge_store::dismiss_tx` | `Record`: dismisses a hint raised to the person (`v_agent_nudge`, audience `person`), stamping its `delivered_at`; dismissing one already dismissed is `unchanged` (no record), one never raised `Invalid`. Not undoable |
| `oxplow.hint.dismiss_all {}` (`Tx`) | `commands/hint.rs` over `agent_nudge_store::dismiss_all_tx` | `Record`: Clear notices — dismisses every hint still raised to the person, never an agent's; the result says how many, and with none it is `unchanged`. Not undoable |
| `oxplow.context.add { thread \| basket, stream?, uri?, label?, text?, around?, note?, held? }`, `oxplow.context.note { item, note? }`, `oxplow.context.remove { item }`, `oxplow.context.move { item, thread \| basket, held? }`, `oxplow.context.clear { thread \| basket }`, `oxplow.context.send_next { thread, basket?, hold? }`, `oxplow.context.put_back { thread }`, `oxplow.context.create_basket { name }`, `oxplow.context.rename_basket { basket, name }`, `oxplow.context.delete_basket { basket }` (`Tx`) | `commands/context_basket.rs` over `context_store::*_tx` | `Record` (scope `context.write`): the person's context baskets (`agent-model.md` "The context basket"). A named basket is given by its name, which may not start with `@`. An agent's run is refused. `held` (an item in a thread's own basket only) holds it out of the next prompt, as Put Back leaves what it returns to This Thread; `send_next` with no `basket` releases the thread's held items, and its `hold` holds those items again instead (undo). `add` to a basket that isn't there makes it and undoes to `delete_basket`, else to `remove`; `note` to the note before; `remove` to an `add` of the same content to the same basket (stream, note and hold too); `move` back (held again when it was; a move drops the hold); `clear` to a sequence of `add`s; `put_back` (refused with no last send; the one an agent may run, for its own thread only) to `remove`s of what it put back; `send_next` to the thread's choice before with `hold` of what it released (and, when the basket was another thread's next, that too); `rename_basket` back; `delete_basket` to `create_basket` + its items' `add`s + its `send_next` |
| `oxplow.bookmark.set`, `oxplow.bookmark.remove` (`Tx`) | `commands/bookmark.rs` over `bookmark_store::*_tx` | `Record` (bookmarks are the person's navigation). The viewer is `thread` (its stream implied) or `stream`; `set` moves a page's bookmark to its `visibility_scope`. Both undo to what the viewer saw before |
| `oxplow.terminal.open { stream, title?, dir? }`, `oxplow.terminal.rename { terminal, title }` (`Tx`); `oxplow.terminal.close { terminal }` (`External`) | `commands/terminal.rs` over `terminal_store::*_tx` and the PTY registry (scope `terminals.write`) | Opening and renaming run for a person, a page or an agent (an agent only in its own stream); `open` answers `{ id, uri, title }` and titles "Terminal N" when none is given; `rename` undoes. `close` ends the shell the registry keeps under the terminal's URI, then deletes the row — a person's (an agent is refused), `Confirm::Destructive`, the last one may close. Events `terminal.opened@1` / `renamed@1` / `closed@1`. [terminal.md](./terminal.md) "Terminals are rows" |
| `oxplow.config.list_keys {}` / `oxplow.config.get { key }` | `Tx` (read-only) over the key registry (`commands/config_commands.rs`) | every `.oxplow/project.yaml` key with doc, value schema, current value |
| `oxplow.config.set { key, value, level }` / `oxplow.config.unset { key, level }` | `External` (`settings_commands::write_changes`, the one settings write): validate the registry schema and the core rule of each changed key, skip a key whose stored value is the same, write the explicit local/project/global override through `SettingsStore`, refresh runtime configuration | Ordinary values are undoable at the same level; secret values are redacted from command runs/events and have no plaintext inverse. Emits `config.changed` with a canonical key and `ConfigChanged` for the UI. |
| `oxplow.metric.enable { keys, enabled }` | `External`: turns metrics on or off — computes the new `metrics:` list with `MetricsService::apply_metric_enabled` (a bundled code metric is off until a `use:` names it; a producer/extension metric is on until an `enabled: false` marker) starting from the project's list (or the global one it inherits, never a local one), and writes `oxplow.metrics` at project level through the one settings write; an unknown key (not in `metric_catalog`) is refused | undoable (restores the prior list); logs `config.changed@3 { key: oxplow.metrics, … }`; the `config.metrics` reactor reseeds |
| `oxplow.metric.record { key, value, subject?, dims?, stream? }` | `Tx` over `fact_store::record_facts_tx` (`commands/metric.rs`, P4.8) | not undoable. An asserted fact on the metric's source measure, stamped to match its filter, anchored to the stream's latest snapshot; the fact and the command run commit together. A formula or `count` metric, an unknown key, or another stream (for an agent) is refused. After commit: clears the fact memo (the change loop announces `MetricSamplesChanged` for the measure) |
| `oxplow.metric.rebuild { force }` | `External` over `MetricsService::rebuild_baseline` | not undoable. Every snapshot fact collector's whole-tree baseline. It drives snapshot captures and collector scripts, which own their own transactions. (Running one collector is `oxplow.collector.sync`; `metric.run` is deleted.) |
| `oxplow.vcs.commit` / `oxplow.vcs.stage` / `oxplow.vcs.discard` / `oxplow.vcs.fetch` / `oxplow.vcs.pull` / `oxplow.vcs.push` / `oxplow.vcs.merge` / `oxplow.vcs.checkout_branch` / `oxplow.vcs.rename_branch` / `oxplow.vcs.delete_branch` / `oxplow.vcs.resolve_conflict`; `oxplow.git.rebase` / `oxplow.git.cherry_pick` / `oxplow.git.revert` / `oxplow.git.ignore` | `External` over the `Vcs` trait (`git.*`: the git provider's own ops) (`commands/vcs.rs`, P5.B6) | any caller's, a page's too — an agent's on its own stream only (its terminal's `git` reaches only its own worktree; `vcs_op` refuses another stream), not undoable. Each takes the `stream` it acts on, by URI (`oxplow:oxplow/stream?id=str1`), resolved strictly (an unknown stream is refused, never the primary's). `oxplow.vcs.discard`, `oxplow.vcs.merge`, `oxplow.vcs.delete_branch`, `oxplow.git.rebase` and `oxplow.git.revert` are `Destructive` (confirmed); `oxplow.vcs.pull` and `oxplow.vcs.push` don't ask (routine, the person's own click). The spec is the one rule: the Git Dashboard arms its confirm from it (`SpecConfirm`). The result — and the command run row's — is the VCS's `OpOutcome { success, log, conflicts, auto_resolved }` (`oxplow.vcs.commit`: `{ success, revision }`); an op that failed outright (a rejected push, an unreachable remote) is the run's `Failed`, git's log its message, recorded as `failed` — never an `ok` row whose result says it failed — while one that stopped at conflicts answers with them (the repository is mid-merge for a person). Once a run is recorded (its `after_commit`; at once for one that failed, which may have touched the workspace too) the stream's `WorkspaceChanged` (and `VcsRefsChanged`; every stream's after a fetch, push, rename or delete) is announced. See [vcs.md](./vcs.md) |
| `<provider>.<name>` (an enabled external provider's own declared commands — not its capability's verbs: the fake's `estimate`) | `External` over the provider process's `invoke` (`providers/registry.rs`, P5.D3) | registered while its instance is enabled (and removed when it stops), `Experimental`; `confirm`, `access` and `undoable` as the provider declares (its `inverse` becomes `<provider>.<command>`). The events its `invoke` returns are logged caused by the run — only types it declares, and a record event only for its own items. See [providers.md](./providers.md) |
| `oxplow.collector.sync { collector, thread? }` | `External` over the collector's program or script (`collector_runner.rs`, P7.B3; was `source.sync`) | `Write`, not undoable. Runs a collector from the primary worktree and commits its rows, its `collector_run` row and `collector.synced@2` in one transaction (a failed run commits the failure). It never admits anything: a collector whose extension lacks a permission it needs (`process` for an exec one) is `Invalid`. Run by the system it's the `every:` schedule's run (`trigger: every`); by anyone else, `manual`. **The one manual run for every collector** (it replaced `source.sync` and `metric.run`): a fact collector (`facts:`, owner an extension, the project's own `project` among them) runs through the fact engine (`MetricsService::run_collector_by_key`) against the stream's latest snapshot, recording its capture, `collector_run` and `collector.synced@2` (the engine's own writes). Result `{ owner, id, rowCounts, facts }` — `rowCounts` for an entity collector, `facts` (count recorded) for a fact collector. A **report collector** (`records:`) is read by the collection service in `thread` (an agent's own; a person names one): `{ owner, id, recorded: { status, records, run, … } }` |
| `oxplow.provider.sync { instance, collector? }` | `External` over the provider process's `read` (`providers/sync.rs`, P7.A3) | `Record` (it restates items, never claims), not undoable. Reads a running instance's collectors (all, or one — an undeclared one is `Invalid` at `/collector`) from their last checkpoints; the records land as record events, each batch committed with its `$/state`. The one way to read: Settings → Integrations' Sync Now, an agent, the schedule (as the system) and the read an instance gets when it starts. Result `{ reads: [{ collector, records }] }`. See [providers.md](./providers.md) |
| `oxplow.file.save { stream, path, content, filesystem? }` / `oxplow.file.create { stream, path, content? }` / `oxplow.file.create_directory { stream, path }` / `oxplow.file.rename { stream, from, to }` / `oxplow.file.delete { stream, path }` | `External` over `WorkspaceFiles` (`commands/file.rs`, scope `files.write`): the worktree is the system that owns this state; `save` writes through the filesystem registry and refuses a filesystem without the `writable` feature (`oxplow:extensions`, `oxplow:temp`) naming it | a person in any stream, an agent or effect only in its own (`stream_of`); `delete` is `Confirm::Destructive` (a person's). Each logs what it did — `file.saved`, `file.created`, `file.directory_created`, `file.renamed`, `file.deleted` (subject `file:<path>`, the stream anchored) — which an effect `on:` reacts to and which the window follows with its open buffers (`watchFileEvents`, `fileEvents.ts`, followed by `editor/buffers.ts`: a rename moves them, a delete closes them, whoever ran it). `save` and `create` leave `content` out of the command run (`with_unrecorded`). The five bespoke workspace write IPCs are gone |
| `oxplow.capability.choose { capability, provider?, level? }` | `External` (`commands/config_commands.rs`, scope `config.write`) | Sets or unsets the scalar `oxplow.activeProviders.<capability>` at local/project/global level. Resetting one choice restores inheritance independently; the runtime map is a projection. |
| `oxplow.ai.set_provider { id, kind, base_url?, key? }` / `oxplow.ai.remove_provider { id }` / `oxplow.ai.set_role { role, provider?, model?, restrict? }` / `oxplow.ai.test_provider { id, model }` | `External` over `AiService` (`commands/ai_settings.rs`, scopes `ai.settings`, and `ai.test` for the test, which `spends`): the settings are independent `oxplow.ai.<provider>.enabled`, `.baseUrl`, secret `.apiKey`, and atomic `oxplow.ai.roles.<role>` bindings, written at global level through the shared store | AI Settings (`ai-settings`) as forms. `set_provider`'s `key` is a secret (`writeOnly: true` in its schema; "Secrets" below): kept out of the record, masked in the form; `remove_provider` is refused while a role uses the provider; `set_role` with neither provider nor model unbinds the role and announces `ConfigChanged`; `test_provider` returns `{ reply }`. The `save_ai_provider` / `remove_ai_provider` / `set_ai_role` / `test_ai_provider` IPCs are gone; `ai_settings` still reads |
| `oxplow.provider.check_instance { instance, config? }` / `set_instance { instance, enabled, config? }` / `add_instance { instance, provider, scope }` / `turn_off_here { instance }` / `remove_instance { instance }` (scope `providers.write`) and `oxplow.provider.set_credential { instance, name, value? }` / `begin_sign_in { instance, name, redirect_port }` / `complete_sign_in { instance, name, redirect }` / `cancel_sign_in { instance, name, sign_in }` (scope `credentials.write`) | `External` over `ProviderRegistry` (`commands/provider_instances.rs`): enabling an instance runs its program and its credentials are in the OS keychain. `set_credential`'s `value` is `writeOnly`, kept out of every record like `oxplow.collector.set_credential`'s. A destructive `remove_instance` is confirmed. Which actors may enable is the registry's (`Actor::Human` for a global instance's file) | Settings → Integrations (the `IntegrationsSection` until it is a foundation component); instances read as `v_provider_instance`, `v_provider_credential`, `v_provider_collector` |
| `oxplow.collector.set_credential { collector, name, value? }` / `oxplow.extension.set_secret { extension, reference?, value }` / `oxplow.extension.revoke_secret { extension, reference }` | `External` over the keychain (`commands/credentials.rs`, scope `credentials.write`; `set_secret` and `revoke_secret` over `ExtensionSecrets`, which also restarts a provider running on the secret) | Settings → Data (credentials) and Settings → Integrations (provider secrets) as forms (`extensions-credentials`, `extensions-secrets`). `value` is a secret (`writeOnly`): kept out of the record, masked in the form, a blank `set_credential` value clears it. A credential must be one some collector of that extension declares (`Invalid` at `/name`); `set_secret` needs a current owner. Announce `CredentialChanged`; presence reads `v_collector_credential` and `v_extension_secret`. The `set_credential`, `save_extension_secret`, `extension_secret_is_set` and `revoke_extension_secret` IPCs are gone |
| `oxplow.agent.set_prompt_append { text }` | `External` (`commands/agent_settings.rs`, scope `config.write`) over `config_commands::change` | Writes `agentPromptAppend` (blank removes it): the Agents area's prompt form. The same one config write as `oxplow.config.set { key: agentPromptAppend }`, with a text field rather than a JSON value |
| `oxplow.agent.set_agents { agents }` / `oxplow.agent.set_setting { agent, key, value? }` | `External` (`commands/agent_settings.rs`, scope `config.write`) over `config_commands::change` | Settings → Agents: `set_agents` checks each key is a registered harness (`Invalid` at `/agents/<i>`, none at all at `/agents`) and writes `agents`; `set_setting` checks the harness declares `key` (`/key`, an unknown agent `/agent`) and rewrites `agentConfig` (a blank value clears it, an emptied entry goes). Both are the one config write: schema-checked, `config.changed@2`, undoable (the inverse is `oxplow.config.set`). The prompt addition is the key `agentPromptAppend` (`oxplow.config.set` / `unset`). The `set_agents` / `set_agent_setting` / `set_agent_prompt_append` IPCs are gone |
| `oxplow.file.mark_generated { entry, generated? }` | `External` (`commands/config_commands.rs`, scope `config.write`) over `config_commands::change` | The Files page's Mark / Unmark as Generated: adds `entry` (trimmed of `/`) to the `generated` key's sorted `exclude` list, or with `generated: false` takes it off; nothing marked removes the key. One config write: `config.changed@2`, undoable. The `set_generated` IPC is gone |
| `oxplow.delivery.retry { id }` / `oxplow.delivery.discard { id }` | `External` over the event pump (`commands/delivery.rs`, scope `deliveries.write`): the pump owns the dead letter (`v_event_dead_letter`) and a retry runs a consumer's handler again | `discard` is `Confirm::Destructive`. `retry` returns the letter (`retried`, or still `pending` with the new error); a letter that isn't pending, has no consumer or doesn't exist is `Invalid`. The `list_/retry_/discard_dead_letter` IPCs are gone |
| `oxplow.delivery.retry_all {}` / `oxplow.delivery.discard_all {}` | `External` over the same pump calls, every pending letter in turn (`op_all`); `retried`/`delivered`/`pending` or `discarded` counts; discard asks first. A parked event is never retried by itself. |
| `oxplow.lsp.restart_server { stream, language }` | `External` over `LspSessionManager::restart` (`commands/lsp.rs`, scope `lsp.control`): the server is a process on this machine | tears the stream's session down and starts it again, replaying the open documents; pushes `LspServersChanged`. The `restart_lsp_server` IPC is gone |
| `oxplow.effect.retry { effect, event }` | `External` over `effect_triggers::run_reaction` (`commands/effect.rs`, P9.D4; the `effects.run` operation, its services filled at boot when the `effect.triggers` consumer registers) | the Alerts page's failed reactions; `Confirm::Always` (a person's run asks), not undoable. Has an extension's effect react again to an event its reaction to **failed** (its latest attempt, `v_effect_run.latest`), as the next attempt, run as the effect is now — enabled and deployed as it is, composing afresh. Asked every time: a failed attempt interrupted with a step outside oxplow under way may have landed it, and a retry sends it again. `Invalid` for a reaction that didn't fail or was never made, a disabled effect, an unknown effect or event. Result `{ effect, event, attempt, outcome, reason? }` — the retry's own outcome, which may be `failed` again. See [extensions-events.md](./extensions-events.md) "Effects" |
| `oxplow.effect.retry_all {}` | `External`, `retry_one` for every reaction `v_effect_run.retryable` lists (its latest attempt failed, and its effect is declared and enabled; `effect_run_store::failed_latest_tx`); one that can't run is `refused` with why and the rest still run; `retried`/`ok`/`still_failing`; asks first. |
| `oxplow.effect.backfill { effect, from_seq? \| since?, to_seq? }` | `External` over `commands/effect.rs::backfill` (P9.D5) | `Confirm::Always`, not undoable. Has an effect react to the matching events in the range it never reacted to (those logged before it was enabled), oldest first, once each, as it is now — at most 200 a run. Each is an attempt with `origin: backfill`, under the same dedupe, loop guard and health as a live reaction; three failures in a row disable the effect and stop the run, and so does the effect being disabled (checked before each reaction). `Invalid` for an unknown or disabled effect, or both `from_seq` and `since`. Result `{ effect, planned, ran, skipped, proposed, failed, remaining, stopped? }` |
| `oxplow.effect.backfill_plan { effect, from_seq? \| since?, to_seq? }` | `Tx`, `Read` (P9.D5) | What `oxplow.effect.backfill` would react to: `{ effect, planned, from_seq, to_seq }`. The count a person is shown before running one (a confirmation carries a command's summary and input, not a computed count) |
| `oxplow.contribution.enable { extension, kind, contribution }` | `External` over `contribution_health.rs` and the provider registry (P7.C1; replaced `provider.enable`) | Not undoable. Enables a disabled contribution on this machine again — clearing an automatic disable (`contribution_health` back to `ok`, its count starting over). `kind` (`provider` \| `collector` \| `effect`) is part of its name — a provider and a collector may share an id; there must be a failed one of that kind, or a provider instance the registry knows. A provider instance resets its backoff and reconciles, so it starts when `extensionInstances` enables it; a collector runs at its next trigger. Logs `contribution.enabled@1 { extension, contribution, kind }`; the result is its `contribution_health` row. See [providers.md](./providers.md) |
| `oxplow.contribution.enable_all { extension? }` | `External` over `contribution_health.rs` and the provider registry, like `enable` | Not undoable. Enables every contribution a failure policy disabled (`contribution_health.state = 'disabled'`) — `extension`'s, or every extension's — each exactly as `oxplow.contribution.enable` does (a provider instance resets and reconciles; a collector runs at its next trigger), logging one `contribution.enabled@1` apiece. Returns `{ enabled: [<contribution_health row>…] }`, empty when nothing was disabled. Extension problems and Data → Collectors offer it as **Enable All Disabled (N)**. |
| `oxplow.page.show { page?, spec?, params? }` | `External` | Shows a page beside the conversation (an agent's `oxplow_show_page`): a page by id opens at its URI; an inline `spec` is checked (`parse_temp_page`: read-only, no `for:`), written as the page file `v<hash of the YAML>.yaml` on `oxplow:temp` through `Services.temp.put` and shown as `oxplow:oxplow/page?page=temp/<id>` (the same YAML is the same page; a daemon restart keeps it; the GC removes it after 7 idle days). It opens as a tab of the caller's thread when a window is open (`opened`, else `why`) and returns the page's text. Saving it is `oxplow.extension.write_page` (Save as Page) or a knowledge page with an `oxplow-page` block. See [pages-and-tabs.md](./pages-and-tabs.md) "Page files" |
| `oxplow.ui.report_error { label, command?, message?, exit_code?, thread?, stream?, signal?, duration_ms?, stderr?, stdout?, command_run_id?, proposal? }` | `Tx` (`commands/ui.rs`) | `Record`, not undoable. The app's runner (`commands/runForPerson.ts`, usability.md "A person's command that fails") runs it once for each person's command that fails, and `recordOpError` for any other failure; its result's `event` is the row's id, which the window uses to tell a failure it showed beside its control from one the toast announces; the Alerts page lists `v_op_error`, not a client copy. `command_run_id` identifies the person's failed command run: the daemon's dispatch puts it on the IPC error (`IpcError.command_run_id`, from `oxplow_app::commands::tracking_failed_run`, which `record_unapplied` feeds) and the runner passes it on; `proposal` is an approval's, also the event's subject. Logs `ui.op_failed@3` (V68 renames the command link in stored payloads) anchored to `thread` (and its stream), or, from no thread, to the `stream` on screen so its agent can read the output (not both: `Invalid` at `/stream`); stderr / stdout stored as its `output` body (`put_json_tx`, namespace `ui`); the agent reads them as `v_op_error` and `read_event_content(event_id, output)` |
| `oxplow.op_error.mark_seen { event }`, `oxplow.op_error.dismiss { event }`, `oxplow.op_error.dismiss_all {}` (`Tx`) | `commands/op_error.rs` over `oxplow_db::op_error_store` | `Record`, not undoable: the person opened a failed operation's output, or cleared it from the app's list. Each logs `ui.op_error.seen@1` / `ui.op_error.dismissed@1 { event }`, anchored as the failure and kept as long (the `ui` window), read back as `v_op_error.seen` / `dismissed` (dismissing is seeing). `event` that isn't a `ui.op_failed` event is `Invalid` at `/event`; one already in that state is `unchanged` (a run without an execution event); `dismiss_all` logs one per failure not dismissed, `{ dismissed: n }` |
| `oxplow.metric.scaffold { name, language?, glob?, origin? }` | `Tx`, `Read` | the files of a code metric extension (`{ name, dir, files: [{ path, content }] }`: manifest, collector script, page, fixture — the template `oxplow extension new metric` writes); writes nothing (the agent writes them, then `extension check` / `extension test`) |

**Callers.** A task edit or stage change is a store's command
(`oxtask.task.*`) or a task action point, run under its actor. The desktop
runs them through RPC `run_command` as `Actor::Human`; an agent runs them
through MCP `run_command` as `Actor::Agent` with the caller's verified
thread and stream (`McpCaller` — see "MCP identity"), so an anonymous
connection can't file or change a task. An epic is an `oxtask.task.create`
per row (children with `parent`), and closing is the
`oxplow.command.sequence` above.

**Config writes use the shared settings service.** Generic `config.*` commands
and specialized domain commands resolve registered keys and choose a level
explicitly. Runtime state remains in its own stores. The `config.changed` pump
consumers apply changes to running services; `ConfigChanged` refreshes the UI.
**The person's way onto the bus** (P5.A1): RPC `run_command { id,
input, confirmed }` and `undo_command { command_run_id, confirmed }`
(`oxplow_rpc::commands::bus`, `Actor::Human`; desktop `runCommand` /
`undoCommand` in `api.ts`). A call that needs confirmation comes back
`NEEDS_CONFIRMATION` and the UI asks, then calls again with `confirmed`.
A typed IPC setter is a convenience over one command; anything new the
UI writes goes through `run_command`. Parity: `both("run_command")`,
`ui("undo_command")`. A person's undo is offered where they ran it: a
command run through `personCommands` (a board move, a search offer)
that comes back undoable (its outcome's `inverse`) shows **Undo** on its
"done" toast, which runs `undoCommand(command_run_id)` — the click is the
person's confirmation — and says "undone" or records the failure
(P11). The thrown error keeps its IPC code
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
(one command) or `prefix` (an area, `oxtask.task.`), each as the
agent needs it — `id`, `summary`, `input_schema`, its `example` input
(lifted out of the schema's `examples`, shown once), `confirm`,
`undoable`. The person-facing parts of a spec (`ui`, `op`, `atomicity`,
`needs`, `access`) never reach an agent. An id or prefix it doesn't offer
is refused naming the offered ids that end alike (`config.get` →
`oxplow.config.get`). Measured on our own sessions, agents guessed shapes
rather than read a listing of every full spec (164 KB): the index keeps
discovery cheap, the example makes the first call right, and a refusal
quotes the example. `run_command` answers with the outcome:
`result`, `command_run_id`, `event_id`, `inverse?` — or, for an effect's run that asks,
`{ kind: "proposed", proposal, message }`). Every other tool is a
command or page an extension declares with `tool:` ([extensions-commands.md](./extensions-commands.md)
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
no run without an actor to record it to. `OxplowMcp::verified_actor` still
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
   agent's runs are recorded alike.
6. Tests: the input schema rejection names the field; the
   confirmation rules hold; the command run row and `command.executed` share
   the transaction; undo applies the inverse.

## Secrets

A secret is a kind of value. A command input field whose JSON Schema says
`writeOnly: true` is a secret (`oxplow.ai.set_provider`'s `key` is one).
Nothing else declares it: `commands::secret_fields` reads the schema.

- It is unrecorded by itself: the command run row and the events keep it out,
  as a `with_unrecorded` field is kept out ("Private operation inputs").
- Every `SchemaForm` masks it, and it never enters a draft.
- No model returns it: a read gives "set" or "not set", never the value.

## Private operation inputs

An operation's `with_unrecorded` fields are represented by omitted-byte metadata
in command runs. The reserved `*` marker withholds the entire input, including non-object
inputs, and counts JSON bytes without allocating a serialized payload. Private
state CAS uses this for values, returns only a stamp and has no inverse payload.

Composition captures private descendants from the routed command tree. Its
invocation-local spec then withholds the entire parent input, since arbitrary
composers can derive a private child from any field. This never changes the live
registry, schema, grants or handler. Step success/failure metadata respects each
child's recording policy; private failure diagnostics are replaced with fixed
messages in stored command run history. Early failed composites conservatively
withhold their input before descendants are known. The caller still receives the
actual error.

Proposals retain replayable inputs in a public dataset. A private-input call or
composition needing confirmation is therefore refused before proposal storage
or dry run. A person can make a direct confirmed invocation; unconfirmed calls
can still run where ordinary command admission and grants permit them. Private
data read and intentionally returned/emitted by extension code is governed by
that extension's own explicit output contract; input redaction does not sanitize
arbitrary extension-authored outputs.

Declared jobs expose native `jobs.write/enqueue`, `jobs.write/cancel` and
`jobs.read/snapshot` operations for extension-owned command declarations. Intent
and provenance commit in the recorded transaction; request content is
`unrecorded`, including parent composition/proposal handling. Model work is
performed later outside SQL under the job's extension's current permissions and exact registry
owner. `jobs.read` is also a read-only scope service for composers and workers;
it returns owned status/results and never the original request. See
[extensions-jobs.md](./extensions-jobs.md) for declaration/input shapes and lifecycle.

### Settings file completion

Generic config writes and domain settings adapters are External: file/keychain
failures reach the caller before success events, command run, and inverse are recorded.
`config.update {level, set, unset}` validates every ordinary key, takes one stable
file lock, edits the whole document, and performs one atomic replacement. Domain
operations that split a runtime object into keys use the same batch; multi-key
undo uses `config.update` too. Secret writes remain individual secure-backend
operations and cannot enter a YAML batch. There is no atomic transaction spanning
YAML and SQLite; after the external write, normal External run-record semantics apply.

Opening a session is a transaction over session records and requires an explicit
enabled harness. It writes no preference; the UI remembers picker selection as
UI state. Undo closes the slot.
