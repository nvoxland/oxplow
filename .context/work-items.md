# Work items

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C, `target-architecture.md` §6). oxplow's own tasks are the
built-in provider, `oxplow`; an issue tracker is another provider (P5.D
brings external ones). Reads are SQL; writes are the `work_item.*`
commands — **one write surface for every provider** (P7.A1), which the
bus dispatches to the item's provider.

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
backlog or everything, in list order; each read returns its `reads` so a
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
(`workItemTabRef`). The oxplow task writes (`createTask`, `updateTask`)
send a status as oxplow's `native_state` and thread / priority under
`native`.

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
  `in_progress_opens_effort` (moving an item to `in_progress` opens its
  effort itself), `idempotent_writes` (a write sent twice with one
  idempotency key is done once — [providers.md](./providers.md)
  "Idempotency").
- **`WorkItemsRegistry`** (`Services.work_items`): providers by name;
  `for_ref` picks one by the ref's provider segment, and an unknown one
  is refused naming the registered providers; `active()` /
  `set_active()` name the provider a `create` without one files on
  (from `activeProviders`; see below).
- **`VERBS`**: `create`, `update`, `transition`, `link`, `comment`,
  `delete` — the capability's verbs.

**One write surface: the dispatching `work_item.*`** (P7.A1;
`commands/work_item.rs`). Each verb is a `Dispatch` command
([commands.md](./commands.md) "Tx, External and Dispatch"): the bus
routes by the item's provider — the ref's segment, or for `create` the
named (else active) provider — to oxplow's `Tx` core in the bus's
transaction, or to the provider's `ExternalVerbs` through its process,
with **one audit row** `work_item.<verb>` either way. The route also
refuses, before anything runs: an unregistered provider (`/ref`, naming
the registered), a parent or link target of another provider
(`/parent_ref`, `/target`), and a feature the provider doesn't declare.
An external run hands the provider the input less `provider` and renames
its inverse to `work_item.<verb>`, so an undo dispatches again.
`reorder` and `move` stay oxplow's own `Tx` (they place a task in
oxplow's lists). A Rust client, **`work_items::WorkItems`**
(`Services::work_items_client()`), types the calls; the conformance
suite uses it. Extension commands compose the same verbs:
oxplow-review's Accept Review / Request Changes (P7.C5) comment on and
transition an effort's work item whatever its provider
([extensions.md](./extensions.md) "oxplow-review").

**The contract is the `v_work_item` columns**, one shape for every
provider (a provider's verb receives the same input, less `provider`):

- `create { provider?, title, body?, parent_ref?, state?, native_state?,
  native? }` — no `provider` files on the active one, which must be
  running (never a silent fallback);
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
asked); `native` holds `{ thread?, priority? }` (`deny_unknown_fields`;
`thread` only on `create` — a task changes lists with `work_item.move`).
A `native_state` alone (no `state`) is a valid update or create: that is
how the task writes send a status. A person's link (no thread of their
own) belongs to the linked task's thread, else the target's.
`work_item.comment` and `work_item.link` refuse a deleted task (tsk572).
`effort.open` asks the ref's provider's features: refused when it
declares `in_progress_opens_effort`, open to an unregistered provider's
item.

**Features reach the UI as a model** (P6b.C2): `v_capability_provider`
(`capability`, `provider`, `extension`, `features` JSON, `active`) lists
each capability's providers with the flags **the provider** declares —
never a manifest. Core's (`work_items/oxplow`, `vcs/git`,
`knowledge/oxplow`) are restated at boot (`capabilities::publish_core`,
which also drops a previous run's external rows); an external provider's
row is written while its instance runs (`ProviderRegistry::publish`, a
work-items provider's features as `ExternalWorkItems` reads them) and
removed when it stops. **`active`** is the capability's active provider
(P7.A2): the project's `activeProviders` (`{ work_items: <instance id>
}` — a provider's default instance has the provider's id, P9.B1 —, a
person-only config key — an agent's change is a proposal; Settings →
Integrations offers it as "Active for work items", oxplow's own being the
key unset), oxplow's own when it names none; a capability nobody can swap
(`vcs`, `knowledge`) has its one provider active. `capabilities::is_active`
is the one rule the rows follow (`publish_core`, `ProviderRegistry::
publish`), and `capabilities::apply_active` restates it — the registry's
`active()` and the column — at boot and on every reconcile (each config
change). A `work_item.create` naming no `provider` files on the active
one; one that isn't running is `Invalid` at `/provider` ("the active
work-items provider isn't running: …"), never a fallback to oxplow. The desktop reads it with
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
row reports lands there (`in_progress` opens exactly one effort iff the
provider says so); a parent resolves with `hierarchy` and is refused
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
item, and after a read back two items carry the keyed title
(`WorkItemsProbe::titled`; P10, the fake's `forget-keys` hook is the
red); and delete follows its feature, cleaning up what the suite filed
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
