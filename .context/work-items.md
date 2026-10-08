# Work items

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C). oxplow's own tasks are the
built-in provider, `oxplow`; another provider (P5.D brings external ones)
is a **backend that replaces it**: the person picks the active tracker,
and every new item goes there (tsk1058). So it should have the
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
tables (`v_task` is oxplow's tasks'; `oxplow-dev` is the one development
exception). The views show **the active work list's items only**: one
list at a time, whichever implementation it is, and nothing with none.

`v_work_item` (table `work_item`, V115) holds the items by ref,
`work_item:<provider>:<id>`: `title`, `body`, a canonical `state`
(`todo`, `in_progress`, `blocked`, `done`, `canceled`), the list's own
`native_state` and `native` JSON, `parent_ref` (NULL when the parent is
deleted: a deleted item isn't a live one, so the model's parent
relationship holds; tsk572), and (V17) `thread_id` — the list it's on, a
thread's or the backlog (NULL) — `rank` on that list and `closed_at`.
`v_work_item_link` is its links (`blocks`, `discovered_from`,
`relates_to`, `duplicates`, `supersedes`, `replies_to`),
`v_work_item_comment` its comments. Core reads work items only through
these: `get_thread_context`'s items, a work item's ref summary
(`ref_resolver`), a `[[…]]` link's check (`link_check`) and search
(`v_search_work_item`).

**Declared fields.** What a list keeps in `native` beyond those columns
is declared, so screens render and edit it without knowing which list is
active: `[{ name, title, kind: enum|text|number, values? }]`
(`oxplow_domain::work_items::FieldDecl`, checked by `fields_problem`),
published as `v_capability_provider.fields`. A built-in's are core's
table's (`BuiltIn.fields`: oxplow's tasks declare `priority`, an enum of
urgent/high/medium/low); an external provider's are its `providers:`
entry's `fields:`. None declares none. How the rows are written, the state mapping and the
cascade trigger are in [data-model.md](./data-model.md) "`work_item`".

There are two writers, one schema:

- **oxplow's rows** are restated from the task row by the task cores in
  the same transaction (`oxplow_tasks::store::project_work_item_tx`), so they
  never disagree with `v_task`; their links and comments follow
  `task_link` / `task_note` by trigger.
  That includes every row a core touches on the side: `place_task_tx`
  renumbers the moved item's neighbours and restates each one it changed
  (`native.sort_index`), checked by the reorder test's `stale_native_rows`.
