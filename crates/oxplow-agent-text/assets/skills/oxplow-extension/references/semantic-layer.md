# Finding the data, URIs, and publishing datasets

## Find the data

- `v_dataset` lists every queryable view (`v_stream`, `v_thread`, `v_work_item`,
  `v_effort`, `v_comment`, `v_wiki_page`, `v_snapshot`, `v_measure`,
  `v_capture`, `v_fact`, …) with its doc, and `v_field` documents
  every field (column): `SELECT view, name, sql_type, doc FROM v_field WHERE
  view = 'v_work_item'`. **Read them first**; don't guess column names.
- Comment threads on pages are `v_comment`, and each one's messages
  `v_comment_message` (`WHERE comment = <the thread's uri> ORDER BY
  position`).
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
- Also read live, for the stream a page runs for (its `stream_id`; the
  primary checkout's without one): `v_file` (the files and folders
  of a filesystem: the worktree's by default, with each file's change;
  `WHERE filesystem = 'oxplow:extensions'` and `AND rev = '<revision>'` name
  another filesystem and a revision, no `filesystem` is `oxplow:local`; a
  folder's listing is `WHERE parent = 'src'` and fills `size` and
  `modified_at`; each row's `uri` opens it) and `v_agent_conversation` (every harness's saved conversations
  begun there; join `v_agent_session` on `resume_session_id = native_id`).
  And for the project: `v_agent_harness` (the registered agents, those
  `agents:` names first, `enabled` and `position`), `v_config_value` (every setting's value now and
  where it comes from) and `v_vcs_workspace` (the repository's working
  copies and the stream in each; `stream_id IS NULL` is adoptable). A stream's
  terminals — shells in its worktree, in order — are `v_terminal` (open one with
  `oxplow.terminal.open`). A page
  over a live dataset re-runs when what it reads changes.
- Made when a query names it: `v_commit` / `v_commit_file` `WHERE sha =
  :sha` (the full sha) reads a commit however old, and `v_change WHERE
  kind = 'commit' AND target = :sha` (or `kind = 'working' AND target =
  ''`, the page's stream) creates that change and starts its analysis.
- Only `v_*` views can be read; a physical table is refused, naming the
  view to read instead. Metrics are read with `metric_grid()` and
  `MEASURE('<key>')` (see the `oxplow-metrics` skill).

## Refs in scripts and SQL

Everything in oxplow has one identity, its URI (`oxplow:oxplow/effort?id=eff12`,
`oxplow:oxplow/work_item?id=<id>&provider=oxplow`, `oxplow:oxplow/file?path=src/a.rs`, `oxplow:oxplow/collector?collector=triage/todos`).
Carry URIs; never assemble or dissect them:

- **In SQL**, select the dataset's `uri` column (`v_effort.uri`,
  `v_decision.uri`, `v_thread.uri`, `v_work_item.uri`, …) — each an
  oxplow URI (`oxplow:oxplow/effort?id=eff12`) — and compare URIs to URIs:
  `WHERE uri = :uri`. To build one, `uri(resource, key1, value1, …)`:
  `uri('oxplow/effort', 'id', json_extract(e.payload, '$.effort'))` (an event
  payload field named for one resource holds a bare id); to read a param back,
  `uri_param(uri, 'id')`. Never `'oxplow:…' || id`, `'eff' || id` or `substr(uri, …)`.
- **A field named for one resource is a bare id, not a URI**: a command
  input `thread` is `thr3`, `effort` is `eff12`, `claim` is the integer `7`.
  Select `uri_param(t.uri, 'id')` to feed one; `{{param.x.id}}` does the
  same in an action's input. A URI goes only where a field can name anything
  (`uri`, `target`, `subject`) and for work items.
- **To link to a page from SQL**, `uri('oxplow/page', 'page',
  'oxplow-bundled/thread-activity', 'thread_id', thread_id)` gives
  `oxplow:oxplow/page?page=oxplow-bundled/thread-activity&thread_id=7`
  (params escaped and sorted, a NULL value left out).
- **In a script**, build with `uri.build("oxplow/file", path = f["path"])`
  and read with `uri.parse(r)` (`{ resource, params, view }`) /
  `uri.param(r, "path")` / `uri.id(r, resource = "oxplow/effort")` (which
  refuses a URI of another resource) / `uri.resource(r)` / `uri.is_uri(v)`.
  Never `"oxplow:oxplow/effort?id=" + id` or slicing a URI.
- **In a page action's input**, pass a row's uri column
  (`{ uri: "{{row.uri}}" }`), not a URI rebuilt from an id.
- **A URI may dangle.** The thing it names can be deleted, or its
  extension removed; store the URI anyway, and expect a reader to show it
  as unresolved rather than fail.

