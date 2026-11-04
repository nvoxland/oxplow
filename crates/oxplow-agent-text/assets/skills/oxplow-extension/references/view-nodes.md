# The `view:` tree

`view:` is one node, usually a `stack`. An optional `rail:` is a second tree
drawn first in the page's rail, under its root's `title` as a group (bare
without one): a fact about the page as a whole (`title: About`, a `detail`
of the branch a history is of) goes there, never in a section of `view:`. Every node has an `id`: unique in the page, by
default its loader, embedded page, slot or component name, else `stack`,
`stack-2`, … Give a node an `id` to point at it: `#node=<id>` in a link
scrolls to it, and `oxplow_run_page` text prints each node under `[node
<id>]`. Any node may have `title`, `hide_title`, `replaceable` (a contract
another extension's page may take over) and `size`.

**A section needs a `title`.** A section is a child of a `stack`, `grid` or
`switch` (in `view:` or `rail:`). Its title says what it is: it is the
heading, the entry in the rail's Contents, and the heading `oxplow_run_page`
text prints. A section with no `title`, or `title: ''`, is refused when the
page loads. Hide a heading (`hide_title: true`) only for a section that
fills the page (a diff, a terminal, the page's one tree) or is a one-line
banner or intro above the rest; a section of facts, a list, a table or a
chart among others always shows its title (the page's own title is in the
nav bar, not above the body, so the first section's heading is what tells
the reader where they are). A hidden heading is still in Contents, screen
readers and agents' text.
A page whose whole `view:` is one node needs no title, and a `slot` or
`embed` takes its title from the page it shows (an embed's own `title`
replaces it).

Related pages are not a node: the page key `see_also:` (`page-format.md`) puts
them in the rail as "See also", after Contents.

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
- `description: <sentence>` on any section: what it holds and what the
  person does there. The page shows it as a ⓘ after the heading (the
  sentence is its hover), and `oxplow_run_page` prints it under the
  heading. Write one where the heading alone would leave a reader
  guessing (an Alerts section: what lands there and how it leaves).
- `switch: { param, children }` shows the one child whose `id` equals the
  param's value (else the first). The param is a filter, drawn in the rail.
  Loaders only hidden children show do not run. Children may be switches.
- `{ markdown: "text" }`: static text. `{ component: { id, props } }`: one of
  the extension's custom components, fed the page's loaders
  (`commands-and-forms.md`).

## `viz`

`{ viz: <kind>, from: <loader>, columns?, empty?, hide_header?, lead?, actions?, <role block> }`.
Rows come from the loader (a viz has no query). A viz that reads no rows
(`approvals`) has no `from`. A section is never left out for being
empty: it stays over its `empty:` line, so write one worth reading
("No commands.", "Nothing to commit."); a `form` over a loader with no
rows draws its `empty:` line instead of its fields.
`hide_header: true` leaves a table's header row out, for columns that explain
themselves.
`fold: true` starts the rows closed under a row count, for a long list
nobody reads by default.
`lead: <column>` puts that column's first-row text as a plain paragraph
under the node's heading, above its viz: what a thing is, above its facts
(null or blank draws nothing, no gap).

- `table`: rows. `list`: one line per row, headlined by the first column.
  `number`: the first cell, big. `markdown`: the first row's text
  (`markdown: { text: <column>, base?: <column> }`; `base` holds the URI of
  the file the text came from, so its relative links and images are that
  file's folder's; an action with `edit: true` makes it an
  editor, its Save that action: bind one input to `{{row.<text column>}}`
  and fix the rest with `fixed: [slug]`).
- `bar`: `chart: { x, y }`. `line`: `chart: { x, y, series?, targets? }`
  (`targets`: a target line from that column's last value). `treemap`:
  `chart: { label, size, group? }`; a tile click follows the first column's link.
- `tree`: `tree: { id, parent, label, value?, count? }`. `value` and `count`
  name columns drawn beside the label (one line, the whole value on hover; `(n)`).
  A level at a time: `lazy: <param>`
  (a param with a `default`, the top level) and `branch: <column>`, one
  level per param value (`WHERE parent = :folder`); a folder opens by
  setting the param to its `id`.
- `sections`: one subsection per row, in row order: `sections: { title:
  <column>, id?, link?, body?, group?, group_link? }`. Use it when a page is about several
  things (every extension, every failing check, every page of one): each
  thing gets a headed section, not a table row (a wide table is for data
  that is really tabular). A heading is never a link: when `link` names a
  column holding a URI, an arrow after the heading text ("Open <heading>")
  opens that page. Under its heading a subsection draws the viz's `columns` as a short
  facts list, then `body` (a column of markdown). Its `id` column (default:
  a column named `id`, else the title) keys the subsection (`data-node`
  `<node id>/<id>`), so give it a stable one. The viz's row `actions` are
  in each subsection's right-click menu (`when:` per row, as for a row).
  `group: <column>` (and `group_link: <column of URIs>`) puts rows with
  the same value (groups in the order each first appears) under one group
  heading, a section one level above the rows' subsections, with the same
  arrow when `group_link` is set; a row with no group value stays at the
  groups' level. The
  subsections are real sections: their heading is one level below the
  section the viz is in. A page with two or more sections has a Contents
  (a `sections` viz's rows and groups count), which lists up to 10 of them
  under their parent (per group too) and then one "... N more" that goes to
  the parent (or the group). It stays in
  the reading column. An agent reads each as a `###` heading with its facts
  and body.
- `board`: groups as columns of cards: `group: { by: <column>, values:
  [<columns in order>] }`. `timeline`, `steps`: ordered events or stages.
- `code`: each row's file read-only, on a filesystem at a revision (a
  column each; none: the working tree): `code: { path, filesystem?,
  revision?, lines?, sync? }`. `diff`: the first row's file between two
  revisions: `diff: { path, from, to, lines?, base? }` (several files:
  `hunks`). `editor`: edits the first row's working-tree file: `editor: {
  path, filesystem?, lines? }` (the person saves it; only the local
  working tree). `media`: each row's file shown as what it is, an image or
  a PDF, from its bytes: `media: { path, filesystem?, revision? }` (a file
  that is neither says so).
- `embed`: the first row's web page (`http`/`https`) in a sandboxed window:
  `embed: { url: <column> }`. `terminal`: a terminal.
- `form`: one command's input (`commands-and-forms.md`).
- `custom`: `custom: { component: <id>, props }`, a sandboxed component;
  reach for it only when no built-in viz fits.

An inline page (`markdown-blocks.md`) may not use `form`, `approvals`,
`terminal`, `editor`, `custom`, actions, components or slots.

### Example: `sections`

```yaml page
title: Things
loaders:
  things:
    sql: |
      SELECT name AS id, name, uri, kind, summary, failing FROM v_thing ORDER BY name
view:
  stack:
    - title: Things
      viz: sections
      from: things
      sections: { title: name, id: id, link: uri, body: summary }
      columns:
        - { key: kind, label: Kind }
      empty: Nothing here yet.
      actions:
        - { id: fix, label: Repair, command: x.thing.repair, row: true,
            when: failing, input: { name: "{{row.id}}" } }
```

To put the rows under a heading each, add `group: owner` and
`group_link: owner_uri` (columns of the loader) to `sections:`: a "Problems"
page with the extension as the group and each problem a subsection.

Each `things` row is a subsection of "Things": the name as a heading
with an arrow to its page, "Kind" as a fact, `summary` as markdown, and Repair in
its right-click menu where `failing` is set. A "Problems" line is one more
`body` line (a link in markdown); nesting is by what the SQL puts in
`body`.

## Columns

`columns: [{ key, label?, width?, format?, link?, select?, highlight?,
expand?, icon?, tone?, unit?, when? }]` (a column the viz doesn't list still
exists for actions and links).

- `width` is pixels; the rest share the space. `format`: `date`,
  `datetime`, `relative` (how long ago), `bytes`, `duration`
  (milliseconds), `integer`, `percent` (a 0-1 fraction), `sparkline` (a
  JSON array of numbers), `badge` (a pill in the column's `tone`), `items`
  (below). It changes only what the person sees; agents read the raw values
  (but for `items`).
- `format: items`: the value is a JSON array of items, each a string or
  `{ "text": ..., "hint"?: ..., "uri"?: ... }`, drawn inline separated by
  " · " in a `detail` or `sections` fact, a `table` or `list` cell. `hint`
  is the item's hover text, `uri` makes it a link. Build the array in SQL
  (`json_group_array(json_object('text', name, 'hint', why))`). An agent
  reads `text (hint) · text`.
- `link: { kind, from? }` makes cells clickable: `page` (any URI: a work
  item's `v_work_item.uri`, `oxplow:oxplow/commit?sha=…`), `file` (a path;
  `line: <column>`), `wiki` (a slug), `effort-diff`, `commit`, `metric`.
  `from` names the column holding the id (default: the column itself).
- `select: <param>`: a click sets that param to the column's value, and
  the page shows the picked row as its selection (master and detail with
  no code: loaders that read the param run again). Give the detail an
  `empty:` that says what to click.
- `highlight: <param>`: marks the param's value in the cell's text (use it
  with a search param). `expand: <part>` (or a page): opens it under the row, its
  params taking the row's same-named columns.
- `icon: <column>` draws an icon named by that column's value (a work-item
  state, a file status, `epic`, `wiki`, `file`, `folder`, `commit`, …);
  `tone: <column>`: `accent`, `success`, `warning`, `danger`, `muted`.
  `when: <column>` (a `detail`): the fact is drawn only while that column
  of the row is truthy, as an action's `when`; a fact that holds only
  sometimes (a merge's second parent) rather than a section of its own.
  Unknown values draw nothing.

Row styling on `list`/`table`: `group: { by: <column>, link?, labels? }` (`labels: { stored_value: Heading }` names a heading in plain words) puts rows
under a heading per value; `emphasis: <column>` highlights rows whose value
is truthy; `depth: <column>` indents.

## Actions

`actions: [{ id, label, command, input, row?, group?, all?, when?, form?,
confirm?, prominent?, icon?, submenu?, drag?, key?, edit? }]` on a viz node (a
row action) or the page (`page-format.md`).

- `input` is the command's input; `"{{param.x}}"` binds a page param and,
  with `row: true` (a row's right-click action), `"{{row.col}}"` binds that
  row's column, which the loader must return even when `columns:` doesn't
  show it. e.g. `{ id: finish, label: Finish, command:
  oxplow.work_item.transition, row: true, input: { uri: "{{row.uri}}", to:
  done } }`.
- A viz node's action without `row` is a button under its rows. `all:
  true` says it acts on every row shown (a Clear, a Retry all): the
  button appears only while there are rows. Leave it off a button that
  needs no rows (an Add, a New…).
- Every titled section stays on the page, empty or not, over its `empty:`
  line: give each one worth reading ("No notices.", not "No rows.").
- `form: true` opens the command's form instead of running it. `group:
  <value>` puts the button in that group's heading. `prominent: true` keeps
  one a button in the nav bar (only for what a person comes to do);
  `icon:` names an icon (`plus`, `pencil`, `play`, …).
- A page's actions without `row` or `group` go in its ☰ menu. Copy and Add
  to Context are on every page: don't declare them.
- An action runs as the page acting for whoever pressed it.
- `open: <column>` instead of `command` opens the URI that column of the row
  holds, and runs nothing (`page-format.md` "Whole-page actions").

## `embed`

```yaml
- { embed: { page: acme/picked, params: { total: { from: first.total } } } }
```

Shows a part, or another page, inline. Its params flow **by name**: the embed's
`params` first, else the host's param of that name, else that column of the
host loaders' first rows, in loader order. Embedded nodes are
`<embed-id>/<node-id>`.

## `slot`

```yaml
- { slot: sections }
```

A named place other extensions mount parts at (`ui-contributions.md`). The
mounted part runs as an embed: its params by name from the host; its node id
is the part id with `.` for `/` (`beta.extra`).

The YAML binds; logic is SQL or a read command, never the view.