- **Another provider's rows** arrive by projection: its
  `work_item.recorded@2 { item }` events (the item as it now stands —
  ref, title, body, canonical and native state, native fields, parent,
  deleted, and — when the provider keeps them — `rank`, `links` and
  `comments`, each the whole set: stated, the host restates it; absent,
  it keeps what it has; `@1` lacked the three and upcasts as "not
  stated") are upserted by ref by the pump consumer `work_items.project`
  (`crates/oxplow-app/src/work_items.rs`). A replay restates the same
  row. An `oxplow` record is refused (dead-lettered): those rows are the
  task cores' alone. Why one record event rather than state on each of
  `created` / `edited` / `state_changed`: the "what happened" events stay
  small and the same for every list, and the projection is one upsert
  from one event type.

**oxplow's tasks are their own crate**, `crates/oxplow-tasks`: the
store, the links and notes, the service, the task types, the mapping of
statuses and priority onto the interface (`mapping.rs`) and its answers
to the verbs (`verbs.rs`: `create_tx`, `update_tx`, `transition_tx`,
`link_tx`, `comment_tx`, `delete_tx`, `reorder_tx`, `move_tx`, each over a
connection, the interface's input and the actor, answering with its
result and the verb that undoes it). Core's `commands/work_item.rs`
calls them in the bus's transaction and names their inverses as the
commands; nothing else names the crate.

**Page refs** for every list's item come from the interface: its body's
mentions, its links (`work_item_link:<type>`, the list's own types) and
its comments' mentions (`comment_*`, keyed by the item), restated by
`work_item_refs::restate_tx` on the item's events (the
`page_ref.work_item` consumer, after `work_items.project`) and by the
boot repair. A list writes no page refs of its own.

## Reading them in the UI

The desktop reads **only the interface**, whichever list is active —
never oxplow's task tables, ids or refs; a guard test
(`apps/desktop/src/workItemInterface.guard.test.ts`) fails on `v_task`,
a built `work_item:oxplow:` ref or `tsk` id parsing in desktop code.

`apps/desktop/src/workItems.ts` is the one read and write path:

- **`WorkItem`** is a `v_work_item` row — `ref`, `provider`, `title`,
  `body`, `state`, `parentRef`, `threadId` (null = the backlog), `rank`,
  `closedAt`, the list's own fields as `native`, and `commentCount`
  (`v_work_item_comment`).
- **`readWorkList(thread | null)`** reads a thread's list or the backlog
  in list order (`rank`, then `created_at`) and buckets it into a
  **`WorkList`** by state (an item with a child on the list is an epic;
  done and canceled share `done`), with the thread's followups.
  `readWorkItem`, `readWorkItemsByRef` and `readWorkItems` (the Board's
  scopes: a thread, the backlog, everything) read the rest;
  `readItemEfforts(ref)` an item's activity.
- **Writes** are `work_item.*` commands by ref: `createWorkItem` (no
  list named — it files on the active one; `native` carries the list's
  own fields), `updateWorkItem`, `transitionWorkItem`, `applyItemChange`
  (an edit from any surface: fields and/or state), `deleteWorkItem`,
  `reorderWorkItems` (a drag's new order becomes one `oxplow.work_item.reorder`
  by neighbour, `placementFromOrder`) and `moveWorkItem`.
- **The active list's profile** — `readWorkListProfile` /
  `useWorkListProfile` (one shared read, re-read on a switch): its
  provider, features, declared `fields` and `idPattern`
  (`v_capability_provider`). Screens offer only what it can do: drag
  reordering with `ordering`, the backlog and thread moves with `lists`,
  epics, parents and "+ Item" with `hierarchy`, Comment… / Link… with
  `comments` / `links`, Delete with `delete`. **Declared fields render
  generically** (`components/WorkItemFields.tsx`: an enum as a pill
  picker — a `priority` enum with the priority glyph — text and number
  as inputs, a `read_only` field shown only): on list rows and Board
  cards, in the detail rail, in New Item, as bulk "Change …" actions,
  and as the Tasks page's filter chips. oxplow declares `priority` and
  a read-only `author` (who filed it). **`workItemRefOfMention`** turns
  a loose id in text into a ref by the list's `idPattern` (markdown
  `[[tsk42]]`, the effort header's link field) — none matches nothing.

Every read returns what it read (`reads`), and a consumer re-runs it
through `useRerunOnChange` — or `readsChanged` outside a component
(`useBackendSubscriptions`, the title cache) — when one of those models
changes. With **none** active every read is empty and the screens show
their empty states.

**One page for every item** (`pages/WorkItemPage.tsx`, routed from any
`work_item:<provider>:<id>` tab, payload `ref`): title and body edited
in place, the rail's state pill and declared fields, activity (its
efforts), backlinks and outbound (`canonicalIdForTarget` maps the ref to
the graph's `<provider>:<id>`), comments targeting `work_item` /
`<provider>:<id>`, and the feature-gated actions above. The item's own
list's extension may add its state view (`work_item.detail.state`,
`Replaceable`), beside the state pill — never instead of it, so an item
can always move. The `work_item.detail.body` / `.sidebar` slots get
`{ ref }`. Link…'s link type is free text (default `relates_to`): the
provider names its own types.

