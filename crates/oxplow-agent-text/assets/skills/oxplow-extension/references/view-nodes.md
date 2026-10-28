# The `view:` tree

`view:` is one node, usually a `stack`. An optional `rail:` is a second tree
drawn in the page's rail. Every node has an `id`: unique in the page, by
default its loader, embedded page, slot or component name, else `stack`,
`stack-2`, … Give a node an `id` to point at it: `#node=<id>` in a link
scrolls to it, and `oxplow_run_page` text prints each node under `[node
<id>]`. Any node may have `title`, `hide_title`, `replaceable` (a contract
another extension's page may take over) and `size`.

**A section needs a `title`.** A section is a child of a `stack`, `grid` or
`switch` (in `view:` or `rail:`). Its title says what it is: it is the
heading, the entry in the rail's Contents, and the heading `oxplow_run_page`
text prints. A section with no `title`, or `title: ''`, is refused when the
page loads. If the heading should not show (the section is plainly what the
page is, or the page's title already says it), add `hide_title: true`: the
heading is not drawn, but Contents, screen readers and agents still read it.
A page whose whole `view:` is one node needs no title, and a `slot` or
`embed` takes its title from the page it shows (an embed's own `title`
replaces it).

## Layout

```yaml
view:
  stack:                       # children one after another (sections)
    - { title: Total, hide_title: true, viz: number, from: total }
    - title: Activity
      grid:                    # children as tiles, each at its size
        - { title: By day, viz: bar,  from: by_day, size: wide, chart: { x: day, y: n } }
        - { title: Recent, viz: list, from: recent, size: tall }
```

- `stack: [nodes]`; `grid: [nodes]` (a child's `size`: `small` default,
  `wide`, `tall`, `full`).
- `switch: { param, children }` shows the one child whose `id` equals the
  param's value (else the first). The param is a filter, drawn in the rail.
  Loaders only hidden children show do not run. Children may be switches.
- `{ markdown: "text" }`: static text. `{ component: { id, props } }`: one of
  the extension's custom components, fed the page's loaders
  (`commands-and-forms.md`).

## `viz`

`{ viz: <kind>, from: <loader>, columns?, empty?, actions?, <role block> }`.
Rows come from the loader (a viz has no query). A viz that reads no rows
(`form`, `approvals`) has no `from`.

- `table`: rows. `list`: one line per row, headlined by the first column.
  `number`: the first cell, big. `markdown`: the first row's text
  (`markdown: { text: <column> }`; an action with `edit: true` makes it an
  editor, its Save that action: bind one input to `{{row.<text column>}}`
  and fix the rest with `fixed: [slug]`).
- `bar`: `chart: { x, y }`. `line`: `chart: { x, y, series?, targets? }`
  (`targets`: a target line from that column's last value). `treemap`:
  `chart: { label, size, group? }`; a tile click follows the first column's link.
- `tree`: `tree: { id, parent, label }`. A level at a time: `lazy: <param>`
  (a param with a `default`, the top level) and `branch: <column>`, one
  level per param value (`WHERE parent = :folder`); a folder opens by
  setting the param to its `id`.
- `board`: groups as columns of cards: `group: { by: <column>, values:
  [<columns in order>] }`. `timeline`, `steps`: ordered events or stages.
- `code`: each row's file at a revision, read-only: `code: { path,
  revision, lines?, sync? }`. `diff`: the first row's file between two
  revisions: `diff: { path, from, to, lines?, base? }` (several files:
  `hunks`). `editor`: edits the first row's working-tree file: `editor: {
  path, lines? }` (the person saves it).
- `embed`: the first row's web page (`http`/`https`) in a sandboxed window:
  `embed: { url: <column> }`. `terminal`: a terminal.
- `form`: one command's input (`commands-and-forms.md`).
- `custom`: `custom: { component: <id>, props }`, a sandboxed component;
  reach for it only when no built-in viz fits.

An inline page (`markdown-blocks.md`) may not use `form`, `approvals`,
`terminal`, `editor`, `custom`, actions, components or slots.

## Columns

`columns: [{ key, label?, width?, format?, link?, select?, highlight?,
expand?, icon?, tone?, unit? }]` (a column the viz doesn't list still
exists for actions and links).

- `width` is pixels; the rest share the space. `format`: `date`,
  `datetime`, `relative` (how long ago), `bytes`, `duration`
  (milliseconds), `integer`, `percent` (a 0-1 fraction), `sparkline` (a
  JSON array of numbers), `badge` (a pill in the column's `tone`). It
  changes only what the person sees; agents read the raw values.
- `link: { kind, from? }` makes cells clickable: `page` (any URI: a work
  item's `v_work_item.uri`, `oxplow:oxplow/commit?sha=…`), `file` (a path;
  `line: <column>`), `wiki` (a slug), `effort-diff`, `commit`, `metric`.
  `from` names the column holding the id (default: the column itself).
- `select: <param>`: a click sets that param to the column's value, and
  the page shows the picked row as its selection (master and detail with
  no code: loaders that read the param run again). Give the detail an
  `empty:` that says what to click.
- `highlight: <param>`: marks the param's value in the cell's text (use it
  with a search param). `expand: <page>`: opens that page under the row, its
  params taking the row's same-named columns.
- `icon: <column>` draws an icon named by that column's value (a work-item
  state, a file status, `epic`, `wiki`, `file`, `folder`, `commit`, …);
  `tone: <column>`: `accent`, `success`, `warning`, `danger`, `muted`.
  Unknown values draw nothing.

Row styling on `list`/`table`: `group: { by: <column>, link? }` puts rows
under a heading per value; `emphasis: <column>` highlights rows whose value
is truthy; `depth: <column>` indents.

## Actions

`actions: [{ id, label, command, input, row?, group?, when?, form?, confirm?,
prominent?, icon?, submenu?, drag?, key?, edit? }]` on a viz node (a row
action) or the page (`page-format.md`).

- `input` is the command's input; `"{{param.x}}"` binds a page param and,
  with `row: true` (a row's right-click action), `"{{row.col}}"` binds that
  row's column, which the loader must return even when `columns:` doesn't
  show it. e.g. `{ id: finish, label: Finish, command:
  oxplow.work_item.transition, row: true, input: { uri: "{{row.uri}}", to:
  done } }`.
- `form: true` opens the command's form instead of running it. `group:
  <value>` puts the button in that group's heading. `prominent: true` keeps
  one a button in the nav bar (only for what a person comes to do);
  `icon:` names an icon (`plus`, `pencil`, `play`, …).
- A page's actions without `row` or `group` go in its ☰ menu. Copy and Add
  to Context are on every page: don't declare them.
- An action runs as the page acting for whoever pressed it.

## `embed`

```yaml
- { embed: { page: acme/picked, params: { total: { from: first.total } } } }
```

Shows another page inline. Its params flow **by name**: the embed's
`params` first, else the host's param of that name, else that column of the
host loaders' first rows, in loader order. Embedded nodes are
`<embed-id>/<node-id>`.

## `slot`

```yaml
- { slot: sections }
```

A named place other extensions mount pages at (`ui-contributions.md`). The
mounted page runs as an embed: its params by name from the host; its node id
is the page id with `.` for `/` (`beta.extra`).

The YAML binds; logic is SQL or a read command, never the view.
