# Work items

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C, `target-architecture.md` §6). oxplow's own tasks are the
built-in provider, `oxplow`; an issue tracker is another provider (P5.D
brings external ones). Reads are SQL; writes go through a provider.

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
`clear_recently_finished`) are gone. The MCP task tools stay, and write
the same way: `reorder_tasks` runs `work_item.reorder` once per listed
item as the agent (the listed items in order, ahead of the rest), so a
reorder is audited and dense like the UI's; `delete_task` is gone, since
`work_item.delete` is destructive and an agent never confirms one (an
agent cancels or archives). `TaskService::reorder` and `soft_delete` went
with them (P6 review, tsk609).

The **Board** (`page:board`, `components/Board/WorkBoard.tsx`) shows
items as cards in one column per canonical state (archived tasks left
out). Drag a card to a column, or right-click → Move To, to transition
it through its provider (`transitionWorkItem`: oxplow's
`work_item.transition` with oxplow's status, another provider's
`<provider>.transition` with the canonical state — `workItemCommand`).
Every card opens its item's page (`workItemTabRef`).

**Another provider's item has a page of its own** (P6b.C3,
`pages/WorkItemPage.tsx`; oxplow's tasks keep `TaskPage`):
`refFromTabId("work_item:<p>:<id>")` is a `work_item` tab with a `ref`
payload. It reads the item (`readWorkItem`) and the provider's features
(`readCapabilityProviders` → `featuresFor`) and shows title, state
(canonical and native), body and Move To; its Parent only with
`hierarchy`, **Comment…** only with `comments` and **Link…** only with
`links`, each run through `personCommands` as `<provider>.comment` /
`<provider>.link` (the field keeps its text until the run succeeds —
`personCommands.run` returns whether it ran). Comments on another
provider's item are write-only here: `v_comment` is oxplow's store. Both
pages mount the `work_item.detail.body` and `work_item.detail.sidebar`
slots with `{ ref, task_id }` (`task_id` null for another provider's
item).

## The capability

`oxplow_domain::work_items`:

- **`WorkItemsProvider`** (async): `provider()` (the ref segment),
  `features()`, `create`, `update`, `transition(ref, Canonical(state) |
  Native(string))`, `link`, `comment`.
- **`WorkItemsFeatures`**: `hierarchy`, `comments`, `links`,
  `in_progress_opens_effort` (moving an item to `in_progress` opens its
  effort itself).
- **`WorkItemsRegistry`** (`Services.work_items`): providers by name;
  `for_ref` picks one by the ref's provider segment, and an unknown one
  is refused naming the registered providers.

**`OxplowWorkItems`** implements the trait over the bus: each call runs a
`work_item.*` command as the given actor (one write path; audited and
policy-checked), filing new items on the actor's thread. It holds the bus
weakly — the bus's commands hold the registry the provider sits in.
Canonical `todo` is oxplow's `ready`; native states are oxplow's statuses
(`archived` included).

The `work_item.*` commands ([commands.md](./commands.md)) are the oxplow
provider's: they take canonical refs (`ref`, `parent_ref`, `target`) and
refuse another provider's; `work_item.comment` and `work_item.link`
refuse a deleted task (tsk572). Dispatching them across providers by ref is
decided with a real second provider (P7). `effort.open` asks the ref's
provider's features: refused when it declares `in_progress_opens_effort`,
open to an unregistered provider's item.

**Features reach the UI as a model** (P6b.C2): `v_capability_provider`
(`capability`, `provider`, `extension`, `features` JSON, `active`) lists
each capability's providers with the flags **the provider** declares —
never a manifest. Core's (`work_items/oxplow`, `vcs/git`,
`knowledge/oxplow`) are restated at boot (`capabilities::publish_core`,
which also drops a previous run's external rows); an external provider's
row is written while its instance runs (`ProviderRegistry::publish`, a
work-items provider's features as `ExternalWorkItems` reads them) and
removed when it stops. `active` is P7's hook for choosing a capability's
active provider; every row is `1` today. The desktop reads it with
`readCapabilityProviders(capability)` (`workItems.ts`, with `reads`) and
`featuresFor(providers, provider)`, which turns every flag a provider
doesn't declare — or a provider that isn't listed — off.

## Conformance

`oxplow_app::work_items_conformance::suite(provider, probe, actor)` —
plain functions returning `Finding`s — is what every provider must do:
create lands a `todo` row; every canonical state round-trips (and
`in_progress` opens exactly one effort iff the provider says so); a
parent resolves with `hierarchy` and is refused without it; links and
comments follow their features; a foreign ref is refused naming this
provider; every write logged an event naming the item (the provider's own kinds:
oxplow's `work_item.created` / `transitioned` / `linked` /
`commented`, an external provider's `work_item.recorded`). A `WorkItemsProbe` reads back what
the host recorded (`ServicesProbe` over the database). It runs in-tree
against oxplow's provider (`oxplow_tasks_are_a_conforming_provider`) and
against an external one through the host over the fake provider
(`the_work_items_suite_passes_through_the_host_over_the_fake`, P5.D3);
the kit (P5.D5) runs it against any provider. The probe runs the pump
once per settle, so the projection has landed before each read.

## External providers

`ExternalWorkItems` ([providers.md](./providers.md)) is the capability
over an enabled provider instance: each call runs the provider's
`<id>.create` / `update` / `transition` / `link` / `comment` command
through the bus (inputs: `create { title, body, parent_ref? }`,
`update { ref, title?, body?, parent_ref? }`, `transition { ref, to }`
with a canonical or native state, `link { ref, target, link_type }`,
`comment { ref, body }`), refuses another provider's ref naming its own,
and refuses an unsupported feature before calling. The provider's
`work_item.recorded` events reach `work_item` through the projection. Undo and
delete aren't on the trait, so they aren't in the suite: undo is the
bus's (its tests), delete is oxplow's own (the model's agreement test).
