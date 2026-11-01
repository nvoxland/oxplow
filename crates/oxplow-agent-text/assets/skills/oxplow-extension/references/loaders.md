# Loaders (`loaders:`)

`loaders:` is a mapping of named loaders. Each gets rows; a node shows one by
`from: <name>`. Loaders run **in order** and a loader reads only the ones
**before** it. One that fails carries its error and the rest of the page
still shows. A loader no visible node shows still runs when a later loader,
an embed or the badge uses it; one only a hidden `switch` child shows does
not.

A page with `for:` (a resource page, `page-format.md` "Renderers") also has
one loader you never write: `content`, always first, the record its `uri`
names. Read it like any earlier loader (`SELECT … FROM content`, `{ from:
content.path }`); the name is reserved.

A loader has exactly one of `sql`, `command`, `page` or `rows`, plus
optional `columns` (keys, labels and formats its nodes default to) and
`empty` (text when it has no rows).

**Names** are lowercase letters, digits and `_`, starting with a letter.
Never `v_…`, `m_…`, `temp` or `main`: they must not look like a dataset.
Never a SQL keyword SQL can't name a table by (`order`, `group`, `from`,
`commit`, `add`; `row` and `key` are fine): check says which.

**Args** (`params:` of a sql or page loader, `args:` of a command loader)
are `{ param: <name> }` (a page param), `{ from: <loader>.<column> }` (an
earlier loader's **first row**) or a literal.

## `sql`

```yaml
loaders:
  base:  { sql: "SELECT id, state FROM v_work_item" }
  open:  { sql: "SELECT * FROM base WHERE state = :state" }   # base is a table
  child:
    sql: "SELECT title FROM v_work_item WHERE parent_uri = :p"
    params: { p: { from: open.id } }
```

- One read-only statement over the `v_*` datasets (`oxplow_query_sql`'s rules;
  read column names from `v_field`, never guess).
- `:name` binds a page param or a `params:` entry. An earlier loader reads
  as a **temp table of its name**; naming a later one is an error.
- It runs again when a dataset it read changes.
- To open another page from a cell, select its URI: `uri('oxplow/commit',
  'sha', sha)`; see `semantic-layer.md`.

## `command`

```yaml
loaders:
  defs:
    command: oxplow.page.describe
    args: { uri: "oxplow:oxplow/file?filesystem=oxplow:extensions&path=acme/pages/list.yaml" }
    reads: [v_page]
    items: nodes
```

- Only a **read** command runs (access `View` or `Read`), as the viewer;
  writes are actions.
- Say what makes it run again: `reads: [datasets]` or `refresh: manual` /
  `refresh: { every: 30s }`. Neither is an error.
- The result is `{ columns, rows }` as is, an array of objects (a row each),
  else one `value` row. `items: <field>` takes the list inside the result's
  `<field>` as the rows (a command that returns `{ title, nodes: […] }`).
- **One command, several lists: run it once.** An `items:` loader runs
  the command again for each loader that names it (the run memo keys on
  the loader, `items` included), so a page that shows three lists of one
  result would run it three times. Instead declare one command loader
  with no `items:`, whose single row holds each list as a JSON column,
  and read each list with a `sql` loader over it (an earlier loader is a
  table):

  ```yaml
  loaders:
    page: { command: oxplow.page.describe, args: { uri: … }, reads: [v_page] }
    params:
      sql: "SELECT j.value ->> '$.name' AS name FROM page, json_each(page.params) j"
    loaders:
      sql: "SELECT j.value ->> '$.name' AS name FROM page, json_each(page.loaders) j"
  ```

  Use `items:` for a command that returns its only list.
- A table whose query is only known at run time (which dataset, which
  columns, from params): build the SQL in a `sql` loader, then run it with
  the read command `oxplow.sql.query { sql }` — `args: { sql: { from:
  query.sql } }` — which returns `{ columns, rows }` over the same `v_*`
  datasets.

## `page`

```yaml
loaders:
  count: { page: acme/numbers, loader: count, params: { n: 1 } }
```

Runs that page, part or query and takes its loader `loader`. Pages nest at most 8 deep;
the same definition with the same inputs runs once per run, embeds included.

## `rows`

```yaml
loaders:
  colors: { rows: [{ state: failed, tone: danger }, { state: passed, tone: success }] }
```

Inline rows (columns are the first row's keys). Use for lookup tables, a
snapshot of numbers you computed, and the `choices` of a form. A list two pages share is a query.

## Budgets

Do filtering, grouping and `LIMIT` in SQL. `oxplow_run_page` shows up to
200 rows per loader (json) with `truncated`, 50 in text. Keep a page to a
handful of loaders; one nothing shows is wasted work.

**Filter a large dataset by a constant, never a join from an earlier
loader.** An earlier loader is a temp table, and `FROM about a JOIN v_fact f
ON f.measure_key = a.key` makes SQLite scan every fact (on a real project it
times out at 5 s). Write the filter as a scalar subquery instead —
`FROM v_fact f WHERE f.measure_key = (SELECT key FROM about)` — which is
evaluated once and uses the index; for a two-step lookup, `f.capture_id IN
(SELECT c.id FROM v_capture c WHERE c.producer = (SELECT id FROM about))`.
Order newest-first by the row's `id`, not a timestamp from a joined table.
A fixture's throwaway data is too small to show this: check such a page with
`oxplow_run_page` on the project's own data.
