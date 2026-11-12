# Shared models (contract, being built)

**Status: being built.** Built so far (`crates/oxplow-db/src/model_store.rs`,
V29; `crates/oxplow-app/src/models.rs`; manifest parsing in
`extension/extensions/shared_models.rs`):

- `models:` / `fills:` / `fields:` in the manifest; the registry of the
  enabled extensions' models (`ModelRegistry::of`), swapped in with the
  vocabulary; each model's table and views made in the publication;
  its change event registered as the owner's declared type with core's
  schema (`models::change_event_schema`).
- The store: the table and `v_<ns>_<model>` / `_all` views, diffing
  upsert / delete / rename / snapshot, quarantine, tombstones (revival,
  `sweep_tombstones_tx`), the removal guard, before/after values, fields
  kept apart by namespace, edges from markdown / stated refs / uri
  columns with `file_pin` / `dir_pin` pins.
- A collector's `models` / `fields` / `cursor` output applied in its run's
  transaction, one change event per changed row.
- `v_ref_pin` and `snapshot.ref_drifted@1`.

Also built: `search:` (a generated `v_<ns>_search_<model>`, the resource
searched through it unless it names its own `searchable:`),
`v_model_field_decl` and the long field view `v_<ns>_<model>_field`,
`oxplow.model.set_fields`, and the observer triggers (F2,
[extensions-jobs.md](./extensions-jobs.md) "Triggers for observers").

Also built: `oxplow.refs.verify`; a rebuild copies the rows in place; a
disabled store's rows are hidden; a store id two extensions claim refuses
the later one; a run names the rows it quarantined.

Also built: children ("Children" below).

Also built: the edge repair over models (`model_store::repair_edges_tx`,
run in the publication when a model's edge code, declaration or the
vocabulary changed: `models::repair_edges_if_needed_tx`, recorded as
`model_repair:<model>`); a stated row keeps its refs (`refs` column) so
the repair restates them as written.

Not yet: the provider `read` adapter, the owner's
`oxplow.model.write`, bare-link patterns, the tombstone sweep's schedule, and the shipped-extension plumbing (F7).
This doc is the contract for the rest. The other pieces
have their contracts in their own docs:

- triggers (`files`, `sqlite`, `start`): [extensions-jobs.md](./extensions-jobs.md) "Triggers for observers";
- an extension's own SQLite file and the scoped file service: [extensions-permissions.md](./extensions-permissions.md) "Store services" (built);
- named action points and the store filter: [extensions-slots.md](./extensions-slots.md) "Action points" (built);
- the manifest keys: [extensions-manifest.md](./extensions-manifest.md) "`models:` and `fills:`";
- the new fixture kinds: [extensions-sdk.md](./extensions-sdk.md) "Fixtures for observers".

Collectors are named by their extension's namespace (`<namespace>/<id>`),
and a dashed name's default namespace uses underscores, so a fixture
syncs `shelf_folder/books`, not `shelf-folder/books`.

## The idea

```
tool's store ──watch / poll / event──▶ store's collector ──models output──▶ shared model
(a SQLite file, a folder, a provider)        ▲                              (rows of every store)
                                             │ triggers it
                          store's commands ──┘ write the tool only (its file, its folder)
shared pages: read-only views over the model · action points · annotations keyed by item URI
```

- A **shared model** is a table that one extension (the **owner**)
  declares and other extensions (the **stores**) fill. Bundled owns the
  task, task link, task comment, task stage and knowledge page models.
- **One writer per store: its collector.** A store's commands write only
  the tool (its SQLite file, its folder, its API); the collector observes
  that and states the model. So a store's model rows are never written by
  two paths at once, and a snapshot can't undo a newer write. A
  provider's `read` stream is its collector.
- **Writes are declarative output**, as entity rows are today. A
  collector returns what to write; the host applies it after the script,
  in one transaction. No script writes during its run, so `check` can dry
  run every example, and an `exec` collector fills a model the same way
  as a Starlark one.
