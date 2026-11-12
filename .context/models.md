# Shared models (contract, being built)

**Status: contract.** Nothing here is built yet. It is the contract for
the "observed systems" work: tasks and knowledge stop being write-through
capabilities, and a store's extension watches its own tool and fills a
shared model. This doc is the contract for the framework piece F1. The
other pieces have their contracts in their own docs:

- triggers (`files`, `sqlite`, `start`): [extensions-jobs.md](./extensions-jobs.md) "Triggers for observers";
- an extension's own SQLite file and the scoped file service: [extensions-permissions.md](./extensions-permissions.md) "Store services";
- named action points and the store filter: [extensions-slots.md](./extensions-slots.md) "Action points";
- the manifest keys: [extensions-manifest.md](./extensions-manifest.md) "`models:` and `fills:`".

Update this doc as each part lands. Move a part's text into "Built"
when its fixtures pass.

## The idea

```
tool's store ──watch / poll / event──▶ store extension ──model.write──▶ shared model
(a SQLite file, a folder, a provider)        │                          (rows of every store)
                                             └─ its own commands, forms, skills, settings
shared pages: read-only views over the model · action points · annotations keyed by item URI
```

- A **shared model** is a table that one extension (the **owner**)
  declares and other extensions (the **stores**) fill. Bundled owns the
  task, task link, task comment and knowledge page models.
- A store writes only through the **`model.write` service**. oxplow does
  the SQL from the declared schema. No extension SQL runs on the main
  database.
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
    key: id                    # one column: the store's native id, the URI's `id`; unique per store
    change_event: tasks.task.changed   # declared in event_types:, one per changed row
    columns:
      title: text
      body: { type: text, markdown: { pins: false } }
      state: { type: text, values: [todo, in_progress, blocked, done, canceled] }
      parent: uri
      closed_at: time
    search: { title: title, body: body }
```

- **Implicit columns** on every row: `store` (text), `uri` (text, the
  resource URI built from the declared resource with params `store` and
  `id`: `oxplow:<ns>/<resource>?id=<id>&store=<store>`), `fields` (JSON,
  "Fields"), `written_at` (time, set by oxplow). The key is unique per
  `(store, <key columns>)`. `uri` is unique.
- **Column types:** the entity types (`text`, `int`, `real`, `bool`,
  `time`) plus `uri` (one resource URI) and `uris` (a JSON list of URIs).
  `values:` makes a text column an enum, checked on write.
- **Edges from columns.** A `uri` or `uris` column projects `page_ref`
  edges from the row's URI, ref type `<model>.<column>` (a task's
  `parent` is `task.parent`). This replaces hand-written projections
  such as task links.
- **Markdown bodies.** A column with `markdown:` is parsed for refs as
  oxplow's markdown, the same parser for every model
  (`page_ref_projections::body_edges`, ref types `file_mention`,
  `dir_mention`, `wikilink`, `task_mention`, `finding_mention`,
  `commit_mention`). The edges are restated in the same transaction as
  the row, by diff (an unchanged body writes no edge).
  - `markdown: { pins: true }` pins the column's file and dir refs to the
    primary stream's latest snapshot when they are new, and keeps the
    pin of an edge that is already there (today's knowledge behaviour,
    `knowledge_record::write_page_tx`). Staleness is then generic:
    `v_ref_pin` (source URI, path, pinned snapshot, latest snapshot,
    `stale`) over every pinned edge: columns `source`, `source_resource`,
    `path`, `pinned_snapshot_id`, `latest_snapshot_id`, `stale`; a reader
    filters on `source_resource`. `knowledge_file_ref` / `knowledge_dir_ref` go.
  - A store whose tool states its own links (a provider) sends a `uris`
    column instead (`refs:` in the row); then the markdown column is not
    parsed (today's `stated` path).
  - Verifying is generic: core's command `oxplow.refs.verify { uri,
    files? }` re-pins the named (or all) pinned file edges of a row to
    now, checked against the row's cited files and dirs (today's
    `verify_tx`). It never writes the store.
- **Search.** `search: { title, body }` makes the model's resource
  searchable with no SQL of its own: oxplow generates the searchable
  dataset `v_search_<ns>_<model>` (`uri`, `title`, `body`, `version =
  written_at`). One per model.
- **The change event** is declared by the owner (`event_types:`), payload
  `{ uri, store, id, op: upserted | removed, changed: [column…] }`,
  subject the row's URI, anchors from the caller (a command's actor, a
  collector's causing event). One per row that changed; an unchanged row
  logs none. The owner's own reactions (assignment, request links, pins)
  follow these events.

## Filling a model (a store)

```yaml
requires: [oxplow-bundled]
fills:
  - model: oxplow/task          # <owner namespace>/<model>
    stores: [oxtask]            # store ids this extension writes; default [<namespace>]
    fields:                     # this store's own fields (see Fields)
      - { name: priority, title: Priority, kind: enum, values: [urgent, high, medium, low] }