A collector's `doc:`, and the optional `description:` of a `resources:`,
`dimensions:` or `implementations:` entry, are one sentence each; they show as
`description` on `v_collector`, `v_resource`, `v_dimension`, `v_capability_provider`
and `v_contribution`.

`v_command` is every registered command with its summary, scope and whether a
person confirms it; `v_skill`, `v_slot` and `v_custom_component` describe what
an extension gives the agent, mounts and draws. Each carries the `uri` that
`v_contribution` holds, so they join.

## Publishing datasets

When other pages, extensions or agents should read a shaped set of rows —
not just one page — publish it as a **dataset**: a documented, versioned
view. Declare it under `datasets:` in `extension.yaml` and write its SQL in
`datasets/<name>.sql`:

```yaml
datasets:
  - name: blocked
    version: 1
    description: Blocked tasks, oldest first.
    fields:                        # the contract: every column, in order
      - { name: uri, type: TEXT, doc: The work item. }
      - { name: title, type: TEXT, doc: Title. }
    relationships:                 # optional: a field holding another dataset's values
      - { from: uri, to: work_item.uri }
    tests:
      - { not_null: uri }
      - { unique: uri }
```

```sql
-- datasets/blocked.sql
SELECT uri, title FROM ref('work_item') WHERE state = 'blocked'
```

- It publishes as `v_<extension>_<name>` (dashes as underscores):
  `v_late_work_blocked` for extension `late-work`.
- Read through `ref()` only: `ref('work_item')` is a core dataset (`v_work_item`),
  `ref('blocked')` your own dataset or synced entity. Another extension's
  dataset is read through an import ("Importing another extension's dataset" below). `source('<table>')` is only
  for your own source tables (`ext__<ext>__<entity>`).
- `type` is what SQLite reports for the field's column (`PRAGMA table_info`);
  leave it out for a computed one (`count(*)`, an expression).
- `relationships: [{ from: <field>, to: <dataset>.<field> }]` says a field
  holds another dataset's values (`to` names the dataset as `ref()` does:
  `work_item.uri`, `blocked.uri`, `other-ext/items.uri`). `from` must be one
  of the fields; a `to` that names no dataset or field is an extension error.
  Readers see them in `v_dataset.relationships`. Not part of the contract.
- **Changing the fields is a breaking change:** bump `version`. To keep
  the old shape for readers, keep its SQL in another file and list it:
  `deprecated: [{ version: 1, file: blocked.v1.sql, until: 2027-01-01 }]`
  publishes `v_<ext>_blocked_v1` until then.
- **`ai_context`** is optional guidance for an agent that reads the
  catalog, on a dataset, on one of its fields, on a metric and on a
  dimension: `ai_context: { instructions: "When to use it and how to read it.",
  synonyms: [tickets], examples: ["open items by assignee"] }`. Every key
  is optional; an unknown key is refused. It is not part of a field's
  contract: changing it never needs a `version` bump. Agents read it in
  `v_dataset.ai_context`, `v_field.ai_context`,
  `v_metric_spec.ai_context` and `v_dimension.ai_context` (a JSON text,
  NULL when absent), and in the `catalog` tool.
- Check runs the compile without publishing; oxplow publishes after
  the files change. A failing declared test shows in the extension's
  errors and `v_dataset_test`, and the view stays up.
- **Let other extensions build on it** by exporting it:
  `exports: [{ contribution: "oxplow:oxplow/dataset?dataset=<ext>/<name>", version: 1,
  visibility: stable }]` (the version is the dataset's). Unexported, it's yours alone.

### Importing another extension's dataset

Declare what you read, at the version you read it, and read it through
the binding:

```yaml
imports:
  - { id: reviews, contribution: "oxplow:oxplow/dataset?dataset=effort-review/reviews", version: 1, optional: true }
```

```sql
SELECT work_item, review FROM import_ref('reviews')
```

- `optional: true`: when the producer is missing, disabled or exports
  another version, what reads the binding (a page, a dataset, a slot that
  mounts the page) is withheld with a warning, and the rest of your
  extension still loads. Without it, your whole extension is blocked.
- `$OXPLOW_BIN extension check` prints how each import resolved
  (``import `reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: available``, or
  why not).
- The producer bumping its dataset's version is a break: your import of
  the old version goes unavailable until you change it. Leave `version`
  off to bind whatever version it exports (fine for extensions you own
  together; pin it for one someone else changes).
- An extension you need on but import nothing from (you run its
  commands, read its datasets by name) goes in `requires: [<name>]`:
  deploying yours refuses while it's off, and turning it off refuses
  while yours is on.