- Shared pages read the model and never write it. Writes are each
  store's own commands (on an item's menu, in a slot, or at a named
  action point).
- Real-time watching is optional. A store that only reconciles at start
  is correct, only late.

## Declaring a model (the owner)

```yaml
models:
  - name: task                 # table model__<ns>__task, view v_<ns>_task
    resource: task             # its rows are this resource (declared in resources:)
    open: true                 # other extensions may fill it (they must say so in fills:)
    key: id                    # one column: the store's native id
    store_param: provider      # the URI param that names the store (default: store)
    change_event: tasks.task.changed   # declared in event_types:
    bare_link: { pattern_from: fills }  # how a loose id in text resolves (below)
    columns:
      id: text
      title: text
      body: { type: text, markdown: {} }
      state: { type: text, values: [backlog, todo, in_progress, blocked, done, canceled] }
      parent: { type: uri, edge: task.parent }
      closed_at: time
    children:                  # rows that belong to one task (decided 2026-11-12)
      links:
        key: [to, type]        # unique within the parent
        columns:
          to: uri
          type: { type: text, values: [blocks, discovered_from, relates_to, duplicates, supersedes] }
        edge: { to: to, type_from: type, prefix: "task_link:" }
      comments:
        key: [id]
        columns: { id: text, author: text, at: time, body: { type: text, markdown: {} } }
```

- **Implicit columns** on every row: `store` (text), `uri` (text),
  `fields` (JSON, "Fields"), `written_at` (time, set by oxplow, ms),
  `removed_at` (time, "Removal"). A top-level model's URI is the declared
  resource with params `<store_param>` and `id`: tasks keep today's
  `oxplow:oxplow/task?id=<id>&provider=<store>`, knowledge pages today's
  `oxplow:oxplow/knowledge?id=<id>&store=<store>`, so stored URIs do not
  move. A child's rows have no URI of their own: they are named by their
  parent's URI and their key (`parent`, then the key columns, in
  `v_<ns>_<model>_<child>`).
- **Column types:** `text`, `int`, `real`, `bool`, `time`, `uri` (one
  resource URI), `uris` (a JSON list of URIs). `values:` makes a text
  column an enum, checked on write.