```

- A store id is owned by one extension. Two extensions naming the same
  store id is a load error. A provider instance's store id is its
  instance id.
- **The service.** `model.write` is callable from collectors, commands
  and effects (`needs: [model.write]`), and only for the stores in the
  caller's `fills:`:

```python
scope("model.write", {"model": "oxplow/task", "store": "oxtask",
    "upsert": [{"id": "t1", "title": "Fix it", "state": "todo",
                "fields": {"priority": "high"}}],
    "delete": ["t0"]})
scope("model.write", {"model": "oxplow/task", "store": "oxtask",
    "snapshot": [ ...every row the store has... ]})
# -> {"upserted": 1, "removed": 1, "unchanged": 0}
```

  - `upsert` writes rows by key; a missing optional column is NULL.
  - `delete` removes keys.
  - `snapshot` states every row of that store: rows of the store not in
    it are removed. This is the full reconcile (`on: start`).
  - Each row is checked against the declared columns and the store's
    declared fields; a bad row fails the whole call, naming the row and
    the column.
  - It diffs against what is stored and writes only what changed (the
    rule of the asset refill and the page-ref repair). It logs one change
    event per changed or removed row, in the same transaction.
  - In a command it runs in the command's transaction. In a collector it
    runs in the run's transaction (with its `collector_run` row).
- **Cursor.** A collector may return `"cursor": {...}` beside its output;
  it is kept in `collector_run.cursor_json` and given back as
  `x["cursor"]` on the next run. A failed run keeps the old cursor.
- **Providers.** A provider process's `read` stream feeds `model.write`
  too, for its instance's store. `TasksProjection` and
  `KnowledgeRecords` go.

## Fields

Every open model keeps per-item values beyond its columns in `fields`.

- **Declared once.** A store declares its fields in its `fills:` entry
  (`[{ name, title, kind: enum | text | number, values?, read_only? }]`,
  today's `FieldDecl`). Any other extension may declare fields on a model
  too, in a top-level `fields: [{ model: <owner ns>/<model>, fields: [...] }]`
  (an extension that scores tasks). Published as `v_model_field_decl` (`model`, `namespace`,
  `name`, `title`, `kind`, `values`, `read_only`). Shared screens draw
  fields from it without knowing the store.
- **Kept apart by namespace.** Values are stored per `(item uri,
  namespace, name)`. The `fields` column is the JSON object
  `{ "<namespace>": { "<name>": value } }`. An extension writes only its
  own namespace's values:
  - a store writes its fields in its rows (`"fields": {...}` in
    `upsert` / `snapshot`), always into its own namespace;
  - another extension writes `scope("model.fields", {"model", "uri",
    "values": {...}})` (`needs: [model.write]`, a `fields:` entry
    declaring them), into its namespace.
  - A store's `snapshot` restates its own namespace's values only. It
    never removes another namespace's values on an item that is still
    there. **Values go when the item goes.**
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

- `v_<ns>_<model>` is the model's view: every store's rows, its implicit
  columns, its declared columns. Bundled's task model is `v_oxplow_task`;
  its pages read that. There is no "active store": every enabled store's
  rows show; a disabled store's rows are hidden (kept until it is
  removed).
- Any extension may `ref()` an open model, and add dimensions and
  metrics over it, as over any entity.

## Rules

- No generic write verbs: no `task.create` that routes to a store, and no
  standard bindings. Confirm, undo and audit are each store's job.
- **Rebuilds.** A model's table is a cache of its stores: a changed
  shape adds new columns in place, and a removed or retyped column
  rebuilds the table, which the stores refill at their next `start`. A
  model that is not `open` (its owner's own data: assignment, rank) is
  never rebuilt with loss: a removed or retyped column is a load error
  until the owner ships a migration.
- **The change event's `v_event` row:** `type` is the declared type,
  `payload` the JSON above.
- With the owner extension off, its models do not exist; a store that
  requires it is held. The shell still starts.
- A model's contract (columns, key, types, its change event) is
  versioned like a dataset's ([semantic-layer.md](./semantic-layer.md)
  "Keys").

## The example stores (framework fixtures)

The framework is proven by three example extensions, written first as
red fixtures (they live in `examples/observed/` until their pieces land,
then move to `examples/extensions/`, which must load clean):

- **`shelf`**: the owner. An open model `book` (markdown `body` with
  pins, `author` text, `about` uri, search), its change event, the
  action point `shelf/book.new`, a read-only page.
- **`shelf-folder`**: a store over markdown files in `shelf/`: `on:
  [start, { files: "shelf/*.md" }]`, the file service, `model.write`
  (`snapshot` at start, `upsert` / `delete` on change), a command at
  `shelf/book.new` that writes a file.
- **`shelf-db`**: a store over its own SQLite file: `storage: { sqlite:
  true }`, `store.sql`, `on: [start, { sqlite: own }]`, a command at
  `shelf/book.new`, a field (`rating`).
- A fourth, **`shelf-score`**, adds a field (`score`) to both stores'
  items with `model.fields`.

Their fixtures cover: upsert, delete, full reconcile, unchanged rows
writing nothing, change events, each trigger, the SQLite file, the file
service, fields kept apart by namespace through a reconcile, an action
point with zero, one and two contributors, and the store filter.
