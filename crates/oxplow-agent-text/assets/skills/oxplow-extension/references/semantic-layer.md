# Finding the data, refs, and publishing models

## Find the data

- `v_model` lists every queryable view (`v_stream`, `v_thread`, `v_work_item`,
  `v_effort`, `v_comment`, `v_wiki_page`, `v_snapshot`, `v_measure`,
  `v_capture`, `v_fact`, …) with its doc, and `v_model_column` documents
  every column: `SELECT view, name, sql_type, doc FROM v_model_column WHERE
  view = 'v_work_item'`. **Read them first**; don't guess column names.
- Work items are `v_work_item` (with `v_work_item_link` and
  `v_work_item_comment`): the active work list's, whichever it is — never
  a particular list's own tables.
- `oxplow_query_sql` runs one read-only `SELECT`/`WITH`. Iterate on the query
  here until the rows are what the user wants, before writing any files.
- The project's architectural zones are `v_zone` (each rule of the `zones:` table
  in `.oxplow/project.yaml`, in order, with the files it claims in the worktree,
  then a closing `other` row) and `v_zone_unmatched` (paths no rule matched);
  both are read live. Read the rules with `oxplow.config.get { key: "zones" }`,
  change them with `oxplow.config.set` (the whole ordered table).
- Only `v_*` views can be read; a physical table is refused, naming the
  view to read instead. Metrics are read with `metric_grid()` and
  `MEASURE('<key>')` (see the `oxplow-metrics` skill).

## Refs in scripts and SQL

Everything in oxplow has one identity, its ref (`effort:eff12`,
`work_item:oxplow:<id>`, `file:src/a.rs`, `collector:triage/todos`).
Carry refs; never assemble or dissect them:

- **In SQL**, select the model's `ref` column (`v_effort.ref`,
  `v_decision.ref`, `v_thread.ref`, `v_work_item.ref`, …) and compare
  refs to refs: `WHERE ref = :ref`. Never `'effort:eff' || id` or
  `substr(ref, …)`.
- **To link one lens to another from SQL**, use `lens_ref(id, key1,
  value1, …)`: `lens_ref('oxplow-bundled/thread-activity', 'thread_id',
  thread_id)` gives `lens:oxplow-bundled/thread-activity?thread_id=7`
  (params escaped, a NULL value left out; an odd argument count is an
  error). Never `'lens:…?x=' || value`.
- **In a script**, build with `ref.build("file", f["path"])` and read
  with `ref.parse(r)` / `ref.id(r, kind = "effort")` (which refuses a ref
  of another kind) / `ref.kind(r)` / `ref.is_ref(v)`. Never
  `"effort:eff" + str(id)` or `r[len("effort:eff"):]`.
- **In a lens action's input**, pass a row's ref column
  (`{ ref: "{{row.ref}}" }`), not a ref rebuilt from an id.
- **A ref may dangle.** The thing it names can be deleted, or its
  extension removed; store the ref anyway, and expect a reader to show it
  as unresolved rather than fail.

## Publishing models

When other lenses, extensions or agents should read a shaped dataset —
not just one lens — publish it as a **model**: a documented, versioned
view. Declare it in `extension.yaml` and write its SQL in
`models/<name>.sql`:

```yaml
models:
  - name: blocked
    version: 1
    description: Blocked tasks, oldest first.
    columns:                       # the contract: every column, in order
      - { name: id, type: INTEGER, doc: Task id. }
      - { name: title, type: TEXT, doc: Title. }
    tests:
      - { not_null: id }
      - { unique: id }
```

```sql
-- models/blocked.sql
SELECT ref, title FROM ref('work_item') WHERE state = 'blocked'
```

- It publishes as `v_<extension>_<name>` (dashes as underscores):
  `v_late_work_blocked` for extension `late-work`.
- Read through `ref()` only: `ref('work_item')` is a core model (`v_work_item`),
  `ref('blocked')` your own model or synced entity. Another extension's
  model is read through an import ("Importing another extension's model" below). `source('<table>')` is only
  for your own source tables (`ext__<ext>__<entity>`).
- `type` is what SQLite reports for the column (`PRAGMA table_info`);
  leave it out for a computed column (`count(*)`, an expression).
- **Changing the columns is a breaking change:** bump `version`. To keep
  the old shape for readers, keep its SQL in another file and list it:
  `deprecated: [{ version: 1, file: blocked.v1.sql, until: 2027-01-01 }]`
  publishes `v_<ext>_blocked_v1` until then.
- Check runs the compile without publishing; oxplow publishes after
  the files change. A failing declared test shows in the extension's
  errors and `v_model_test`, and the view stays up.
- **Let other extensions build on it** by exporting it:
  `exports: [{ contribution: <ext>/model/<name>, version: 1, visibility:
  stable }]` (the version is the model's). Unexported, it's yours alone.

### Importing another extension's model

Declare what you read, at the version you read it, and read it through
the binding:

```yaml
imports:
  - { id: reviews, contribution: model:effort-review/reviews, version: 1, optional: true }
```

```sql
SELECT work_item, review FROM import_ref('reviews')
```

- `optional: true`: when the producer is missing, disabled or exports
  another version, what reads the binding (a lens, a model, a slot that
  mounts the lens) is withheld with a warning, and the rest of your
  extension still loads. Without it, your whole extension is blocked.
- `oxplow extension check` prints how each import resolved
  (``import `reviews` → model:effort-review/reviews v2: available``, or
  why not).
- The producer bumping its model's version is a break: your import of
  the old version goes unavailable until you change it.