- **Edges from columns.** A `uri` / `uris` column with `edge: <type>`
  projects `page_ref` edges from the row's URI with that ref type. A
  child's `edge: { to, type_from, prefix }` projects one edge per row, from
  its parent's URI to its `to` column, typed `prefix + <value of
  type_from>` (or a fixed `ref_type:`), so a task link is the edge
  `task_link:blocks` it is today.
- **Markdown bodies.** A column with `markdown:` is parsed as oxplow's
  markdown (`page_ref_projections::body_edges`: `file_mention`,
  `dir_mention`, `wikilink`, `task_mention`, `finding_mention`,
  `commit_mention`), restated in the same transaction as the row, by diff.
  A child's markdown column (a comment) projects onto the parent:
  `comment_*` types from the parent's URI, as comment mentions are today.
  - `markdown: { pins: true }` makes its file and dir refs **pinned
    refs**, ref types `file_pin` / `dir_pin` (not `file_mention`, so a
    reader tells them apart): new ones pinned to the primary stream's
    latest snapshot, existing ones keep their pin, an edge verified under
    a still-cited `dir_pin` is kept on a rewrite (today's
    `knowledge_record::write_page_tx`). Staleness is `v_ref_pin`
    (`source`, `source_resource`, `path`, `kind` file|dir,
    `pinned_snapshot_id`, `latest_snapshot_id`, `stale`): a file is stale
    when its primary snapshot is newer than its pin or it was never
    pinned; a dir is stale when any file under it is.
  - A pinned source drifting is the generic event `snapshot.ref_drifted@1 {
    source, snapshot, paths }` (today's `knowledge.page.drifted`, for any
    pinned source), so counts follow drift and not every snapshot.
  - **Verify** is core's `oxplow.refs.verify { uri, files? }`: re-pin the
    named (or all) pinned edges of a source to now, checked against its
    `file_pin` / `dir_pin` edges (cited, or under a cited dir). It never
    writes the store. It is refused for a source with no pinned edges yet
    (a body not projected yet), saying so.
  - A store whose tool states its own links (a provider) sends `refs:
    [uri…]` in the row (implicit, not a column): then the markdown column
    is not parsed, each ref becomes the same typed edge the parser would
    give (file/dir refs pinned when the column pins).
  - The page-ref boot repair covers every model's markdown and edge
    columns, keyed on the vocabulary too (edges depend on which ids
    resolve).
- **Bare links.** A loose id in text (`tsk12`, `[[slug]]`) resolves by
  the stores' declared patterns: each `fills:` entry may declare
  `id_pattern:` (a regex); two enabled stores whose patterns overlap is a
  load error naming both. `[[slug]]` resolves to the store the model's
  owner names as `default_store` (bundled: `oxwiki`), else is left as
  text.
- **Search.** The resource's `searchable: <dataset>` stays the mechanism
  (it can scope by stream, drop archived items, index ids and comments).
  `search: { title, body }` on a model only generates a default
  searchable dataset `v_search_<ns>_<model>` when the resource names
  none. Shipped extensions (foundation, bundled, oxtask, oxwiki) are
  indexed unbounded, as foundation is today; others keep the row and
  body bounds.
- **The change event** is declared by the owner (`event_types:`):
  `{ uri, store, id, op: upserted | removed | renamed, changed: [column…],
  before: {col: value}, after: {col: value}, cause: trigger | reconcile
  | fields, renamed_from? }` — `before` / `after` hold the changed
  non-markdown columns (a markdown column is named in `changed` only).
  Subject the row's URI; anchors from the causing event. One per row that
  changed; an unchanged row logs none. `cause: reconcile` (a `start`
  snapshot) is for catching up: the owner's reactions that mean "this
  happened now" (linking a request, assigning on start) act only on
  `trigger` and `fields`. A fields change names each field that moved,
  `fields.<namespace>.<name>`, in `changed`, with its value in `before` /
  `after` (an assignment is `fields.oxplow.assignee`). A change to a parent's children logs the
  parent's event with `changed: [<child>]`.

## Filling a model (a store)

```yaml
requires: [oxplow-bundled]
fills:
  - model: oxplow/task          # <owner namespace>/<model>
    stores: [oxtask]            # store ids this extension writes; default [<namespace>]
    id_pattern: "tsk[0-9]+"     # how its ids look in text (optional)
    stages:                     # a task store's own stages (bundled's task model asks for them)
      - { id: backlog, title: Backlog, state: backlog }
    fields:
      - { name: priority, title: Priority, kind: enum, values: [urgent, high, medium, low] }
