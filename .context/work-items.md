# Work items

Tasks, issues, tickets — whatever a provider tracks — behind one
capability (P5.C, `target-architecture.md` §6). oxplow's own tasks are the
built-in provider, `oxplow`; an issue tracker is another provider (P5.D
brings external ones). Reads are SQL; writes go through a provider.

## The model

`v_work_item` (table `work_item`, V115) holds every provider's items by
ref, `work_item:<provider>:<id>`: `title`, `body`, a canonical `state`
(`todo`, `in_progress`, `blocked`, `done`, `canceled`), the provider's own
`native_state` and `native` JSON, and `parent_ref`. `v_task` stays
oxplow's native view. How the rows are written, the state mapping and the
cascade trigger are in [data-model.md](./data-model.md) "`work_item`".

There are two writers, one schema:

- **oxplow's rows** are restated from the task row by the task cores in
  the same transaction (`task_store::project_work_item_tx`), so they
  never disagree with `v_task`; `v_work_item`'s own `sql` test checks it.
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
refuse another provider's. Dispatching them across providers by ref is
decided with a real second provider (P7). `effort.open` asks the ref's
provider's features: refused when it declares `in_progress_opens_effort`,
open to an unregistered provider's item.

## Conformance

`oxplow_app::work_items_conformance::suite(provider, probe, actor)` —
plain functions returning `Finding`s — is what every provider must do:
create lands a `todo` row; every canonical state round-trips (and
`in_progress` opens exactly one effort iff the provider says so); a
parent resolves with `hierarchy` and is refused without it; links and
comments follow their features; a foreign ref is refused naming this
provider; `work_item.created` / `transitioned` (and `linked` /
`commented`) name the item in the log. A `WorkItemsProbe` reads back what
the host recorded (`ServicesProbe` over the database). It runs in-tree
against oxplow's provider (`oxplow_tasks_are_a_conforming_provider`) and,
in P5.D, against an external one through the host and the kit. Undo and
delete aren't on the trait, so they aren't in the suite: undo is the
bus's (its tests), delete is oxplow's own (the model's agreement test).
