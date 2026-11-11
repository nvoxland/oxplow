# Tasks

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C). oxtask is the built-in tracker (`crates/oxplow-oxtask`),
called exactly like any other; another tracker is a **backend that replaces it**: the person picks the
active tracker, and every new item goes there. So it should have the
visibility the person wants for their work — usually still just theirs,
like a local tracker such as beads — not a team's tracker
([providers.md](./providers.md) "Which trackers are backends"). Reads are
SQL; writes are the `task.*` commands — **one write surface for every
provider** (P7.A1), which the bus dispatches to the item's provider.

## The model

**The interface.** Everything — the UI, core, extensions, the agent's
tools — reads tasks through `v_task`, `v_task_link` and
`v_task_comment`, and writes them with the `task.*` commands;
nothing outside a task tracker's implementation reads that list's own
tables (`oxplow-dev` is the one development exception). oxtask
publishes no dataset of its own — `v_task`, `v_task_link` and
`v_task_note` were removed; its own fields are `stage` and
`native`. The views show **the active task tracker's items only**: one
list at a time, whichever implementation it is, and nothing with none.

`v_task` (table `task`, V115) holds the items by URI,
`oxplow:oxplow/task?id=<id>&provider=<provider>`: `title`, `body`, its `stage`
(one the list declares, below) with the list's `stage_title` and its
bucket as `state`, the list's own `native` JSON, `parent_uri` (NULL when the parent is
deleted: a deleted item isn't a live one, so the dataset's parent
relationship holds; tsk572), `closed_at`, and core's assignment
(below): `assignee` and `rank`.
`v_task_link` is its links (`to_uri`, its `to_kind`, `link_type`):
to another item of the list (`blocks`, `discovered_from`, `relates_to`,
`duplicates`, `supersedes`, `replies_to`), or `about` any URI — what the
item is about (`oxplow:oxplow/collector?collector=triage/hot`, `oxplow:oxplow/request?id=req12`). A link's target
needn't resolve; it may name something deleted or an extension's resource
since uninstalled (resources.md "Using URIs"),
`v_task_comment` its comments. Core reads tasks only through
these: a thread's assigned items (`v_task`), a task's URI summary
(`ref_resolver`), a `[[…]]` link's check (`link_check`) and search
(`v_search_task`).

**Buckets and stages.** A task tracker declares its own **stages**, its
workflow in order — `[{ id, title, bucket }]`
(`oxplow_domain::tasks::StageDecl`, checked by `stages_problem`: ids
snake_case and unique, titles given, a stage in each of `todo`,
`in_progress` and `done`, and a stage named like a bucket in it) — and
core keeps six **buckets**, `Bucket`: `backlog` (filed, not ready),
`todo` (ready: what an idle thread may pull), `in_progress` (started),
`blocked` (waits on an answer), `done` and `canceled` (closed). Core acts
on buckets only — assignment and its release, the pool, Waiting
on Me — and the screens draw the list's stages: a board's columns, a
state picker, a Mark menu. **The first stage of a bucket is its
default**: an input naming the bucket (`to: done`) lands there, so an
agent or an effect speaks buckets and works with any list. A record
names only its stage; the projection looks its bucket up in the list's
declaration (`stage_bucket_tx`, from `capability_provider.stages_json`),
and an undeclared stage dead-letters. Published as
`v_capability_provider.stages`. A built-in's are core's table's
(`BuiltIn.stages`: oxtask' are its statuses, `backlog`, `ready`
"Ready", `in_progress`, `blocked`, `done`, `canceled`); an external
provider's are its `providers:` entry's `stages:`, which a task tracker must
declare. None's are the six buckets' names (`bucket_stages`).

**Declared fields.** What a list keeps in `native` beyond those columns
is declared, so screens render and edit it without knowing which list is
active: `[{ name, title, kind: enum|text|number, values? }]`
(`oxplow_domain::tasks::FieldDecl`, checked by `fields_problem`),
published as `v_capability_provider.fields`. A built-in's are core's
table's (`BuiltIn.fields`: oxtask declare `priority`, an enum of
urgent/high/medium/low); an external provider's are its `providers:`
entry's `fields:`. None declares none. How the rows are written and
oxplow's state mapping are in [data-model.md](./data-model.md) "`task`".

