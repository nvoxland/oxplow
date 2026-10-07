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

`v_work_item` (table `work_item`, V115) holds every provider's items by
ref, `work_item:<provider>:<id>`: `title`, `body`, a canonical `state`
(`todo`, `in_progress`, `blocked`, `done`, `canceled`), the provider's own
`native_state` and `native` JSON, and `parent_ref` (NULL when the
parent is deleted: a deleted item isn't a live one, so the model's
parent relationship holds; tsk572). `v_task` stays
oxplow's native view. How the rows are written, the state mapping and the
cascade trigger are in [data-model.md](./data-model.md) "`work_item`".

There are two writers, one schema:

- **oxplow's rows** are restated from the task row by the task cores in
  the same transaction (`task_store::project_work_item_tx`), so they
  never disagree with `v_task`; `v_work_item`'s own `sql` test checks it.
  That includes every row a core touches on the side: `place_task_tx`
  renumbers the moved item's neighbours and restates each one it changed
  (`native.sort_index`), checked by the reorder test's `stale_native_rows`.
- **Another provider's rows** arrive by projection: its
  `work_item.recorded@1 { item }` events (the item as it now stands —
  ref, title, body, canonical and native state, native fields, parent,
  deleted) are upserted by ref by the pump consumer `work_items.project`
  (`crates/oxplow-app/src/work_items.rs`). A replay restates the same
  row. An `oxplow` record is refused (dead-lettered): those rows are the
  task cores' alone. Why one record event rather than state on each of
  `created` / `edited` / `transitioned`: the "what happened" events stay
  small and oxplow's own need nothing new, and the projection is one
  upsert from one event type.

## Reading them in the UI

`apps/desktop/src/workItems.ts` (P6.E1a) is the UI's one read path:
`v_work_item` joined to `v_task` for oxplow's own fields (thread,
`sort_index`, priority, author, note count), scoped to a thread, the
backlog or everything, in list order. A thread's scope is
`v_work_item.thread_id` (v2, tsk1041): an oxplow task's own thread, and
an outside tracker's item **the thread it was filed on** — `create`'s
common `thread`, else an agent's own (`filing_thread`, tsk1058), carried
as the `work_item.recorded` envelope's anchor and kept by the projection
from the item's first record (`work_item.filed_in_thread`, V167) — so it
shows on that thread's Board and in the rail's Work panel ("On your
tracker"). The tracker never sees the thread: it is oxplow's record. Each read returns its `reads` so a
page re-runs with `useRerunOnChange`. Writes are `work_item.*` commands
(`transitionWorkItem`; the task pages reorder and move through `reorderTasks` / `moveTask`). `modelIds.ts`
converts the models' integer ids to the UI's `thr3` / `tsk42`.

It is also the **only** way the UI reads and writes oxplow's tasks
(P6.E1b): `Task`, `ThreadWorkState` and `BacklogState` are its types over
`v_task` (with `author`, not the vestigial `created_by`);
`readThreadWork`, `readBacklog`, `readTask`, `readTasksById`,
`readTaskEfforts` and `readRecentlyFinished` read the models;
`createTask`, `updateTask`, `deleteTask`, `reorderTasks` (a drag's new
order becomes one `work_item.reorder` by neighbour, `placementFromOrder`)
and `moveTask` run commands. Every read returns what it read (`reads`;
`ThreadWorkState` and `BacklogState` carry theirs), and a consumer
re-runs it through `useRerunOnChange` — or `readsChanged`, outside a
component (`useBackendSubscriptions`, the title caches) — when one of
those models changes: the one rerun rule, with no list of "task models"
to keep in step with the queries (P6 review, tsk610). The typed task RPCs
(`get_thread_work_state`,
`get_backlog_state`, `list_backlog`, `get_task`, `upsert_task`,
`create_task`, `update_task`, `delete_task`, `reorder_tasks`, `move_task`,
`list_work_item_efforts`, `list_recently_finished`,
`clear_recently_finished`) are gone, and so are the MCP task-write tools
(`create_task`, `update_task`, `complete_task`, `upsert_task`,
`transition_tasks`, `reorder_tasks`, `file_epic_with_children`, P8.A10):
an agent runs the same `work_item.*` commands through MCP `run_command`
as itself, so its reorders are `work_item.reorder { ref, before?, after?
}` by neighbour, audited and dense like the UI's. An agent has no delete,
since `work_item.delete` is destructive and an agent never confirms one
(an agent cancels or archives). `TaskService::reorder` and `soft_delete` went
with them (P6 review, tsk609).

The **Board** (`page:board`, `components/Board/WorkBoard.tsx`) shows
items as cards in one column per canonical state (archived tasks left
out). Drag a card to a column, or right-click → Move To, to transition
it (`transitionWorkItem`: `work_item.transition { ref, to: <canonical
state> }` for any provider — the bus dispatches it). Like Comment… and
Link… it runs through `personCommands` (one person path: its
confirmation and its error reporting). Every card opens its item's page
(`workItemTabRef`). **The New Task page files on the active tracker**
(`createTaskInput`, tsk1059): the canonical `state` (Ready is `todo`) and
`create`'s common `thread`, which any tracker takes; a parent epic and
priority are oxplow's own, so the page offers and sends them only while
oxplow's list is the active tracker (`activeProviderOf` over
`v_capability_provider`, re-read when it changes). `updateTask` sends a
status as oxplow's `native_state` and priority under `native`.