```

- A store id is owned by one extension. Two extensions naming the same
  store id is a load error. A provider instance's store id is its
  instance id.
- **The output.** A collector of the store returns, beside `cursor`:

```json
{"models": [
  {"model": "oxplow/task", "store": "oxtask",
   "upsert": [{"id": "t1", "title": "Fix it", "state": "todo", "fields": {"priority": "high"}}],
   "delete": ["t0"],
   "rename": [{"from": "t9", "to": "t10"}]},
  {"model": "oxplow/task", "store": "oxtask", "snapshot": [ ...every row... ]}
]}
```

  - `upsert` writes rows by key; a missing optional column is NULL.
  - `delete` removes keys (tombstones, "Removal").
  - `rename` re-keys an item: its URI, its edges, its fields, every
    annotation keyed by its URI move with it, and one `renamed` change
    event is logged. A file renamed in a folder is a rename, not a delete
    and an add.
  - `snapshot` states every row of that store: rows of the store not in
    it are removed. It is the full reconcile (`on: start`). A snapshot
    that would remove more than half of a store's rows, and more than 10,
    is held as a problem on the collector (nothing applied) unless it
    says `"confirm_removals": true` (a folder missing during a checkout
    must not empty a store).
  - **Bad rows are quarantined.** A row that fails its checks (an unknown
    enum value, a wrong type, an undeclared field) is skipped and
    reported as a problem naming the row and column; the rest is
    written. A quarantined key is never counted as removed by a snapshot.
    A malformed output (not the shape above) fails the run.
  - It diffs against what is stored and writes only what changed (the
    rule of the asset refill and the page-ref repair), and logs one
    change event per changed row, in the run's transaction (with its
    `collector_run` row and `collector.synced`).
  - The run's result counts `{upserted, removed, renamed, unchanged,
    quarantined}` per model.
- **Cursor.** A collector may return `"cursor": {...}`; it is kept in
  `collector_run.cursor_json` and given back as `x["cursor"]` on the next
  run. A failed run keeps the old cursor.
- **Providers (decided 2026-11-12: build the adapter).** A provider
  process's `read` stream is turned into the same `models` output for its
  instance's store (its instance id), applied with `models::apply_tx`, so
  an existing provider tracker (Linear) keeps showing its items with no
  change to the provider program; its writes become its own commands (one
  per provider operation, as today). `TasksProjection` and
  `KnowledgeRecords` go once the adapter stands in for them.
  **Built (adapter, beside the projections).** `providers/model_records.rs`
  (`records.models`, an async pump consumer) turns each `task.recorded` /
  `knowledge.page.recorded` of a running provider instance into a
  `ModelOutput` for `oxplow/task_record` / `oxplow/knowledge_record`
  (store = the instance id, `upsert` of one row or `delete`), applied with
  `apply_tx`: cause `trigger`, the record's anchors, source
  `provider:<instance>`. A task row has its stage's title and bucket (as the
  instance declares its stages), the first-seen `closed_at` / `created_at`,
  the instance's declared `native` fields as `fields`, and `links` /
  `comments` when the record states them (a link whose type the model's
  enum lacks, or whose target is no ref, is dropped with a warning); a page
  row's stated `uris` are its `refs`. The fills come from the registry:
  `ModelRegistry::with_provider_fills` adds, from the generation's
  capability rows (`PreparedCapabilities::rows`, called in
  `PreparedExtensionDatabase::with_vocabulary`), one fill per available
  external instance (task_tracker -> task_record, knowledge ->
  knowledge_record), written as the provider's extension; the views list
  its store because the fill is in the swapped registry. Bundled off (the
  models undeclared) -> no fill, the consumer returns without error.

## A script that composes a store's write (decided 2026-11-12, built)

oxtask surfaced it: a script runs once, before its steps, so its `result`
and `inverse` can't use what its steps answered — a minted task id, so
an agent can't learn the id it filed, and `create` / `comment` can't be
undone. To build:

- **`finish(x)`**: a script command's script may also define `finish`.
  After all its steps have landed, the bus calls it with `x["input"]`,
  `x["actor"]` and `x["steps"]` (`[{ name, input, result }]`, each step's
  answer: a store write's `rows` carry the minted id), and it returns `{
  result?, inverse? }`, which become the run's result and its undo
  (replacing what `transform` said). Pure, like `transform`: no writes, no
  composed commands. A run that stops partway calls no `finish`.
  Built: `extensions-commands.md` "`finish(x)`" has the rules for the
  inverse (a steps run checks it after finish returns; an in-transaction run,
  which undoes itself, refuses one and rolls back) and for reads.
- **`x["actor"]`** for `transform` and `finish`: who runs it (`{ kind:
  agent | person | system, thread?: <uri>, uri: <uri> }`), so a store
  records its author reliably.

## A store's folder and worktree streams (decided 2026-11-12)

`.oxplow/` is gitignored, so a store's folder (oxwiki's `.oxplow/oxwiki/`)
exists only in the primary worktree as far as oxplow is concerned: the
`files` trigger follows the primary, and `oxplow.extension_files.write`
resolves a path against the project root, never the caller's checkout.
So a store's own command (oxwiki's tool, from any stream) always writes the
primary's folder: no refusal for an agent in a worktree stream, and nothing
written into a worktree's copy. An agent editing a worktree's copy with its
own file tools isn't refused either; it just isn't seen, and the store's
skill says to use its tool. oxwiki stays on markdown files (decided
2026-11-12: agents and people read, grep and edit them with their own
tools). To build: a one-time hint when an agent in a non-primary
worktree writes under a store's declared folder ("this isn't seen; use the
store's tool"), not a refusal.

## Attribution (to settle in Phase 3)

A store's collector writes the model, so its change events carry the
collector run's anchors: an event-triggered run has its event's, a
`files` / `sqlite` run has none, and it can land after the command (and the
turn) that changed the tool. Linking a request to a task "by the change
falling in a turn's window" needs better: the plan is that
`oxplow.extension_store.write` and `oxplow.extension_files.write` fire the
composing extension's observers directly with the command's anchors and
cause, so the change event is the command's. Built: a store's write is
observed as its command, and its change events (and `set_fields'`) carry the
command's thread and open turn. The cause is `reconcile` for a `start`
run and for any `snapshot` output (it states the whole store: catching up,
whatever ran it), else `trigger`. There is no "first run" rule: the app runs
a store's `start` when it activates it (and the fixture harness does the
same before each fixture), so the store's later incremental runs are
`trigger`, and filing an item in a turn is that turn's change.

