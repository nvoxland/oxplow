# Work items

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C). oxplow's own tasks are one provider, `oxplow` — a
built-in list (`crates/oxplow-tasks`) called exactly like any other;
another provider is a **backend that replaces it**: the person picks the
active tracker, and every new item goes there. So it should have the
visibility the person wants for their work — usually still just theirs,
like a local tracker such as beads — not a team's tracker
([providers.md](./providers.md) "Which trackers are backends"). Reads are
SQL; writes are the `work_item.*` commands — **one write surface for every
provider** (P7.A1), which the bus dispatches to the item's provider.

## The model

**The interface.** Everything — the UI, core, extensions, the agent's
tools — reads work items through `v_work_item`, `v_work_item_link` and
`v_work_item_comment`, and writes them with the `work_item.*` commands;
nothing outside a work list's implementation reads that list's own
tables (`oxplow-dev` is the one development exception). oxplow's tasks
publish no dataset of their own — `v_task`, `v_task_link` and
`v_task_note` were removed; their own fields are `stage` and
`native`. The views show **the active work list's items only**: one
list at a time, whichever implementation it is, and nothing with none.

`v_work_item` (table `work_item`, V115) holds the items by URI,
`oxplow:oxplow/work_item?id=<id>&provider=<provider>`: `title`, `body`, its `stage`
(one the list declares, below) with the list's `stage_title` and its
bucket as `state`, the list's own `native` JSON, `parent_uri` (NULL when the parent is
deleted: a deleted item isn't a live one, so the dataset's parent
relationship holds; tsk572), `closed_at`, and core's assignment
(below): `assignee` and `rank`.
`v_work_item_link` is its links (`to_uri`, its `to_kind`, `link_type`):
to another item of the list (`blocks`, `discovered_from`, `relates_to`,
`duplicates`, `supersedes`, `replies_to`), or `about` any URI — what the
item is about (`oxplow:oxplow/collector?collector=triage/hot`, `oxplow:oxplow/effort?id=eff12`). A link's target
needn't resolve; it may name something deleted or an extension's resource
since uninstalled (resources.md "Using URIs"),
`v_work_item_comment` its comments. Core reads work items only through
these: a thread's assigned items (`v_work_item`), a work item's URI summary
(`ref_resolver`), a `[[…]]` link's check (`link_check`) and search
(`v_search_work_item`).

**Buckets and stages.** A work list declares its own **stages**, its
workflow in order — `[{ id, title, bucket }]`
(`oxplow_domain::work_items::StageDecl`, checked by `stages_problem`: ids
snake_case and unique, titles given, a stage in each of `todo`,
`in_progress` and `done`, and a stage named like a bucket in it) — and
core keeps six **buckets**, `Bucket`: `backlog` (filed, not ready),
`todo` (ready: what an idle thread may pull), `in_progress` (started),
`blocked` (waits on an answer), `done` and `canceled` (closed). Core acts
on buckets only — efforts, assignment and its release, the pool, Waiting
on Me — and the screens draw the list's stages: a board's columns, a
state picker, a Mark menu. **The first stage of a bucket is its
default**: an input naming the bucket (`to: done`) lands there, so an
agent or an effect speaks buckets and works with any list. A record
names only its stage; the projection looks its bucket up in the list's
declaration (`stage_bucket_tx`, from `capability_provider.stages_json`),
and an undeclared stage dead-letters. Published as
`v_capability_provider.stages`. A built-in's are core's table's
(`BuiltIn.stages`: oxplow's tasks' are its statuses, `backlog`, `ready`
"Ready", `in_progress`, `blocked`, `done`, `canceled`); an external
provider's are its `providers:` entry's `stages:`, which a work list must
declare. None's are the six buckets' names (`bucket_stages`).

**Declared fields.** What a list keeps in `native` beyond those columns
is declared, so screens render and edit it without knowing which list is
active: `[{ name, title, kind: enum|text|number, values? }]`
(`oxplow_domain::work_items::FieldDecl`, checked by `fields_problem`),
published as `v_capability_provider.fields`. A built-in's are core's
table's (`BuiltIn.fields`: oxplow's tasks declare `priority`, an enum of
urgent/high/medium/low); an external provider's are its `providers:`
entry's `fields:`. None declares none. How the rows are written and
oxplow's state mapping are in [data-model.md](./data-model.md) "`work_item`".