**Another provider's item has a page of its own** (P6b.C3,
`pages/WorkItemPage.tsx`; oxplow's tasks keep `TaskPage`):
`refFromTabId("work_item:<p>:<id>")` is a `work_item` tab with a `ref`
payload. It reads the item (`readWorkItem`) and the provider's features
(`readCapabilityProviders` → `featuresFor`) and shows title, state
(canonical and native), body and Move To; its Parent only with
`hierarchy`, **Comment…** only with `comments` and **Link…** only with
`links`, each an `InlinePromptStrip` run through `personCommands` as
`work_item.comment` / `work_item.link` (the strip keeps its text until
the run succeeds — `personCommands.run` returns whether it ran), and a
rail **Delete** only with `delete` (an `InlineConfirm`, which is the
person's confirmation of the destructive `work_item.delete`). Link…'s
link type is free text (default `relates_to`): the provider names its
own types, so oxplow's enum isn't offered as a list. The tab is titled
with the item's title (`usePageTitle`), and the page has backlinks and
outbound like `TaskPage` (`canonicalIdForTarget` maps the ref to the
graph's `<provider>:<id>`). Comments on another
provider's item are write-only here: `v_comment` is oxplow's store. Both
pages mount the `work_item.detail.body` and `work_item.detail.sidebar`
slots with `{ ref, task_id }` (`task_id` null for another provider's
item).

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
refuses, before anything runs: an unregistered provider (`/ref`, naming
the registered), a parent or link target of another provider
(`/parent_ref`, `/target`), and a feature the provider doesn't declare.
An external `create` hands the provider the input less `thread` (the host
anchors the item to it) and renames its inverse to `work_item.<verb>`, so
an undo dispatches again.
`reorder` and `move` stay oxplow's own `Tx` (they place a task in
oxplow's lists). A Rust client, **`work_items::WorkItems`**
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
changes lists with `work_item.move`).
A `native_state` alone (no `state`) is a valid update or create: that is
how the task writes send a status. A person's link (no thread of their
own) belongs to the linked task's thread, else the target's.
`work_item.comment` and `work_item.link` refuse a deleted task (tsk572).
A comment's `task_note.author` names who made it (`note_author`, tsk1000):
`user`, `agent` (a lens acting for one included), `effect:<extension>/<id>`
or `oxplow` — as a task's `author` is left empty for an effect or oxplow
(`task_author`), neither is shown as the person's.

**`work_item.state_changed@1 { work_item, to }`** is core's, logged by
`dispatching()` for every provider alike, subject the item, anchored to
the actor's thread: a `create` always; a `transition` or an `update`
whose state changed (oxplow's state is read before and after in the
bus's transaction; for an external provider, whose prior state oxplow
can't read, one that names a state counts, `to` taken from the
`work_item.recorded` its answer carries). An item's state opens and
closes no effort itself; the effort policy reacts to this event
(`.context/work-tracking.md`).

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
person-only key), else the default (`oxplow`). A choice that isn't
available falls to `none` (the work list is optional), listed with
`available = 0` and the active row's `chosen_by = fallback`. The
registry's `active()` resolves from the config as it is now, so a
person's choice applies to the very next `create`. Every
`work_item.create` files on the active work list; with none active it is
`Invalid` ("no work list is active …"), never another list. The
conformance suite runs with the provider under test active, and checks it. The desktop reads it with
`readCapabilityProviders(capability)` (`workItems.ts`, with `reads`) and
`featuresFor(providers, provider)` → `WorkItemsFeatures` (the Rust type,
exported through the bindings), which turns every flag a provider
doesn't declare — or a provider that isn't listed — off.

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
without it; links and comments follow their features; every write that
changed the item logged an event naming it (oxplow's
`work_item.created` / `edited` / `transitioned` / `linked` /
`commented`, an external provider's `work_item.recorded`); reading the
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