The **Board** (`page:board`, `components/Board/WorkBoard.tsx`) shows
items as cards in one column per canonical state. Drag a card to a
column, or right-click → Move To, to transition it through
`personCommands` (one person path: its confirmation and its error
reporting). List rows and Board cards share one drag
(`WORK_ITEM_DRAG_MIME`: refs, each one's title and state), which the
agent terminal reads as context refs. **New Item** files on the active
list: title, body, a canonical starting state (todo or blocked), the
list's editable fields and — with `hierarchy` — a parent.

## Reading them for the agent

The agent's work-item tools read the interface too
(`crates/oxplow-app/src/work_item_reads.rs`, MCP `list_work_items`,
`get_work_item`, `next_work_item`): a list's items in order, one item
with its links and comments, and what to pick up next (an epic with its
ready descendants, or every ready non-epic item; open `blocks` links
hold an item). None of them belongs to an implementation, so the tool
list never changes with a switch; with none they read empty. A source
guard (`source_guards::only_oxplows_implementation_names_its_task_list`)
pins the few production files that may name oxplow's task store,
service or models.

## The capability

`oxplow_domain::work_items`:

- **`WorkItemsProvider`** (a struct): `id` (the ref segment),
  `features`, and `external: Option<Arc<dyn ExternalVerbs>>` — `None`
  for oxplow's own (its verbs are the `work_item.*` commands' `Tx`
  cores), the provider's verbs for an external one.
- **`ExternalVerbs::invoke(actor, verb, input, idempotency_key) ->
  VerbOutcome { result, events, inverse? }`**: a provider outside the
  bus's transaction; its inverse is named by its **verb**. The key is
  the caller's when it has one (an effect's step), else the host mints
  one ([providers.md](./providers.md) "Idempotency").
- **`WorkItemsFeatures`**: `hierarchy`, `comments`, `links`, `delete`,
  `idempotent_writes` (a write sent twice with one
  idempotency key is done once — [providers.md](./providers.md)
  "Idempotency").
- **`WorkItemsRegistry`** (`Services.work_items`): providers by name;
  `for_ref` picks one by the ref's provider segment, and an unknown one
  is refused naming the registered providers; `active()` names the
  provider every `create` files on, resolved each time by the capability
  registry from the config as it is now — no copy, so a person's choice
  applies to the very next `create` (see below).
- **`VERBS`**: `create`, `update`, `transition`, `link`, `comment`,
  `delete` — the capability's verbs.

**One write surface: the dispatching `work_item.*`** (P7.A1;
`commands/work_item.rs`). Each verb is a `Dispatch` command
([commands.md](./commands.md) "Tx, External and Dispatch"): the bus
routes by the item's provider — the ref's segment, or for `create` the
active one — to oxplow's `Tx` core in the bus's
transaction, or to the provider's `ExternalVerbs` through its process,
with **one audit row** `work_item.<verb>` either way. The route also
refuses, before anything runs: a ref of another list's item (`/ref`,
naming the active list), a parent, link target or place of another list
(`/parent_ref`, `/target`, `/before`, `/after`), and a feature the list
doesn't declare.
An external `create` hands the provider the input less `thread` (the host
anchors the item to it) and renames its inverse to `work_item.<verb>`, so
an undo dispatches again.
`reorder` (feature `ordering`: an item's place on its list, read as
`rank`) and `move` (feature `lists`: to a thread's list or the backlog,
read as `thread_id`) are interface verbs like the rest; a provider that
declares the feature must declare the verb. A Rust client, **`work_items::WorkItems`**
(`Services::work_items_client()`), types the calls; the conformance
suite uses it. Extension commands compose the same verbs:
oxplow-bundled's Accept Review / Request Changes (P7.C5) comment on and
transition an effort's work item whatever its provider
([extensions.md](./extensions.md) "The review packet").

**The contract is the `v_work_item` columns**, one shape for every
provider (a provider's verb receives the same input, less `create`'s
`thread`):

- `create { title, body?, parent_ref?, state?, native_state?, native?,
  thread? }` — **always on the active tracker** (tsk1058): the person
  chose it, and nothing a caller says — a person, an agent, an effect or
  oxplow itself — files anywhere else. One that isn't running is an
  error, never a fallback. `thread` is the thread it's filed on: absent,
  an agent's own; a person's without one has none (oxplow: the backlog);
- `update { ref, title?, body?, parent_ref? ("" detaches), state?,
  native_state?, native? }`;
- `transition { ref, to, native_state? }` — `to` canonical; a
  `native_state` must map to it;
- `link { ref, target, link_type }` (the provider names its link types),
  `comment { ref, body }`, `delete { ref }` (Destructive; only with
  `features.delete`).

