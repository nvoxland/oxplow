# Loaders (`data:`)

`data:` is a mapping of named loaders. Each gets rows; a node shows one by
`from: <name>`. Loaders run **in order** and a loader reads only the ones
**before** it. One that fails carries its error and the rest of the page
still shows. A loader no visible node shows still runs when a later loader,
an embed or the badge uses it; one only a hidden `switch` child shows does
not.

A loader has exactly one of `sql`, `command`, `page` or `rows`, plus
optional `columns` (keys, labels and formats its nodes default to) and
`empty` (text when it has no rows).

**Names** are lowercase letters, digits and `_`, starting with a letter.
Never `v_…`, `m_…`, `temp` or `main`: they must not look like a model.

**Args** (`params:` of a sql or page loader, `args:` of a command loader)
are `{ param: <name> }` (a page param), `{ from: <loader>.<column> }` (an
earlier loader's **first row**) or a literal.

## `sql`

```yaml
data:
  base:  { sql: "SELECT id, state FROM v_work_item" }
  open:  { sql: "SELECT * FROM base WHERE state = :state" }   # base is a table
  child:
    sql: "SELECT title FROM v_work_item WHERE parent_uri = :p"
    params: { p: { from: open.id } }
```

- One read-only statement over the `v_*` models (`oxplow_query_sql`'s rules;
  read column names from `v_model_column`, never guess).
- `:name` binds a page param or a `params:` entry. An earlier loader reads
  as a **temp table of its name**; naming a later one is an error.
- It runs again when a model it read changes.
- To open another page from a cell, select its URI: `uri('oxplow/commit',
  'sha', sha)`; see `semantic-layer.md`.

## `command`

```yaml
data:
  defs:
    command: oxplow.page.definition
    args: { page: { param: of } }
    reads: [v_page]
    items: nodes
```

- Only a **read** command runs (access `View` or `Read`), as the viewer;
  writes are actions.
- Say what makes it run again: `reads: [models]` or `refresh: manual` /
  `refresh: { every: 30s }`. Neither is an error.
- The result is `{ columns, rows }` as is, an array of objects (a row each),
  else one `value` row. `items: <field>` takes the list inside the result's
  `<field>` as the rows (a command that returns `{ title, nodes: […] }`).

## `page`

```yaml
data:
  count: { page: acme/numbers, loader: count, params: { n: 1 } }
```

Runs that page and takes its loader `loader`. Pages nest at most 8 deep;
the same definition with the same inputs runs once per run, embeds included.

## `rows`

```yaml
data:
  colors: { rows: [{ state: failed, tone: danger }, { state: passed, tone: success }] }
```

Inline rows (columns are the first row's keys). Use for lookup tables, a
snapshot of numbers you computed, and the `choices` of a form.

## Budgets

Do filtering, grouping and `LIMIT` in SQL. `oxplow_run_page` shows up to
200 rows per loader (json) with `truncated`, 50 in text. Keep a page to a
handful of loaders; one nothing shows is wasted work.
