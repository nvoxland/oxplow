# Loaders (`data:`)

`data:` is a mapping of named loaders. Each gets rows; a node shows one
by `from: <name>`. Loaders run **in order** and a loader reads only the
ones **before** it. One that fails carries its error and the rest of the
page still shows. A loader that no visible node shows still runs when a
later loader, an embed or the badge uses it; one only a hidden `switch`
child shows does not.

A loader has exactly one of `sql`, `command`, `page` or `rows`, plus
optional `columns` (column keys, labels and formats nodes default to),
`empty` (text when it has no rows).

**Names** are lowercase letters, digits and `_`, starting with a letter.
Never `v_…`, `m_…`, `temp` or `main`: they must not look like a model.

## `sql`

```yaml
data:
  base:  { sql: "SELECT id, state FROM v_work_item" }
  open:  { sql: "SELECT * FROM base WHERE state = :state" }   # base is a table
  child:
    sql: "SELECT title FROM v_work_item WHERE parent = :p"
    params: { p: { from: open.id } }                          # first row's id
```

- One read-only statement over the `v_*` models (`oxplow_query_sql`'s
  rules; read column names from `v_model_column`, never guess).
- `:name` binds a page param, or a `params:` entry. `params` values are
  `{ param: name }`, `{ from: <loader>.<column> }` (the loader's **first
  row**) or a literal.
- An earlier loader reads as a **temp table of its name**. Naming a later
  loader is an error.
- It reruns when a model it read changes.

## `command`

```yaml
data:
  q:
    command: oxplow.sql.query
    args: { sql: "SELECT 7 AS seven" }
    reads: [v_model]
```

- Only a **read** command (access `View` or `Read`) runs, as the viewer.
  A write command is refused: writes are actions.
- `args` values are `{ param }`, `{ from }` or literals.
- Say what makes it run again: `reads: [models it reads]`, or
  `refresh: manual` / `refresh: { every: 30s }`. Neither is an error.
- The result is `{ columns, rows }` as is, an array of objects one row
  each, else a single `value` row.

## `page`

```yaml
data:
  count: { page: acme/numbers, loader: count, params: { n: 1 } }
```

Runs that page and takes its loader `loader`, giving it `params`. Pages
nest at most 8 deep. The same definition with the same inputs runs once
per run, embeds included.

## `rows`

```yaml
data:
  colors: { rows: [{ state: failed, tone: danger }, { state: passed, tone: success }] }
```

Inline rows: columns are the keys of the first row. Use for lookup tables
and for the `choices` of a form.

## Budgets

A sql loader's result is capped like `oxplow_query_sql`'s; `oxplow_run_page
{ format: json }` shows up to 200 rows per loader with `truncated`. Do the
filtering, grouping and `LIMIT` in SQL. Keep a page to a handful of
loaders; a loader that nothing shows is wasted work.