**One writer, one schema.** Every list's rows arrive by projection: its
`task.recorded@3 { item }` events — what each verb answers with,
and what a read back restates — carry the item as it now stands: URI,
title, body, stage, native fields, parent, deleted,
and, when the list keeps them, `links` and `comments`, each the whole
set (stated, the host restates it; absent, it keeps what it has). Who it
is assigned to and its place in their queue are never a list's
("Assignment"). The pump consumer `tasks.project`
(`crates/oxplow-app/src/tasks.rs`) upserts it by URI; a replay
restates the same row. Why one record event rather than state on each of
`created` / `edited` / `state_changed`: the "what happened" events stay
small and the same for every list, and the projection is one upsert
from one event type. A run's projections are delivered before the run
returns ([commands.md](./commands.md) "A run's projections are
delivered"), so a write reads back at once.

**oxtask is its own crate**, `crates/oxplow-oxtask`, named by
nothing outside it but the built-in factory and `Services`' stores: the
task store and its links and notes (`store.rs`, `satellite.rs`), the
task types, the mapping of statuses and priority onto the interface
(`mapping.rs`), its answers to the verbs (`verbs.rs`: `create_tx`,
`update_tx`, `transition_tx`, `link_tx`, `comment_tx`, `delete_tx`,
`reorder_tx`, `move_tx`, each over a connection, the interface's input
and the actor, answering with its result, the verb that undoes it and the
tasks it changed), the record of a task (`record.rs`: the item whole,
from the task tables — links, notes, list and rank included) and
`OxtaskTracker`, its `TaskVerbs`: each verb in a transaction of its own
over the task tables, answering with the `task.recorded` of every
task it changed (a reorder's renumbered neighbours too). A refused verb
rolls its transaction back. It writes nothing of core's: no `task*`
row (V29 dropped the triggers that did), no page ref, no event — core
logs the run, its canonical events and the records. The store's own
async writes (tests) reach the interface only when restated
(`test_fixtures::restate_task`).

**A built-in list registers by declaration.** `oxplow-bundled` declares
`implementations: [{ capability: task_tracker, id: oxplow, entry:
"oxplow:oxtask" }]`; `tasks::register_built_ins` registers the
provider each declared built-in names (`built_in_provider`, the factory:
its features and id pattern from `capabilities::BUILT_INS`, its verbs
the crate's) when services are built and at every `capabilities::refresh`,
and unregisters it when nothing declares it — the task data stays, and
nothing reaches it. A built-in whose items' URIs carry a provider id
(`BuiltIn.provider`: `oxplow:oxtask` is `oxplow`, as every
`oxplow:oxplow/task?id=tsk<n>&provider=oxtask` says) is declared under that id and no other;
`implementations.rs` refuses another, which would register a list that
refuses its own items.

**A script list.** An extension may write a task tracker as a script:
`implementations: [{ capability: task_tracker, id: notes, entry:
trackers/notes.star, needs: [sql.read], stages: [...] }]`, one Starlark
function per verb (`create`, `update`, `transition`, and those its
features add). Its host makes it an `ExternalTaskTracker` over a
`providers::script::ScriptEndpoint`, so `task.<verb>` dispatches to it as
to a provider's instance, and its answers are checked the same way
(extensions-commands.md "Implementations" has the ABI). It keeps nothing
of its own: its verbs answer `task.recorded@3` and read rows back from
`v_task`. The conformance suite passes against the example
(`examples/extensions/note-tracker`, `providers::script` tests) and runs
in `oxplow extension test` for any script tracker.

**Page refs** for every list's item come from the interface: its body's
mentions, its links (`task_link:<type>`, the list's own types) and
its comments' mentions (`comment_*`, keyed by the item), restated by
`task_refs::restate_tx` on the item's events (the
`page_ref.task` consumer, after `tasks.project`) and by the
boot repair. A list writes no page refs of its own.

## Assignment

**Who is going to work on an item is core's record, for every list**,
never a list's: an item is assigned to a thread
(`oxplow:oxplow/thread?id=thr3`), the person (`oxplow:oxplow/person`, a
resource foundation declares; its optional `id` names one of several
later), or nobody. It is independent of the state: a backlog item can
be assigned, which is how a person or a controller agent routes work to
threads before it is ready. An unassigned `todo` item is **the pool**:
what any thread may pull when it has nothing of its own
(`oxplow/next-task`). Each assignee — nobody included — has a queue,
the open items assigned to it in `rank` order. **A done or canceled item
is nobody's**: who worked on it is its requests (`v_request_task`), the
same for every list. Filing assigns nobody unless the
create names an `assignee`; who filed it is the event's thread anchor.

The record is `task_assignment` (V7: `uri`, `assignee` NULL for
nobody, `rank`, `assigned_at`), kept by the `tasks.project` consumer
from core's **`task.assigned@1 { task, from?, to?, before?,
after? }`**: the item joins `to`'s queue next to the item named, or at its
end, and the queue is renumbered; a record of the item as done or
canceled, from whichever list, deletes its row. `v_task` joins it
(`assignee`, `rank`). The writes, all core's and in the bus's
transaction (`commands/task.rs`):

- `oxplow.task.assign { uri, to?, before?, after? }` — `to` a thread's
  URI or id, the person's URI, or null; a closed thread (or none) is
  refused at `/to`, a closed item at `/uri`. Undone by assigning it back
  to its place. With no list active (none) it answers `{ tracked: false }`.
- `oxplow.task.reorder { uri, before?, after? }` — its place in its
  assignee's queue; the neighbour must be in the same queue.
- `create { …, assignee? }` — checked first, kept from the list, and
  logged as `task.assigned` once the item exists.
- A verb that closes an assigned item logs `task.assigned` to nobody.
- `oxplow.thread.close` and `oxplow.stream.archive` release their threads'
  items to nobody.
- `transition` to an in-progress stage by an agent takes the item: it's
  assigned to the agent's thread, from whoever had it (`dispatching`,
  every tracker). A person's start assigns nothing.
- Taking a handoff assigns its item to the recipient thread
  (`.context/agent-model.md` "Handoffs").

## Reading them in the UI

The desktop reads **only the interface**, whichever list is active —
never oxplow's task tables, ids or URIs; a guard test
(`apps/desktop/src/taskInterface.guard.test.ts`) fails on a `v_task`
name, a built `oxplow/task` URI or `tsk` id parsing in desktop code.

`apps/desktop/src/tasks.ts` is the one read and write path:

- **`Task`** is a `v_task` row — `uri`, `provider`, `title`,
  `body`, `state`, `parentRef`, `assignee` (null = nobody), `rank`,
  `closedAt`, the list's own fields as `native`, and `commentCount`
  (`v_task_comment`).
- **`readTask`** and **`readTasksByRef`** read the rest (Tasks
  and Task List are pages over `v_task`: an assignee's queue in
  order (`rank`, then `created_at`), an item with a child an epic).
- **Writes** are `task.*` commands by uri: `createTask` (no
  list named — it files on the active one; `native` carries the list's
  own fields), `updateTask`, `transitionTask`, `applyItemChange`
  (an edit from any surface: fields and/or state), `deleteTask`,
  and `reorderTasks` (a drag's new order becomes one
  `oxplow.task.reorder` by neighbour, `placementFromOrder`).
- **The active list's profile** — `readTaskTrackerProfile` /
  `useTaskTrackerProfile` (one shared read, re-read on a switch): its
  provider, features, declared `fields`, `stages` and `idPattern`
  (`v_capability_provider`). A `Task` carries its bucket (`state`,
  the `Bucket` union), its `stage` and `stageTitle`; colours and glyphs
  go by bucket (`stateColor`, `vizIcons`), words by stage. Screens offer only what it can do (reordering
  and assignment are core's, so always):
  epics, parents and "+ Item" with `hierarchy`, Comment… / Link… with
  `comments` / `links`, Delete with `delete`. **Declared fields render
  generically** (`components/TaskFields.tsx`: an enum as a pill
  picker — a `priority` enum with the priority glyph — text and number
  as inputs, a `read_only` field shown only): on list rows and Board
  cards, in the detail rail, in New Item, as bulk "Change …" actions,
  and as the Tasks page's filter chips. oxplow declares `priority` and
  a read-only `author` (who filed it). **`taskRefOfMention`** turns
  a loose id in text into a URI by the list's `idPattern` (markdown
  `[[tsk42]]`) — none matches nothing.

Every read returns what it read (`reads`), and a consumer re-runs it
through `useRerunOnChange` — or `readsChanged` outside a component
(`useBackendSubscriptions`, the title cache) — when one of those datasets
changes. With **none** active every read is empty and the screens show
their empty states.

**One page for every item** (oxplow-bundled's `task` document page,
the renderer of `oxplow/task`; params `uri`, `thread_id`,
`stream_id`). Its rail is what the item is and what can be done to it:
About (its stage by its list's name with its bucket's glyph, who it's
assigned to, its epic, its list, its id, when it was made, changed and
closed; its right-click menu has Mark ▸ the list's stages and Assign to ▸
the stream's threads and You, each one action with `options_from`, the
current one checked, then Unassign and Delete), Stage (a form over
`oxplow.task.transition`, the replaceable state part) and Details
(the title and the list's declared fields, over `update`). Its body is
the description (a `markdown` viz, edited in place), the items under it
(an epic's: each with its glyph, its description's first sentence, its
stage and assignee), Activity (its requests: the `item-requests` page,
embedded), Comments and Links (the `task-comments` and
`task-links` parts, each declaring its list feature in `needs`, so a
list without one draws its section as needing it) and the
`oxplow/task-detail.sections` slot, where bundled mounts Agent Tokens
(`task-tokens`) and Shared work (`item-shared-work`). See also: the Tasks
board of the thread it's assigned to (else the Task List at its stage)
and its epic. `task-actions` is the agents' page for one item (Mark
done, Record a block…). The item's own
list's extension may replace its state part (`task.detail.state`,
decided by the page run from the item's `provider`); About's menu always offers the moves, so an item
can always move. **Custom views** need no provider: any extension may
replace the Work board (`task.board`), the Task List's list (`task.list`)
or an item's description editor (`task.detail`) — with a
page or component over `v_task` that changes items only through the
interface commands, so it works with whichever list is active; the person
picks whose shows (`replacements`, Settings → Integrations).
[extensions-slots.md](./extensions-slots.md) "Replacements" has the contracts. The `oxplow/task-detail.sections` slot gets
`{ uri }`. Link…'s link type is free text (default `relates_to`): the
provider names its own types, and `about` fits any URI. Its target is
**any URI** (a task's, `oxplow:oxplow/request?id=req12`, `oxplow:oxplow/collector?collector=x/y`); a link row
in Outbound has **Unlink** on its right-click menu (`oxplow.task.unlink`,
the link's input, undoable like Link). Link rows render through the
generic ref → page mapping (`useBacklinks.ts` `openFor` → `openRef`): a
kind with a page opens it, a contribution (`collector:…`, `job:…`)
something resolves opens Settings, and a URI nothing resolves is a **dead
link** — its raw text, muted, tooltip "nothing resolves this URI; its
extension may be gone" — that still unlinks. `BacklinkEdge.resolved` (the
IPC's `ref_resolver::resolve_ref`) says which.

The **Tasks** board (oxplow-bundled's `tasks` page, its `board`
component) shows items as compact cards (the title and the description's
first sentence, two lines each at most; an urgent or high priority a
coloured edge; the rest on hover) in one column per stage of the list
(`column-<stage id>`, its title the stage's), in its order, but its
backlog and canceled ones. A to-do stage groups its cards under their epics, each heading folding its items
(remembered per person); Done holds what finished in the last three days,
twenty at most, and links to the rest in the Task List (a done stage's
column, by its bucket). The agent's live card leads the first started
stage. Each column
scrolls on its own. Drag a card to a column to transition it, within
its column to reorder it in its assignee's queue, or use its menu: Mark
<each other stage>, the list's editable choice fields, Assign to (the stream's open
threads, You, Nobody), Make top-level, Delete. Its scope (assigned to
this thread, its stream, you, nobody, or anyone; for a closed item, the
thread or stream whose requests worked it), priority and search are its
toolbar, the tab's view. The **Task List** (`task-list`) is the items in
one stage (its State filter lists the list's stages; `todo`, the default,
is the first to-do stage), closed ones latest first, whoever they're
assigned to — it lists tasks and never filters by assignment, but each
row names its assignee when it has one — with a quick add in its rail
that files a title in the stage it shows (the first to-do stage for a
started or closed one), for nobody. A row's Mark ▸ and Assign to ▸ are one action each over a loader
(`options_from`: the stages; the stream's open threads and You), the
row's own checked; Unassign stays its own. **New Item** files on the active
list: title, body, a starting stage (the list's backlog, to-do and blocked
ones),
who it's for (nobody unless picked), the
list's editable fields and — with `hierarchy` — a parent.

## Reading them for the agent

The agent reads tasks through the interface too, with no tool of
its own: `oxplow_query_sql` over `v_task` for a queue in order
(`WHERE assignee = … ORDER BY rank IS NULL, rank, created_at`),
`oxplow_read` on a task's URI for one item with its `links` and
`comments`, the bundled page `oxplow/thread-tasks` (`oxplow_thread_tasks`:
what is assigned to a thread and what it finished, with the requests that worked on each), and
`oxplow/next-task` (the tool `oxplow_next_task`, param `thread: "thr3"`)
for what to pick up next: the ready items assigned to the thread (an epic
with its ready descendants, or every ready non-epic item; open `blocks`
links hold an item), else the pool, by priority (`role: pool`). Starting
one assigns it to the thread. A controller agent routes work with
`oxplow.task.assign`. None of them belongs to an implementation, so the tool list never
changes with a switch; with none they read empty. A source
guard (`source_guards::only_oxplows_implementation_names_its_task_list`)
pins the few production files that may name oxplow's task store,
service or datasets.

## The capability

`oxplow_domain::tasks`:

- **`TaskTracker`** (a struct): `id` (the URI segment),
  `features`, `verbs: Arc<dyn TaskVerbs>`, its `id_pattern`, and
  `sink` (none).
- **`TaskVerbs::invoke(actor, verb, input, idempotency_key) ->
  VerbOutcome { result, events, inverse? }`**: every list's verbs —
  oxtask (`OxtaskTracker`), an extension's process
  (`ExternalTaskTracker`), none (`Sink`) — run outside the bus's
  transaction; its inverse is named by its **verb**. The key is
  the caller's when it has one (an effect's step), else the host mints
  one ([providers.md](./providers.md) "Idempotency").
- **A blank title is refused** by `oxplow.task.create` and `update` (`INVALID` at `/title`), so a view's form shows the message under the field; the check is the command's, not any list's.
- **`TaskTrackerFeatures`**: `hierarchy`, `comments`, `links`, `delete`,
  `idempotent_writes` (a write sent twice with one
  idempotency key is done once — [providers.md](./providers.md)
  "Idempotency").
- **`TaskTrackers`** (`Services.tasks`): providers by name;
  `for_ref` picks one by the URI's provider segment, and an unknown one
  is refused naming the registered providers; `active()` names the
  provider every `create` files on, resolved each time by the capability
  registry from the config as it is now — no copy, so a person's choice
  applies to the very next `create` (see below).
- **`VERBS`**: `create`, `update`, `transition`, `link`, `unlink`,
  `comment`, `delete` — the capability's verbs.

**One write surface: the dispatching `task.*`** (P7.A1;
`commands/task.rs`). Each verb is an `External` command
([commands.md](./commands.md) "Tx and External") on the item's provider
— the URI's segment, or for `create` the active one — called through its
`TaskVerbs`, the same way for every list, with **one audit row**
`task.<verb>`. In a composite it is a step: a sequence of task
verbs runs as steps, not undoable as a whole. Before anything runs it
refuses: a URI of another list's item (`/ref`,
naming the active list), a parent, link target or place of another list
(`/parent_uri`, `/target`, `/before`, `/after`), and a feature the list
doesn't declare.
A `create` hands the list the input without its `assignee` (core's,
above), and the list's inverse is renamed to `task.<verb>`, so
an undo dispatches again. **The answer is the interface's**, terse and the
same for every list: `{ uri }` (a comment's adds `comment`, the list's id
for it), plus core's `state` when the verb put the item in one and
`link_warnings` for a body — never a list's own row; a reader reads the
item from `v_task`. A create's, or an update's that sets a body,
result carries its `link_warnings` (`LinkDeps::item_warnings`), for every
list.
`assign` and `reorder` aren't a list's verbs: they are core's
("Assignment"). A Rust client, **`tasks::Tasks`**
(`Services::tasks_client()`), types the calls; the conformance
suite uses it. Extension commands compose the same verbs (comment on
and transition a task whatever its provider).

**The contract is the `v_task` columns**, one shape for every
provider (a provider's verb receives the same input, less `create`'s
`thread`):

- `create { title, body?, parent_uri?, stage?, native?, assignee? }` —
  **always on the active tracker**: the person
  chose it, and nothing a caller says — a person, an agent, an effect or
  oxplow itself — files anywhere else. One that isn't running is an
  error, never a fallback. `stage` absent is `todo`'s default; `assignee`
  is core's ("Assignment");
- `update { uri, title?, body?, parent_uri? ("" detaches), stage?,
  native? }`;
- `transition { uri, to }`;
- a `stage` / `to` is one of the list's stages or a bucket's name for its
  default: core resolves it before the list sees it
  (`commands/task.rs` `resolve_stages`), and refuses anything else at
  its field naming the list's stages, so a list's verb only ever receives
  its own stage ids;
- `link { uri, target, link_type }` / `unlink` (same input): the provider
  names its link types, and **every list with `links` takes `about` to
  any URI** (the conformance suite checks it). A target that is an item
  must be the same list's and is checked to exist; any other target
  must be a URI of a kind the running vocabulary has, and may dangle.
  `link` and `unlink` undo each other. `create` takes `links: [{ target,
  link_type }]`, made with the item (oxplow: in one transaction);
  `comment { uri, body }`, `delete { uri }` (Destructive; only with
  `features.delete`).

oxplow's mapping: its status is its stage, one each (V8 made its old
`archived` tasks done when completed, else canceled); `native` holds
`{ priority? }` (`deny_unknown_fields`). A task keeps no thread or order
of its own. A link belongs to its task, not to a thread.
`oxplow.task.comment` and `oxplow.task.link` refuse a deleted task (tsk572).
A comment's `oxtask_note.author` names who made it (`note_author`, tsk1000):
`user`, `agent` (a page acting for one included), `effect:<namespace>/<id>`
or `oxplow` — as a task's `author` is left empty for an effect or oxplow
(`task_author`), neither is shown as the person's.

**The interface's events are core's**, logged by `dispatching()`
(`canonical_events` in `commands/task.rs`) for every list alike,
from the verb, its input, the list's answer and the state it put the
item in — caused by the run's `command.executed`, subject the item,
anchored to the actor's thread. A list logs none of them itself
(oxplow's task store logs nothing); another list's answer adds its
`task.recorded`.

| Verb | Events |
|---|---|
| `create` | `task.created@2 { task, state }`, `task.state_changed@1 { task, to }` |
| `update` | `task.edited@2 { task, fields }` naming what the input set — `title`, `body`, `parent`, `native.<name>` — and `state_changed` when the state moved |
| `transition` | `state_changed` when the state moved |
| `link` | `task.linked@2 { task, target, link_type }` (subject both), unless the link was already there (`result.linked: false`) |
| `unlink` | `task.unlinked@1 { task, target, link_type }` (subject both), unless there was none (`result.unlinked: false`) |
| `comment` | `task.commented@2 { task, comment? }` — `comment` is the list's own id for it, when the answer names one (`result.comment`; oxplow: `not<n>`, the fake: `c<n>`) |
| `delete` | `task.deleted@2 { task }` |
| `reorder` / `move` | `edited` naming `rank` / `list` and `rank` |

Whether the state moved: core reads the item's state as the interface
shows it (`v_task.state`) before a transition or an update runs, and
logs `state_changed` only when the bucket of the stage its answer
recorded differs — a transition within a bucket moves nothing the
assignment acts on; it logs `edited` naming `stage` instead. A create always
counts. `to` is the bucket of the stage the `task.recorded` its
answer carries (a create with none: the stage it asked for).
`edited` names what the command set, not a diff: a list's prior values
aren't readable for every list. These are the only versions: the `@1`
ones spoke oxtask (its statuses, field names and note URIs)
and `task.transitioned` its status moves; V30 rewrote the logged
ones into these words (a transition beside its run's `state_changed`
dropped, the rest made `state_changed`) and they left core, with
oxplow's `OxtaskStatus`. Effects and SDK templates react to these
(`on: [task.state_changed]`, `where: { to: done }`).

**Features reach the UI as a model**: `v_capability_provider`
(`capability`, `provider`, `extension`, `features` JSON, `active`,
`title`, `source`, `available`, `chosen_by`) lists each capability's
implementations, restated whole by the app's `CapabilityRegistry`
(`crates/oxplow-app/src/capabilities.rs`) whenever what it holds or the
choices change. oxtask is the `oxplow:oxtask` built-in, which
`oxplow-bundled` declares (`implementations:`), with its features from
core's built-in table; an external provider's row is there while its
instance runs (`ProviderRegistry::publish`, a task tracker's
features as `ExternalTaskTracker` reads them). **`active`** is the
resolved implementation: a person's own choice (`.oxplow/personal.yaml`),
else the project's `activeProviders` (`{ tasks: <instance id> }`, a
person-only key), else the default (`oxplow`) — and every declared row
of a capability many implementations serve (`chosen_by = declared`,
work-tracking.md "Capabilities"); both are chosen in
Settings → Integrations → Capabilities (the `capabilities` foundation page, below). A choice that isn't
available falls to `none` (the task tracker is optional), listed with
`available = 0` and the active row's `chosen_by = fallback`. The
registry's `active()` resolves from the config as it is now, so a
person's choice applies to the very next `create`. **Every verb goes to
the active task tracker**, never another: a URI of another list's item is
refused (`` `oxplow:oxplow/task?id=ENG-12&provider=issues` is issues's, which isn't the active
task tracker (`oxtask`) ``), as it isn't visible either.

**None is a sink** (`tasks::none_provider`, registered by core
beside oxplow's): every verb succeeds with `{ tracked: false }` and
keeps nothing. It has every feature, takes any item's URI and any id
(`sink`), and the interface reads empty, so nothing that writes tasks is refused or retried while no list is active and no screen or
skill explains it. The typed client's `create` returns `None` for it
(the bundled repair effect skips: it files nothing); an effect's automatic retry is never
sent to a sink (`safe_to_resend`: the list it was meant for went
away). **Loose ids.** A task tracker declares what its ids look
like (`TaskTracker::id_pattern`, a regex matched whole: oxplow's
tasks `tsk\d+`, an external one its `providers:` entry's `id_pattern`).
A task field (`uri`, `parent_uri`, `target`, `task`, and
`reorder` / `move`'s URIs) holding a loose id that matches the active
list's is that list's item — `tsk12` is `oxplow:oxplow/task?id=tsk12&provider=oxtask` while
oxtask is active (`commands/task.rs` `with_loose_refs`,
before routing, over `TaskTrackers::loose_ref`, which compiles each
list's pattern once).
One that doesn't match is `Invalid` at its field, naming both shapes; an
empty `parent_uri` is left alone (it detaches). Free-text recognition (wikilinks, commit bodies) is still
core's. The
conformance suite runs with the provider under test active, and checks it. The desktop reads it with
`readCapabilityProviders(capability)` (`tasks.ts`, with `reads`) and
`featuresFor(providers, provider)` → `TaskTrackerFeatures` (the Rust type,
exported through the bindings), which turns every flag a provider
doesn't declare — or a provider that isn't listed — off.

**Settings → Integrations → Capabilities** (the foundation page `capabilities`, over `v_capability_provider`; `oxplow.capability.choose` writes the choice) is
generated from `v_capability_provider`: for each choosable capability
(`choosable`, `capability_title` and `optional` are core's
`CapabilitySpec`, on every row since v4) the active implementation and
why (`chosenNote`: personal, project, default, or which chosen one fell
back), the project's choice as radios ("The default" unsets the
capability's entry) and "Just for me" (`oxplow.config.set` / `oxplow.config.unset` with
`layer: personal`). An optional capability's `none` names what it turns
off: the enabled extensions' pages and hints that declare a `needs:` on
it (`offWithout`). A click is the confirmation (`activeProviders` is
person-only); a failure goes to the op-errors store.

## Conformance

**The UI with every enhancement off** is
`tests-e2e/specs/pages/plain.spec.ts`: the e2e project with only
oxplow-foundation on (`oxplow-bundled`, `e2e` and `github` disabled). What an
extension adds splits in two. What the page run decides and its JSON shows
— slot mounts (`PageRunner::mounts`), replacements (`RunNode.replaced_by`
and its `embed`), feature-gated actions, `needs` — is every foundation
fixture's, because a test's oxplow has only what is declared: the required
foundation, the extension tested, and what a fixture `requires:`. What only
the desktop decides — decorators (`ref-chip` / `row-badge`, `title="from
<ext>"`), extension commands in the ☰, panels in the rail — is the spec's:
it opens Git History, a commit and Uncommitted changes and asserts each
page's core content and no marker of another extension. The task tracker
itself (the `oxplow:oxtask` implementation) is bundled's, and so are its pages
(Board, Tasks, Backlog, a task) are `oxplow-bundled`'s: with only
foundation they are not offered, and the spec asserts that. Their
fixtures live in oxplow-bundled and cover them with the list. Another provider's item with every flag off gets
an `inactive` page from `task.yaml`'s `needs`, not a plain one.

`oxplow_app::task_tracker_conformance::suite(items, provider, features,
native, probe, actor)` — plain functions returning a `SuiteRun { findings,
left }` (`left`: the items it filed and didn't delete, for a person to
clean up in the provider's own system), writing
through the `Tasks` client (so it exercises the dispatching commands
a person and an agent run) — is what every provider must do: create
lands a row in `todo`'s default stage; an update changes only what it
names; the list's stages pass `stages_problem`, every declared stage
round-trips and reads back in its bucket, naming a bucket lands on its
default, an undeclared stage is refused, and core logs
`task.state_changed` for the create and every move to another
bucket; a parent resolves with `hierarchy` and is refused
without it; links and comments follow their features and read back
through `v_task_link` / `v_task_comment`; an open item of any
list can be assigned to the person and reordered in that queue (it ranks
ahead in `v_task.rank`, reads back in `assignee`) and released;
the log names the item in the interface's words, whichever list —
`task.created`, `edited` (the rename), `assigned`, `linked` and `commented` as the features allow, and `deleted`
for each item it deletes; reading the
provider back restates what its writes recorded — after a
`provider.sync` (`TaskTrackerProbe::sync`; nothing to read for oxplow's
own or a provider without collectors) every item it filed is the row it
was (P7.A7; the fake's `stale-read` hook is the red); a provider that
declares `idempotent_writes` keeps it — a create sent twice with one key
(through `TaskTrackerProbe::verbs`, the provider's `TaskVerbs`, since the
bus never re-sends a key itself) answers alike, another key is another
item, the first key sent again **after the provider's process restarts**
(`TaskVerbs::restart`) still answers alike — the promise outlives the
process, which is when the host re-sends — and after a read back two
items carry the run's own keyed title (`conformance keyed item <8 hex>`,
so a leftover from an earlier run against a real service never counts;
`TaskTrackerProbe::titled`). Every item it files, re-sends included, is
in the cleanup list before it is checked (tsk916). The fake's
`forget-keys` hook, and the fake without `OXPLOW_FAKE_STATE`, are the
red; and delete follows its feature, cleaning up what the suite filed
when the provider can. `native` is the provider's own fields for the items it files
(oxplow's test passes the actor's thread, so `in_progress` claims). A
`TaskTrackerProbe` reads back what the host recorded (`ServicesProbe` over
the database). It runs in-tree against oxplow's provider
(`oxplow_oxtask_are_a_conforming_provider`) and against an external one
through the host over the fake provider
(`the_tasks_suite_passes_through_the_host_over_the_fake`, P5.D3);
the kit (P5.D5) runs it against any provider. The probe runs the pump
once per settle, so the projection has landed before each read.

## External providers

`ExternalTaskTracker` ([providers.md](./providers.md)) is the
`TaskVerbs` of an enabled provider instance: each verb's input is
checked against the schema the provider declared for it (its `native`
fields included) before the process is called, and the
`task.recorded` events it returns reach `task` through the
projection. The verbs aren't commands of their own: `<id>.transition`
doesn't exist on the bus; the provider's **other** declared commands
(the fake's `estimate`) do, as `<id>.<name>`. An external write can land
at the tracker while the reply times out — the run then reports failure
though the item changed; the sync (P7.A3) restates it.

## A task's requests

A task links to the requests that worked on it (`request_task`: `request_id`,
`task_uri`, `basis`, `turn_id`, `at`; V16; the dataset `v_request_task` adds
the request's totals so far). The links are observed, never stated, core's
whichever task tracker is active (like assignment), and written in the
transaction of the evidence. Nothing opens, closes or moves a task from
them: what a task's state means is its tracker's opinion. The bases:

- `prompt`: a person's prompt in the request named the task (`@`-less
  `[oxplow uri …]` references, `uri::prompt_refs`; `log_prompt_tx`).
- `command`: the agent ran `oxplow.task.create|update|comment` on it in a
  turn; `transition`: it ran `oxplow.task.transition` (moved it between
  stages). The Work panel's Finished reads `assigned` and `transition`: a
  thread that only filed, commented on or was prompted with a task did not
  work it. `command.executed` of an agent actor is anchored to the turn
  open for its session (a person's or a page's command is in no turn and
  links nothing); `commands/record.rs`.
- `assigned`: it was in progress and assigned to the thread when a turn of the
  request ended (`close_turn_tx`) — the weakest.

A task's commits derive through its requests (`v_commit_request`;
[git-integration.md](./git-integration.md) "Linked to the requests that made
it"), not from a basis here. With no task tracker active nothing is
written. **Why a table and not `page_ref`:** `page_ref` is the references a
text contains (its source restates its slice), keyed `(source, target,
ref_type)` with no cascade; this link is evidence with a time, a basis and a
turn, written once by ingest, which cascades with its request and joins to
`v_request`. A prompt's mention of the task is still also a `prompt_mention`
edge in `page_ref`.