## Bundled's models (the target of the vertical slice)

Draft, to build next (bundled owns them; namespace `oxplow`):

```yaml
models:
  - name: task
    resource: task             # oxplow/task, as today
    open: true
    key: id
    store_param: provider      # today's URIs: oxplow:oxplow/task?id=<id>&provider=<store>
    change_event: oxplow.task.changed
    search: { title: title, body: body }
    columns:
      id: text
      title: text
      body: { type: text, markdown: {} }
      state: { type: text, values: [backlog, todo, in_progress, blocked, done, canceled] }
      stage: text              # the store's own stage
      stage_title: text
      parent: { type: uri, edge: task.parent }
      closed_at: time
      created_at: time
      updated_at: time
    children:
      links: { key: [to, type], columns: { to: uri, type: { type: text, values: [blocks, discovered_from, relates_to, duplicates, supersedes, replies_to, about] } }, edge: { to: to, type_from: type, prefix: "task_link:" } }
      comments: { key: [id], columns: { id: text, author: text, at: time, body: { type: text, markdown: {} } } }
  - name: knowledge_page
    resource: knowledge        # oxplow/knowledge?id=<id>&store=<store>
    open: true
    key: id
    change_event: oxplow.knowledge_page.changed
    search: { title: title, body: body }
    columns:
      id: text
      title: text
      body: { type: text, markdown: { pins: true } }
      kind: text               # its store's kind (note, memory, instruction)
      url: text
      author: text
      updated_at: time
fields:                        # oxplow's own annotations on every store's tasks
  - model: oxplow/task
    fields:
      - { name: assignee, title: Assignee, kind: text }   # a thread's or the person's URI
      - { name: rank, title: Rank, kind: number }         # sparse: an assign writes one value
```

- **Declared now** (2026-11-12) as `task_record` and `knowledge_record`
  (`v_oxplow_task_record`, `v_oxplow_knowledge_record`): in the `oxplow`
  namespace a model named `task` would be `ref('task')` and shadow core's
  `v_task` for bundled's own datasets. Two Phase 3 decisions follow: a
  bundled dataset is `v_oxplow_<name>`, so `v_task` as the read contract
  needs core to point the name at bundled's dataset (or its readers move);
  and a child edge's prefix must start with its model's name, so task
  links become `task_record_link:<type>` edges (today `task_link:`), and
  `v_task_link` moves with them.
- **Assignment and rank are bundled's fields** (namespace `oxplow`), written
  by its commands (`oxplow.task.assign` / `reorder` become bundled's, over
  `oxplow.model.set_fields`): a store's reconcile never touches them, a
  rename and a tombstone keep them.
