# The `view:` tree

`view:` is one node, usually a `stack`. An optional `rail:` is a second
tree drawn in the page's rail. Every node has an `id`: unique in the page,
by default its loader, embedded page, slot or component name, else
`stack`, `stack-2`, … Give a node an `id` when you want to point at it:
`#node=<id>` in a link scrolls to it, and `oxplow_run_page` text prints each
node under `[node <id>]`. Any node may have `title`, `replaceable`
(a contract another extension's provider may take over) and `size`.

## Layout

```yaml
view:
  stack:                       # children one after another (sections)
    - { viz: number, from: total }
    - grid:                    # children as tiles, each at its size
        - { viz: bar,  from: by_day, size: wide }
        - { viz: list, from: recent, size: tall }
```

- `stack: [nodes]`, `grid: [nodes]` (a child's `size`: `small` (default), `wide`, `tall`, `full`).
- `switch: { param, children }`: shows the one child whose `id` equals the
  param's value (else the first). The param is a filter by default, drawn
  in the rail. Loaders only hidden children show do not run.

## `viz`

`{ viz: <kind>, from: <loader>, columns?, empty?, actions?, <role block> }`.
`viz` is a lens viz (`table`, `list`, `tree`, `number`, `markdown`,
`bar`, `line`, `treemap`, `timeline`, `steps`, `diff`, `code`, `editor`,
`terminal`, `form`, `custom`, …). Its role block (`chart:`, `tree:`,
`form:`, …), `columns` and row `actions` are the lens ones: see
`lens-format.md`. Differences: rows come from the loader, so there is no
`query`; a viz that reads no rows (`form`, `approvals`) has no `from`.
**Row actions** go here, with `row: true` and `{{row.col}}`.

## `embed`

```yaml
- { embed: { page: acme/picked, params: { total: { from: first.total } } } }
```

Shows another page inline. Its params flow **by name**: the embed's
`params` first, else the host's param of that name, else that column of
the host loaders' first rows, in loader order. Embedded nodes are
`<embed-id>/<node-id>`.

## `slot`

```yaml
- { slot: sections }
```

A named place other extensions mount pages at. Declare it and mount into
it from another extension:

```yaml
# beta/extension.yaml
ui:
  slots:
    - { slot: acme/slotted.sections, page: extra }                  # at the slot
    - { slot: acme/slotted.sections, page: early, at: start }       # first among siblings
    - { slot: acme/slotted.sections, page: late, at: { after: last } }  # beside a sibling id
```

`slot: <extension>/<page>.<name>`, `page` is a slug of the mounting
extension. Each mounted page runs as an embed: its params by name from
the host. Its node id is the page id with `.` for `/` (`beta.extra`).

## `markdown` and `component`

- `{ markdown: "text" }` (a node `id` is useful): static text.
- `{ component: { id, props } }`: one of the extension's custom components
  (`commands-and-forms.md`) fed the page's data.

The page YAML binds; logic lives in SQL or a read command, never in the
view.