oxplow's mapping: its status is its `native_state` (`ready` is `todo`;
`archived` rides on `done` or `canceled` — archiving as `done` a task that
wasn't completed passes through `done` first, so the row reads as
asked); `native` holds `{ priority? }` (`deny_unknown_fields`; a task
changes lists with `oxplow.work_item.move`).
A `native_state` alone (no `state`) is a valid update or create: that is
how the task writes send a status. A person's link (no thread of their
own) belongs to the linked task's thread, else the target's.
`oxplow.work_item.comment` and `oxplow.work_item.link` refuse a deleted task (tsk572).
A comment's `task_note.author` names who made it (`note_author`, tsk1000):
`user`, `agent` (a lens acting for one included), `effect:<extension>/<id>`
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
| `link` | `work_item.linked@2 { work_item, target, link_type }` (subject both) |
| `comment` | `work_item.commented@2 { work_item, comment? }` — `comment` is the list's own id for it, when the answer names one (`result.comment`; oxplow: `not<n>`, the fake: `c<n>`) |
| `delete` | `work_item.deleted@2 { work_item }` |
| `reorder` / `move` | `edited` naming `rank` / `list` and `rank` |

Whether the state moved: oxplow's is read from `work_item` before and
after in the bus's transaction; for an external provider, whose prior
state oxplow can't read, a create, a transition and an update naming a
state each count, `to` taken from the `work_item.recorded` its answer
carries (a create with none: the state it asked for, else `todo`).
`edited` names what the command set, not a diff: a list's prior values
aren't readable for every list. The `@1` versions spoke oxplow's task
list (`created { status }`, `edited` with `description` / `priority` /
`thread` / `position`, `commented { comment: "task_note:…" }`) and
upcast to `@2`'s words; `work_item.transitioned@1 { from, to }` (oxplow
statuses) stays registered for the log's history but nothing logs it —
`state_changed` replaces it. An item's state opens and closes no effort
itself; the effort policy reacts to `state_changed`
(`.context/work-tracking.md`). Effects and SDK templates react to these
(`on: [work_item.state_changed]`, `where: { to: done }`).

**Features reach the UI as a model**: `v_capability_provider`
(`capability`, `provider`, `extension`, `features` JSON, `active`,
`title`, `source`, `available`, `chosen_by`) lists each capability's
implementations, restated whole by the app's `CapabilityRegistry`
(`crates/oxplow-app/src/capabilities.rs`) whenever what it holds or the
choices change. oxplow's tasks are the `oxplow:tasks` built-in, which
`oxplow-bundled` declares (`implementations:`), with its features from
core's built-in table; an external provider's row is there while its
instance runs (`ProviderRegistry::publish`, a work-items provider's
features as `ExternalWorkItems` reads them). **`active`** is the
resolved implementation: a person's own choice (`.oxplow/personal.yaml`),
else the project's `activeProviders` (`{ work_items: <instance id> }`, a
person-only key), else the default (`oxplow`); both are chosen in
Settings → Pieces (`PiecesSection.tsx`, below). A choice that isn't
available falls to `none` (the work list is optional), listed with
`available = 0` and the active row's `chosen_by = fallback`. The
registry's `active()` resolves from the config as it is now, so a
person's choice applies to the very next `create`. **Every verb goes to
the active work list**, never another: a ref of another list's item is
refused (`` `work_item:issues:ENG-12` is issues's, which isn't the active
work list (`oxplow`) ``), as it isn't visible either.