**One writer, one schema.** Every list's rows arrive by projection: its
`work_item.recorded@3 { item }` events — what each verb answers with,
and what a read back restates — carry the item as it now stands: URI,
title, body, stage, native fields, parent, deleted,
and, when the list keeps them, `links` and `comments`, each the whole
set (stated, the host restates it; absent, it keeps what it has). Who it
is assigned to and its place in their queue are never a list's
("Assignment"). The pump consumer `work_items.project`
(`crates/oxplow-app/src/work_items.rs`) upserts it by URI; a replay
restates the same row. Why one record event rather than state on each of
`created` / `edited` / `state_changed`: the "what happened" events stay
small and the same for every list, and the projection is one upsert
from one event type. A run's projections are delivered before the run
returns ([commands.md](./commands.md) "A run's projections are
delivered"), so a write reads back at once.

**oxplow's tasks are their own crate**, `crates/oxplow-tasks`, named by
nothing outside it but the built-in factory and `Services`' stores: the
task store and its links and notes (`store.rs`, `satellite.rs`), the
task types, the mapping of statuses and priority onto the interface
(`mapping.rs`), its answers to the verbs (`verbs.rs`: `create_tx`,
`update_tx`, `transition_tx`, `link_tx`, `comment_tx`, `delete_tx`,
`reorder_tx`, `move_tx`, each over a connection, the interface's input
and the actor, answering with its result, the verb that undoes it and the
tasks it changed), the record of a task (`record.rs`: the item whole,
from the task tables — links, notes, list and rank included) and
`OxplowTasks`, its `WorkItemVerbs`: each verb in a transaction of its own
over the task tables, answering with the `work_item.recorded` of every
task it changed (a reorder's renumbered neighbours too). A refused verb
rolls its transaction back. It writes nothing of core's: no `work_item*`
row (V29 dropped the triggers that did), no page ref, no event — core
logs the run, its canonical events and the records. The store's own
async writes (tests) reach the interface only when restated
(`test_fixtures::restate_task`).

**A built-in list registers by declaration.** `oxplow-foundation` declares
`implementations: [{ capability: work_items, id: oxplow, entry:
"oxplow:tasks" }]`; `work_items::register_built_ins` registers the
provider each declared built-in names (`built_in_provider`, the factory:
its features and id pattern from `capabilities::BUILT_INS`, its verbs
the crate's) when services are built and at every `capabilities::refresh`,
and unregisters it when nothing declares it — the task data stays, and
nothing reaches it. A built-in whose items' URIs carry a provider id
(`BuiltIn.provider`: `oxplow:tasks` is `oxplow`, as every
`oxplow:oxplow/work_item?id=tsk<n>&provider=oxplow` says) is declared under that id and no other;
`implementations.rs` refuses another, which would register a list that
refuses its own items.

**Page refs** for every list's item come from the interface: its body's
mentions, its links (`work_item_link:<type>`, the list's own types) and
its comments' mentions (`comment_*`, keyed by the item), restated by
`work_item_refs::restate_tx` on the item's events (the
`page_ref.work_item` consumer, after `work_items.project`) and by the
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
(`oxplow/work-next`). Each assignee — nobody included — has a queue,
the open items assigned to it in `rank` order. **A done or canceled item
is nobody's**: who worked on it is its efforts (`v_effort.work_item`,
`thread_id`), the same for every list. Filing assigns nobody unless the
create names an `assignee`; who filed it is the event's thread anchor.

The record is `work_item_assignment` (V7: `uri`, `assignee` NULL for
nobody, `rank`, `assigned_at`), kept by the `work_items.project` consumer
from core's **`work_item.assigned@1 { work_item, from?, to?, before?,
after? }`**: the item joins `to`'s queue next to the item named, or at its
end, and the queue is renumbered; a record of the item as done or
canceled, from whichever list, deletes its row. `v_work_item` joins it
(`assignee`, `rank`). The writes, all core's and in the bus's
transaction (`commands/work_item.rs`):

- `oxplow.work_item.assign { uri, to?, before?, after? }` — `to` a thread's
  URI or id, the person's URI, or null; a closed thread (or none) is
  refused at `/to`, a closed item at `/uri`. Undone by assigning it back
  to its place. With no list active (none) it answers `{ tracked: false }`.
- `oxplow.work_item.reorder { uri, before?, after? }` — its place in its
  assignee's queue; the neighbour must be in the same queue.
- `create { …, assignee? }` — checked first, kept from the list, and
  logged as `work_item.assigned` once the item exists.
- A verb that closes an assigned item logs `work_item.assigned` to nobody.
- `oxplow.thread.close` and `oxplow.stream.archive` release their threads'
  items to nobody.
- The effort policy: an agent that starts an item not assigned to its
  thread takes it (`.context/work-tracking.md` rule 1).
- Taking a handoff assigns its item to the recipient thread
  (`.context/agent-model.md` "Handoffs").

## Reading them in the UI

The desktop reads **only the interface**, whichever list is active —
never oxplow's task tables, ids or URIs; a guard test
(`apps/desktop/src/workItemInterface.guard.test.ts`) fails on a `v_task`
name, a built `oxplow/work_item` URI or `tsk` id parsing in desktop code.

`apps/desktop/src/workItems.ts` is the one read and write path:

- **`WorkItem`** is a `v_work_item` row — `uri`, `provider`, `title`,
  `body`, `state`, `parentRef`, `assignee` (null = nobody), `rank`,
  `closedAt`, the list's own fields as `native`, and `commentCount`
  (`v_work_item_comment`).
- **`readWorkItem`** and **`readWorkItemsByRef`** read the rest (Tasks
  and Task List are pages over `v_work_item`: an assignee's queue in
  order (`rank`, then `created_at`), an item with a child an epic);
  `readItemEfforts(ref)` an item's activity.
- **Writes** are `work_item.*` commands by uri: `createWorkItem` (no
  list named — it files on the active one; `native` carries the list's
  own fields), `updateWorkItem`, `transitionWorkItem`, `applyItemChange`
  (an edit from any surface: fields and/or state), `deleteWorkItem`,
  and `reorderWorkItems` (a drag's new order becomes one
  `oxplow.work_item.reorder` by neighbour, `placementFromOrder`).
- **The active list's profile** — `readWorkListProfile` /
  `useWorkListProfile` (one shared read, re-read on a switch): its
  provider, features, declared `fields`, `stages` and `idPattern`
  (`v_capability_provider`). A `WorkItem` carries its bucket (`state`,
  the `Bucket` union), its `stage` and `stageTitle`; colours and glyphs
  go by bucket (`stateColor`, `vizIcons`), words by stage. Screens offer only what it can do (reordering
  and assignment are core's, so always):
  epics, parents and "+ Item" with `hierarchy`, Comment… / Link… with
  `comments` / `links`, Delete with `delete`. **Declared fields render
  generically** (`components/WorkItemFields.tsx`: an enum as a pill
  picker — a `priority` enum with the priority glyph — text and number
  as inputs, a `read_only` field shown only): on list rows and Board
  cards, in the detail rail, in New Item, as bulk "Change …" actions,
  and as the Tasks page's filter chips. oxplow declares `priority` and
  a read-only `author` (who filed it). **`workItemRefOfMention`** turns
  a loose id in text into a URI by the list's `idPattern` (markdown
  `[[tsk42]]`, the effort header's link field) — none matches nothing.

Every read returns what it read (`reads`), and a consumer re-runs it
through `useRerunOnChange` — or `readsChanged` outside a component
(`useBackendSubscriptions`, the title cache) — when one of those datasets
changes. With **none** active every read is empty and the screens show
their empty states.

**One page for every item** (oxplow-bundled's `work-item` document page,
the renderer of `oxplow/work_item`; params `uri`, `thread_id`,
`stream_id`). Its rail is what the item is and what can be done to it:
About (its stage by its list's name with its bucket's glyph, who it's
assigned to, its epic, its list, its id, when it was made, changed and
closed; its right-click menu has Mark ▸ the list's stages and Assign to ▸
the stream's threads and You, each one action with `options_from`, the
current one checked, then Unassign and Delete), Stage (a form over
`oxplow.work_item.transition`, the replaceable state part) and Details
(the title and the list's declared fields, over `update`). Its body is
the description (a `markdown` viz, edited in place), the items under it
(an epic's: each with its glyph, its description's first sentence, its
stage and assignee), Activity (its efforts: the `item-efforts` page,
embedded), Comments and Links (the `work-item-comments` and
`work-item-links` parts, each declaring its list feature in `needs`, so a
list without one draws its section as needing it) and the
`oxplow/work-item.sections` slot, where bundled mounts Agent Tokens
(`task-tokens`) and Shared work (`item-shared-work`). See also: the Tasks
board of the thread it's assigned to (else the Task List at its stage)
and its epic. `work-item-actions` is the agents' page for one item (Mark
done, Record a block…), and `link-effort` the form behind an effort's
Link to Task…. The item's own
list's extension may replace its state part (`work_item.detail.state`,
decided by the page run from the item's `provider`); About's menu always offers the moves, so an item
can always move. **Custom views** need no provider: any extension may
replace the Tasks board (`work_item.board`), the Task List's list (`work_item.list`)
or an item's description editor (`work_item.detail`) — with a
page or component over `v_work_item` that changes items only through the
interface commands, so it works with whichever list is active; the person
picks whose shows (`replacements`, Settings → Integrations).
[extensions-slots.md](./extensions-slots.md) "Replacements" has the contracts. The `oxplow/work-item.sections` slot gets
`{ uri }`. Link…'s link type is free text (default `relates_to`): the
provider names its own types, and `about` fits any URI. Its target is
**any URI** (a work item's, `oxplow:oxplow/effort?id=eff12`, `oxplow:oxplow/collector?collector=x/y`); a link row
in Outbound has **Unlink** on its right-click menu (`oxplow.work_item.unlink`,
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
thread or stream whose efforts worked it), priority and search are its
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

The agent reads work items through the interface too, with no tool of
its own: `oxplow_query_sql` over `v_work_item` for a queue in order
(`WHERE assignee = … ORDER BY rank IS NULL, rank, created_at`),
`oxplow_read` on a work item's URI for one item with its `links` and
`comments`, the foundation page `oxplow/thread-work` (`oxplow_thread_work`:
what is assigned to a thread and what its efforts finished), and
`oxplow/work-next` (the tool `oxplow_work_next`, param `thread: "thr3"`)
for what to pick up next: the ready items assigned to the thread (an epic
with its ready descendants, or every ready non-epic item; open `blocks`
links hold an item), else the pool, by priority (`role: pool`). Starting
one assigns it to the thread. A controller agent routes work with
`oxplow.work_item.assign`. None of them belongs to an implementation, so the tool list never
changes with a switch; with none they read empty. A source
guard (`source_guards::only_oxplows_implementation_names_its_task_list`)
pins the few production files that may name oxplow's task store,
service or datasets.

## The capability

`oxplow_domain::work_items`:

- **`WorkItemsProvider`** (a struct): `id` (the URI segment),
  `features`, `verbs: Arc<dyn WorkItemVerbs>`, its `id_pattern`, and
  `sink` (none).
- **`WorkItemVerbs::invoke(actor, verb, input, idempotency_key) ->
  VerbOutcome { result, events, inverse? }`**: every list's verbs —
  oxplow's tasks (`OxplowTasks`), an extension's process
  (`ExternalWorkItems`), none (`Sink`) — run outside the bus's
  transaction; its inverse is named by its **verb**. The key is
  the caller's when it has one (an effect's step), else the host mints
  one ([providers.md](./providers.md) "Idempotency").
- **A blank title is refused** by `oxplow.work_item.create` and `update` (`INVALID` at `/title`), so a view's form shows the message under the field; the check is the command's, not any list's.
- **`WorkItemsFeatures`**: `hierarchy`, `comments`, `links`, `delete`,
  `idempotent_writes` (a write sent twice with one
  idempotency key is done once — [providers.md](./providers.md)
  "Idempotency").
- **`WorkItemsRegistry`** (`Services.work_items`): providers by name;
  `for_ref` picks one by the URI's provider segment, and an unknown one
  is refused naming the registered providers; `active()` names the
  provider every `create` files on, resolved each time by the capability
  registry from the config as it is now — no copy, so a person's choice
  applies to the very next `create` (see below).
- **`VERBS`**: `create`, `update`, `transition`, `link`, `unlink`,
  `comment`, `delete` — the capability's verbs.

**One write surface: the dispatching `work_item.*`** (P7.A1;
`commands/work_item.rs`). Each verb is an `External` command
([commands.md](./commands.md) "Tx and External") on the item's provider
— the URI's segment, or for `create` the active one — called through its
`WorkItemVerbs`, the same way for every list, with **one audit row**
`work_item.<verb>`. In a composite it is a step: a sequence of work-item
verbs runs as steps, not undoable as a whole. Before anything runs it
refuses: a URI of another list's item (`/ref`,
naming the active list), a parent, link target or place of another list
(`/parent_uri`, `/target`, `/before`, `/after`), and a feature the list
doesn't declare.
A `create` hands the list the input without its `assignee` (core's,
above), and the list's inverse is renamed to `work_item.<verb>`, so
an undo dispatches again. **The answer is the interface's**, terse and the
same for every list: `{ uri }` (a comment's adds `comment`, the list's id
for it), plus core's `state` when the verb put the item in one and
`link_warnings` for a body — never a list's own row; a reader reads the
item from `v_work_item`. A create's, or an update's that sets a body,
result carries its `link_warnings` (`LinkDeps::item_warnings`), for every
list.
`assign` and `reorder` aren't a list's verbs: they are core's
("Assignment"). A Rust client, **`work_items::WorkItems`**
(`Services::work_items_client()`), types the calls; the conformance
suite uses it. Extension commands compose the same verbs:
oxplow-bundled's Accept Review / Request Changes (P7.C5) comment on and
transition an effort's work item whatever its provider
([extensions-bundled.md](./extensions-bundled.md) "The review packet").

**The contract is the `v_work_item` columns**, one shape for every
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
  (`commands/work_item.rs` `resolve_stages`), and refuses anything else at
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
`oxplow.work_item.comment` and `oxplow.work_item.link` refuse a deleted task (tsk572).
A comment's `task_note.author` names who made it (`note_author`, tsk1000):
`user`, `agent` (a page acting for one included), `effect:<namespace>/<id>`
or `oxplow` — as a task's `author` is left empty for an effect or oxplow
(`task_author`), neither is shown as the person's.

**The interface's events are core's**, logged by `dispatching()`
(`canonical_events` in `commands/work_item.rs`) for every list alike,
from the verb, its input, the list's answer and the state it put the
item in — caused by the run's `command.executed`, subject the item,
anchored to the actor's thread. A list logs none of them itself
(oxplow's task store logs nothing); another list's answer adds its
`work_item.recorded`.

| Verb | Events |
|---|---|
| `create` | `work_item.created@2 { work_item, state }`, `work_item.state_changed@1 { work_item, to }` |
| `update` | `work_item.edited@2 { work_item, fields }` naming what the input set — `title`, `body`, `parent`, `native.<name>` — and `state_changed` when the state moved |
| `transition` | `state_changed` when the state moved |
| `link` | `work_item.linked@2 { work_item, target, link_type }` (subject both), unless the link was already there (`result.linked: false`) |
| `unlink` | `work_item.unlinked@1 { work_item, target, link_type }` (subject both), unless there was none (`result.unlinked: false`) |
| `comment` | `work_item.commented@2 { work_item, comment? }` — `comment` is the list's own id for it, when the answer names one (`result.comment`; oxplow: `not<n>`, the fake: `c<n>`) |
| `delete` | `work_item.deleted@2 { work_item }` |
| `reorder` / `move` | `edited` naming `rank` / `list` and `rank` |

Whether the state moved: core reads the item's state as the interface
shows it (`v_work_item.state`) before a transition or an update runs, and
logs `state_changed` only when the bucket of the stage its answer
recorded differs — a transition within a bucket moves nothing the effort
policy acts on; it logs `edited` naming `stage` instead. A create always
counts. `to` is the bucket of the stage the `work_item.recorded` its
answer carries (a create with none: the stage it asked for).
`edited` names what the command set, not a diff: a list's prior values
aren't readable for every list. These are the only versions: the `@1`
ones spoke oxplow's task list (its statuses, field names and note URIs)
and `work_item.transitioned` its status moves; V30 rewrote the logged
ones into these words (a transition beside its run's `state_changed`
dropped, the rest made `state_changed`) and they left core, with
oxplow's `TaskStatus`. An item's state opens and closes no effort
itself; the effort policy reacts to `state_changed`
(`.context/work-tracking.md`). Effects and SDK templates react to these
(`on: [work_item.state_changed]`, `where: { to: done }`).

**Features reach the UI as a model**: `v_capability_provider`
(`capability`, `provider`, `extension`, `features` JSON, `active`,
`title`, `source`, `available`, `chosen_by`) lists each capability's
implementations, restated whole by the app's `CapabilityRegistry`
(`crates/oxplow-app/src/capabilities.rs`) whenever what it holds or the
choices change. oxplow's tasks are the `oxplow:tasks` built-in, which
`oxplow-foundation` declares (`implementations:`), with its features from
core's built-in table; an external provider's row is there while its
instance runs (`ProviderRegistry::publish`, a work-items provider's
features as `ExternalWorkItems` reads them). **`active`** is the
resolved implementation: a person's own choice (`.oxplow/personal.yaml`),
else the project's `activeProviders` (`{ work_items: <instance id> }`, a
person-only key), else the default (`oxplow`) — and every declared row
of a capability many implementations serve (`chosen_by = declared`,
work-tracking.md "Capabilities"); both are chosen in
Settings → Integrations → Capabilities (the `capabilities` foundation page, below). A choice that isn't
available falls to `none` (the work list is optional), listed with
`available = 0` and the active row's `chosen_by = fallback`. The
registry's `active()` resolves from the config as it is now, so a
person's choice applies to the very next `create`. **Every verb goes to
the active work list**, never another: a URI of another list's item is
refused (`` `oxplow:oxplow/work_item?id=ENG-12&provider=issues` is issues's, which isn't the active
work list (`oxplow`) ``), as it isn't visible either.

**None is a sink** (`work_items::none_provider`, registered by core
beside oxplow's): every verb succeeds with `{ tracked: false }` and
keeps nothing. It has every feature, takes any item's URI and any id
(`sink`), and the interface reads empty, so nothing that writes work
items is refused or retried while no list is active and no screen or
skill explains it. The typed client's `create` returns `None` for it
(the bundled repair effect skips: it files nothing); an effect's automatic retry is never
sent to a sink (`safe_to_resend`: the list it was meant for went
away). **Loose ids.** A work list declares what its ids look
like (`WorkItemsProvider::id_pattern`, a regex matched whole: oxplow's
tasks `tsk\d+`, an external one its `providers:` entry's `id_pattern`).
A work-item field (`uri`, `parent_uri`, `target`, `work_item`, and
`reorder` / `move`'s URIs) holding a loose id that matches the active
list's is that list's item — `tsk12` is `oxplow:oxplow/work_item?id=tsk12&provider=oxplow` while
oxplow's tasks are active (`commands/work_item.rs` `with_loose_refs`,
before routing, over `WorkItemsRegistry::loose_ref`, which compiles each
list's pattern once; also `oxplow.effort.link` / `oxplow.effort.open`).
One that doesn't match is `Invalid` at its field, naming both shapes; an
empty `parent_uri` is left alone (it detaches). Free-text recognition (wikilinks, commit bodies) is still
core's. The
conformance suite runs with the provider under test active, and checks it. The desktop reads it with
`readCapabilityProviders(capability)` (`workItems.ts`, with `reads`) and
`featuresFor(providers, provider)` → `WorkItemsFeatures` (the Rust type,
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
page's core content and no marker of another extension. The work list
itself (the `oxplow:tasks` implementation) is foundation's, but its pages
(Board, Tasks, Backlog, a work item) are `oxplow-bundled`'s: with only
foundation they are not offered, and the spec asserts that. Their
fixtures live in oxplow-bundled and cover them with the list. Another provider's item with every flag off gets
an `inactive` page from `work-item.yaml`'s `needs`, not a plain one.

`oxplow_app::work_items_conformance::suite(items, provider, features,
native, probe, actor)` — plain functions returning a `SuiteRun { findings,
left }` (`left`: the items it filed and didn't delete, for a person to
clean up in the provider's own system), writing
through the `WorkItems` client (so it exercises the dispatching commands
a person and an agent run) — is what every provider must do: create
lands a row in `todo`'s default stage; an update changes only what it
names; the list's stages pass `stages_problem`, every declared stage
round-trips and reads back in its bucket, naming a bucket lands on its
default, an undeclared stage is refused, and core logs
`work_item.state_changed` for the create and every move to another
bucket; a parent resolves with `hierarchy` and is refused
without it; links and comments follow their features and read back
through `v_work_item_link` / `v_work_item_comment`; an open item of any
list can be assigned to the person and reordered in that queue (it ranks
ahead in `v_work_item.rank`, reads back in `assignee`) and released;
the log names the item in the interface's words, whichever list —
`work_item.created`, `edited` (the rename), `assigned`, `linked` and `commented` as the features allow, and `deleted`
for each item it deletes; reading the
provider back restates what its writes recorded — after a
`provider.sync` (`WorkItemsProbe::sync`; nothing to read for oxplow's
own or a provider without collectors) every item it filed is the row it
was (P7.A7; the fake's `stale-read` hook is the red); a provider that
declares `idempotent_writes` keeps it — a create sent twice with one key
(through `WorkItemsProbe::verbs`, the provider's `WorkItemVerbs`, since the
bus never re-sends a key itself) answers alike, another key is another
item, the first key sent again **after the provider's process restarts**
(`WorkItemVerbs::restart`) still answers alike — the promise outlives the
process, which is when the host re-sends — and after a read back two
items carry the run's own keyed title (`conformance keyed item <8 hex>`,
so a leftover from an earlier run against a real service never counts;
`WorkItemsProbe::titled`). Every item it files, re-sends included, is
in the cleanup list before it is checked (tsk916). The fake's
`forget-keys` hook, and the fake without `OXPLOW_FAKE_STATE`, are the
red; and delete follows its feature, cleaning up what the suite filed
when the provider can. `native` is the provider's own fields for the items it files
(oxplow's test passes the actor's thread, so `in_progress` claims). A
`WorkItemsProbe` reads back what the host recorded (`ServicesProbe` over
the database). It runs in-tree against oxplow's provider
(`oxplow_tasks_are_a_conforming_provider`) and against an external one
through the host over the fake provider
(`the_work_items_suite_passes_through_the_host_over_the_fake`, P5.D3);
the kit (P5.D5) runs it against any provider. The probe runs the pump
once per settle, so the projection has landed before each read.

## External providers

`ExternalWorkItems` ([providers.md](./providers.md)) is the
`WorkItemVerbs` of an enabled provider instance: each verb's input is
checked against the schema the provider declared for it (its `native`
fields included) before the process is called, and the
`work_item.recorded` events it returns reach `work_item` through the
projection. The verbs aren't commands of their own: `<id>.transition`
doesn't exist on the bus; the provider's **other** declared commands
(the fake's `estimate`) do, as `<id>.<name>`. An external write can land
at the tracker while the reply times out — the run then reports failure
though the item changed; the sync (P7.A3) restates it.