- **`v_task` stays, as bundled's dataset** joining `v_oxplow_task` with its
  fields and the store's stages, with today's columns (`provider` = store,
  `native_id` = id, `native` = the store's own fields, `assignee`, `rank`,
  `stage_title`…), so its readers move with no change. Core's `v_task`
  goes in the same publication (two can't both exist).
- **Request links** (`v_request_task`) are bundled's reaction to
  `oxplow.task.changed` events with `cause: trigger`, using the event's
  anchors (now the command's turn: a store's write is observed as its
  command).

## The switch, as decided after the 2026-11-12 review

- **The read contract is bundled's dataset `task` (`v_oxplow_task`)**, over
  `task_record` joined with oxplow's fields and the store's stages, with
  today's `v_task` columns. No core alias for `v_task`: an alias would be a
  core special case. Its readers (pages, datasets, core Rust readers) move
  to the new name. The same holds for `v_task_link` (its edges become
  `task_record_link:<type>`) and the knowledge read contract.
- **Reactions:**
  - request links: a bundled collector `on: [oxplow.task_record.changed]`
    (with the cause filter below) that writes them, declarative and
    replayable, from the event's anchors (a command-started run carries the
    command's turn);
  - assign on start, close the assignment on done: a bundled effect that
    composes `oxplow.model.set_fields`.
- **A cause filter on `on:`** (`where: { cause: [trigger, fields] }`), so a
  thousand-row reconcile doesn't start a thousand script runs that only
  skip. Built: a `where:` value is one scalar or a non-empty list (equals
  any of them), applied before a run is queued or spawned.
- **Order:** the vertical slice runs only on a throwaway oxplow (fixtures,
  an e2e) — a new store `oxtask` on a database whose core tasks share its
  ids would give two items one URI. The switch moves the data first
  (core's rows into `oxtask.sqlite`, rehearsed on a copy of the live home
  with a before/after comparison), then flips the reads (bundled's `task`
  dataset, the resources' `resolve` / `searchable`), and only then deletes
  core's task verbs, tables and projection.
- **Before the deletes:** the provider `read` adapter, and the boot repair
  over models and their children (core's repair goes with the task
  projection).

## Children

- **Declared in the parent** (`children: { <name>: { key, columns, edge? } }`),
  stored in `model__<ns>__<model>__<name>`, read as
  `v_<ns>_<model>_<name>` (`parent`, `store`, the key and columns): only
  the children of a parent the parent's view shows.
- **Written nested in the parent's row, as the whole set:** `"links": [{
  "to": …, "type": "blocks" }]` on a task row is every link that task has
  now (the rest go); a row that leaves the key out keeps its children. In
  a `snapshot` too: a parent restated without `links` keeps them.
- **Removed with the parent** only as far as the views go: a tombstoned
  parent's children are hidden with it and come back with it; swept with
  it.
- A changed set is the parent's change (`changed: [<child>]`); its edges
  (the child `edge`, its markdown's `comment_*` mentions) are restated from
  the parent's URI, by diff.
- A changed child shape keeps the rows, as the parent's does: a new column
  is added in place, a removed or retyped one is rebuilt with the common
  columns copied (an incremental store never refills a child table).
- Names are refused at load when they collide: a model's and a child's
  generated table or view names (`book` + child `notes` vs a model
  `book_notes` or `book__notes`), and a child's fixed `ref_type` that is a
  parent column's `edge` or another child's.

Built (`model_store.rs` `ChildDecl` / `ChildEdge`, `ModelDecl.children`;
manifest parsing in `shared_models.rs`):

- **Rules, checked on load** (`validate_child`): a child's name is a
  snake_case word that is not a parent column, `all` or `field`; its key
  columns exist (text, int, uri or time); its columns may not be named
  `parent`, `store` or `written_at`, and carry no `edge:` (the child's
  `edge` is declared on the child) and no `pins:` (a child's markdown
  projects `comment_*`, never pinned refs).
- **The edge:** `to` is a `uri` column, and exactly one of `type_from` (a
  text column with `values:`; `prefix` is then required) or a fixed
  `ref_type`. The ref types must be the model's own, so core's are out of
  reach: a `prefix` is `<model>_<word>:` (ends in one `:`, starts with the
  model's name, so a core plain-word type can't match and `task_link:`
  is the task model's), and a `ref_type` is `<model>.<word>` with no
  prefix, as a column's `edge` is.
- **Storage and reads:** `model__<ns>__<model>__<child>` (`parent`, `store`,
  the columns, `written_at`; primary key `parent` then the key), read as
  `v_<ns>_<model>_<child>`, which joins the parent table with the parent
  view's filters (live row, live store). It is the owner's entity dataset
  and one of its published views (`public_contracts.rs`).
- **Writes:** a key named like a child holds its whole set, restated by
  diff (insert, update, delete). A bad child row (not an object, an
  undeclared column, a bad value, a missing or repeated key) quarantines
  the parent row. A changed set adds the child's name to `changed`. A
  rename moves the children; the tombstone sweep deletes them.
- **Edges:** after a set changes, the parent's child-edge types (prefix +
  each `values:` value, or the fixed type) and, with a child markdown
  column, the `comment_*` types are restated as one slice from the
  parent's URI, from every child's stored rows. The parent's own column
  edges are a separate slice.
- **Not done:** a child removed from a declaration leaves its table and
  view until the database is rebuilt; the boot repair does not cover
  children's edges yet.

## Removal

- A removed row is a **tombstone**: `removed_at` is set and the view
  hides it (`v_<ns>_<model>` has no removed rows; `v_<ns>_<model>_all`
  has them). Its fields, edges and every annotation keyed by its URI stay
  for 30 days; the same id coming back revives the row with them.
  After 30 days a sweep deletes it and them.
- A table rebuild (a removed or retyped column) copies the rows into the
  new table in place and logs no event.

## Fields

Every open model keeps per-item values beyond its columns.

- **Declared once.** A store declares its fields in its `fills:` entry
  (`[{ name, title, kind: enum | text | number, values?, read_only? }]`,
  today's `FieldDecl`). Any other extension may declare fields on a model
  in a top-level `fields: [{ model: <owner ns>/<model>, fields: [...] }]`
  (an extension that scores tasks). Published as `v_model_field_decl`
  (`model`, `namespace`, `name`, `title`, `kind`, `values`, `read_only`).
  Shared screens draw fields from it without knowing the store.
- **Kept apart by namespace.** Values are stored in `model_field_value
  (model, uri, namespace, name, value)`; the view's `fields` column is
  built from it as `{ "<namespace>": { "<name>": value } }`. The
  namespace is the store id for a store's own fields, else the writing
  extension's namespace (a dashed name's default namespace uses
  underscores: `shelf-score` writes as `shelf_score`).
  - a store writes its fields in its rows (`"fields": {...}`), into its
    store id;
  - another extension returns `{"fields": [{"model", "uri", "values":
    {...}}]}` from a collector, or composes core's command
    `oxplow.model.set_fields { model, uri, values }` from a command
    (both checked against its `fields:` declaration, into its own
    namespace; a null value removes the field). It logs the change event
    with `cause: fields`.
  - A store's `snapshot` restates its own fields only. It never removes
    another namespace's values on an item that is still there. Values go
    with the item (as a tombstone, "Removal").
- **Long form for SQL.** `v_<ns>_<model>_field` (`uri`, `namespace`,
  `name`, `value`, `kind`) beside the model's view, for joins, filters
  and dimensions. A dimension may also read `json_extract(fields,
  '$.<namespace>.<name>')`.
- **Promote, do not fork.** When several stores declare the same field
  (priority, due date), the owner adds a real column and bumps the
  model's version. Stores then write the column. This is the promotion
  path of the target architecture.
- **Data that is not one value per item** (a dependency graph, a
  history) is a model of its own keyed by the item's URI, with a
  declared relationship to the shared model and a slot on the item's
  page.
- **Known limit.** An entity dimension applies only to metrics over its
  own view ([semantic-layer.md](./semantic-layer.md) "Dimensions slice
  data"), so slicing another entity's metric by a task's field does not
  work yet. Not part of this work.

## Reading

- `v_<ns>_<model>` is the model's view: every enabled store's live rows,
  its implicit columns, its declared columns. A disabled store's rows are
  hidden (kept until it is removed). There is no "active store".
- The owner publishes **one read contract** joining its model with its
  annotations: bundled's `v_task` (the model, stages, assignment, rank)
  and `v_knowledge_page` (the model, pins, reads) keep their names and
  the columns their readers use today, so the move is the owner's
  dataset, not every reader. With bundled off they do not exist, and
  core readers of them (`v_request_task`, `v_commit_task`, the ref
  resolver) say so (empty, never an error).
- Any extension may `ref()` an open model, and add dimensions and
  metrics over it, as over any entity.

## Rules

- No generic write verbs: no `task.create` that routes to a store, and no
  standard bindings. Confirm and audit are the bus's, as for any command.
- **Undo is optional and per store (decided 2026-11-12).** A store's
  command may offer one; it doesn't have to (a web app's write often can't
  be taken back). A script that composes a write which leaves oxplow's own
  records (`oxplow.extension_store.write`, `oxplow.extension_files.write`,
  a provider call) runs as steps, which have no automatic undo; it may
  return `"inverse": {"commands": [...]}` beside its `commands`, the calls
  that reverse what it did, and the bus keeps that as the run's undo — the
  same place, the same Undo, the same policy, confirm and audit as a
  built-in one. A run whose script returns no inverse has none. oxtask
  offers one for each of its writes (`create` and `comment` name the row they mint from their `finish`). (Built: the `inverse` key on a
  script command's output, kept by the bus for a steps run — see
  [extensions-commands.md](./extensions-commands.md) "Running" and
  [commands.md](./commands.md) "As steps". The inverse names commands, for
  a store write the extension's own command that makes it: the host's
  `oxplow.extension_store.write` runs only for the composing script.)
- With the owner extension off, its models do not exist; a store that
  requires it is held. The shell still starts.
- A model's contract (columns, key, types, its change event) is
  versioned like a dataset's ([semantic-layer.md](./semantic-layer.md)
  "Keys").
- Owner-only models (not `open`: bundled's assignment and rank) are the
  owner's own data, written by the owner's commands through core's
  `oxplow.model.write { model, rows }` (owner only), never rebuilt with
  loss: a removed or retyped column is a load error until the owner
  ships a migration.

## The example stores (framework fixtures)

The framework is proven by four example extensions, written first as red
fixtures. They live in `examples/observed/` until their pieces land, then
move to `examples/extensions/` (which must load clean):

- **`shelf`**: the owner. An open model `book` (markdown `body` with
  pins, `author` text, `about` uri edge, search), its change event, the
  action point `shelf/book.new`, a read-only page.
- **`shelf-folder`**: a store over markdown files in `shelf/`, triggers
  `start` and `{ files: "shelf/*.md" }`, the file service, a command at
  `shelf/book.new` that writes a file.
- **`shelf-db`**: a store over its own SQLite file (`storage: { sqlite:
  true }`, `store.sql`), triggers `start` and `{ sqlite: own }`, a field
  (`rating`), a command at `shelf/book.new`.
- **`shelf-score`**: a field (`score`) on any book.

Fixtures run with live triggers off (a fixture runs a collector by its
trigger kind explicitly); the watch and the poll are tested in Rust.
They cover: upsert, delete, rename, full reconcile, the removal guard,
tombstones and revival, unchanged rows writing nothing, quarantine,
change events with before/after and cause, pins (new, kept on a prose
edit, verify, `pins: false`), stated refs, the uri edge, enum values,
search through the index, the SQLite file, the file service, fields kept
apart by namespace through a reconcile, an action point with zero, one
and two contributors, and the store filter.