**None is a sink** (`work_items::none_provider`, registered by core
beside oxplow's): every verb succeeds with `{ tracked: false }` and
keeps nothing. It has every feature, takes any item's ref and any id
(`sink`), and the interface reads empty, so nothing that writes work
items is refused or retried while no list is active and no screen or
skill explains it. The typed client's `create` returns `None` for it
(`plugin_repair` files nothing); an effect's automatic retry is never
sent to a sink (`safe_to_resend`: the list it was meant for went
away). **Loose ids.** A work list declares what its ids look
like (`WorkItemsProvider::id_pattern`, a regex matched whole: oxplow's
tasks `tsk\d+`, an external one its `providers:` entry's `id_pattern`).
A work-item field (`ref`, `parent_ref`, `target`, `work_item`, and
`reorder` / `move`'s refs) holding a loose id that matches the active
list's is that list's item — `tsk12` is `work_item:oxplow:tsk12` while
oxplow's tasks are active (`commands/work_item.rs` `with_loose_refs`, on
every path: routing, the transaction, the provider; also `oxplow.effort.link` /
`oxplow.effort.open`). One that doesn't match is `Invalid` at its field, naming
both shapes. Free-text recognition (wikilinks, commit bodies) is still
core's. The
conformance suite runs with the provider under test active, and checks it. The desktop reads it with
`readCapabilityProviders(capability)` (`workItems.ts`, with `reads`) and
`featuresFor(providers, provider)` → `WorkItemsFeatures` (the Rust type,
exported through the bindings), which turns every flag a provider
doesn't declare — or a provider that isn't listed — off.

**Settings → Pieces** (`PiecesSection.tsx`, `piecesModel.ts`) is
generated from `v_capability_provider`: for each choosable capability
(`choosable`, `capability_title` and `optional` are core's
`CapabilitySpec`, on every row since v4) the active implementation and
why (`chosenNote`: personal, project, default, or which chosen one fell
back), the project's choice as radios ("The default" unsets the
capability's entry) and "Just for me" (`oxplow.config.set` / `oxplow.config.unset` with
`layer: personal`). An optional capability's `none` names what it turns
off: the enabled extensions' lenses and hints that declare a `needs:` on
it (`offWithout`). A click is the confirmation (`activeProviders` is
person-only); a failure goes to the op-errors store.

## Conformance

**The UI with every enhancement off** (P6b.C6):
`apps/desktop/src/pages/CapabilityUi.smoke.test.tsx` renders
`WorkItemPage` (another provider's item, every flag false), the Board,
the commit page, uncommitted changes and history with no extensions,
and asserts each page's core content and the absence of every slot
section, the Commands menu, decorations and feature-gated actions; then,
with one extension, that the P6b mounts receive their params.

`oxplow_app::work_items_conformance::suite(items, provider, features,
native, probe, actor)` — plain functions returning a `SuiteRun { findings,
left }` (`left`: the items it filed and didn't delete, for a person to
clean up in the provider's own system), writing
through the `WorkItems` client (so it exercises the dispatching commands
a person and an agent run) — is what every provider must do: create
lands a `todo` row; an update changes only what it names; every
canonical state round-trips, and moving again to the native state the
row reports lands there, and core logs `work_item.state_changed` for
the create and every move to another state; a parent resolves with `hierarchy` and is refused
without it; links and comments follow their features and read back
through `v_work_item_link` / `v_work_item_comment`; `reorder` follows
`ordering` (the reordered item ranks ahead in `v_work_item.rank`) and
`move` follows `lists` (moved to the backlog, its `thread_id` is NULL);
the log names the item in the interface's words, whichever list —
`work_item.created`, `edited` (the rename; the other item's reorder and
move), `linked` and `commented` as the features allow, and `deleted`
for each item it deletes; reading the
provider back restates what its writes recorded — after a
`provider.sync` (`WorkItemsProbe::sync`; nothing to read for oxplow's
own or a provider without collectors) every item it filed is the row it
was (P7.A7; the fake's `stale-read` hook is the red); a provider that
declares `idempotent_writes` keeps it — a create sent twice with one key
(through `WorkItemsProbe::verbs`, the host's `ExternalVerbs`, since the
bus never re-sends a key itself) answers alike, another key is another
item, the first key sent again **after the provider's process restarts**
(`ExternalVerbs::restart`) still answers alike — the promise outlives the
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
`ExternalVerbs` of an enabled provider instance: each verb's input is
checked against the schema the provider declared for it (its `native`
fields included) before the process is called, and the
`work_item.recorded` events it returns reach `work_item` through the
projection. The verbs aren't commands of their own: `<id>.transition`
doesn't exist on the bus; the provider's **other** declared commands
(the fake's `estimate`) do, as `<id>.<name>`. An external write can land
at the tracker while the reply times out — the run then reports failure
though the item changed; the sync (P7.A3) restates it.
