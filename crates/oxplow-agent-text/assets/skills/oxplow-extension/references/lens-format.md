# Lens and manifest format

## Write the files

In the worktree of the stream you're working in:

```
.oxplow/extensions/<name>/extension.yaml
.oxplow/extensions/<name>/lenses/<slug>.yaml
```

`<name>` and `<slug>` are lowercase-with-dashes. The lens id is
`<name>/<slug>`. Reuse an existing extension when the lens fits it; start a
new one for a new area.

`extension.yaml` (manifest v2):

```yaml
manifest: 2
name: review            # must equal the folder name
description: Lenses for reviewing agent work
intent:                 # required: what it is for, and how to know it works
  purpose: Show what is waiting on the reviewer in this stream
  origin: oxplow:oxplow/effort?id=eff42  # the effort you are working under, or null
  examples:
    - name: a stream with one blocked task
      input: { lens: waiting-on-me, params: { stream_id: 1 } }
      expect: one row for the blocked task
  prompts:              # optional: questions it helps answer, offered with Ask
    - { prompt: "What's waiting on me?" }
    - { prompt: "Who should review this effort?", about: oxplow/effort }  # also on effort pages
ui:                     # optional: what it adds to core pages
  slots:                #   mount a lens into a core page
    - { slot: oxplow-foundation/work-item.sections, lens: task-history }
panels:                 # optional: a left-nav panel (body lens compact, badge = an alert lens's count)
  - { id: waiting, title: Waiting on Me, icon: bell, scope: stream, body: waiting-on-me, badge: waiting-on-me }
```

A manifest without `manifest: 2` doesn't load. Unknown keys are errors, and `check` reports every problem as
`file:line: what — fix`.

`lenses/<slug>.yaml`:

```yaml
title: Waiting on me
description: Blocked work items on this stream's threads.
params:                         # optional; bound as :name in the query
  - { name: stream_id, label: Stream }   # filled in: the viewer's stream; see "Params have a type"
  # uri / effort_id / thread_id / stream_id are pickers on the lens page
query: |
  SELECT w.uri, w.title, w.state, t.title AS thread
  FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id
  WHERE w.state = 'blocked' AND t.stream_id = :stream_id
  ORDER BY w.updated_at DESC
viz: table                      # table | list | number | markdown | bar | line | treemap | document | dashboard | custom
columns:                        # optional; controls headers, order and links
  - { key: title, label: Item, link: { kind: page, from: uri } }
  - { key: thread }
  # - { key: took, unit: unit }  # a number shown in another column's unit (ms, %, lines)
  # - { key: created_at, format: relative, width: 120 }  # width in pixels; format is display only
empty: Nothing is waiting on you.
```

- **"Mine" without hardcoding ids**: a param named `stream_id` or
  `thread_id` is filled in with the viewer's current stream / thread
  (numeric, like the `v_*` columns) unless a value is given. The lens
  page uses the stream the person is on and its selected thread;
  `oxplow_run_lens` uses the `stream_id` / `thread_id` you pass (default: the
  stream's selected thread). A `default:` applies only when there is no
  current value.
- **`viz`**:
  - `table` shows rows.
  - `list` shows one line per row, headlined by the first column.
  - `number` shows the first cell as a big number.
  - `markdown` renders the first row's text as markdown:
    `markdown: { text: <column> }` (else the first column). An action
    with `edit: true` makes it an editor whose Save that action is: bind
    one input field exactly to `{{row.<text column>}}` (the text fills
    it) and fix the rest (`fixed: [slug]`).
  - `bar` draws bars: `chart: { x: <label column>, y: <value column> }`.
  - `line` draws a trend: `chart: { x: <time or number>, y: <value>,
    series: <optional column, one line each> }`.
  - `treemap` draws nested rectangles: `chart: { label: <column>, size:
    <column>, group: <optional column> }`. A tile click follows the
    lens's first column link.
  - `code` shows each row's file read-only at a revision, at full size:
    `code: { path: <column>, revision: <column>, lines: <optional column
    holding 12-40>, sync: <optional bool: editors scroll together> }`.
  - `diff` shows the first row's file between two revisions, at full
    size: `diff: { path: <column>, from: <column>, to: <column>, lines:
    <optional>, base: <optional: the older side's name in the header> }`. Several files are `hunks`.
  - `board` draws the groups as columns of cards: `group: { by:
    <column>, values: [<the columns in order>] }`.
  - `document` is a wiki-style page of sections: `sections: [...]`. See
    "Document and dashboard lenses" below.
  - `dashboard` is a responsive grid of titled cards: `tiles: [...]`. See
    "Document and dashboard lenses" below.
  - `custom` renders one of the
    extension's `custom_components` in a sandboxed frame: `custom: {
    component: <id>, props: { … } }`. It still needs a `query` — its rows
    are what the component gets and what an agent reads (as a table).
    Reach for it only when no built-in viz fits.
- **`columns[].width` and `format`**: `width` is in pixels. The columns
  without one share the rest. `format` is one of `date`, `datetime`,
  `relative` (how long ago), `bytes`, `duration` (milliseconds),
  `integer`, `percent` (a 0-1 fraction), `sparkline` (a JSON array of
  numbers, drawn as a small line with its latest value) or `badge` (a
  pill in the column's `tone`). `format` changes only what the person
  sees. Agents read the raw values.
- **`columns[].expand: <lens>`** opens that lens (a slug of the
  extension, or `<extension>/<slug>`) under the row, run with the row's
  values: the params it declares take the same-named columns.
- **`chart.targets: <column>`** (`line` only) draws a target line on each
  series, from that column's last value in it.
- **Params have a `type`**, which picks their control on the page:
  `text` (the default), `choice` (`options: [{ value, label }]` and a
  `default` that is one of them), `bool` (bound 1 or 0), `search`
  (trimmed text, NULL when empty; applies as the person types), `branch`
  (a branch name, NULL for all) and `date_range` (a preset `1d 2d 3d 7d
  30d 90d all`, or `<from>..<to>`; pick the quick picks with `presets:`).
  A `date_range` named `range` gives the query `:range_from` and
  `:range_to` (RFC 3339, NULL for an open end):
  ```yaml
  params:
    - { name: range, label: When, type: date_range, default: 30d, presets: [7d, 30d, 90d, all] }
    - { name: q, label: Search subjects, type: search }
    - { name: merges, label: Merges only, type: bool, default: false }
  query: |
    SELECT sha, subject FROM v_commit
     WHERE (:range_from IS NULL OR committed_at >= :range_from)
       AND (:q IS NULL OR subject LIKE '%' || :q || '%')
       AND (:merges = 0 OR parent_count > 1)
  ```
  You don't say where a param draws: the page puts every param in its
  rail. A param's `role` says where its value rides in the URI. The
  default, `resource`, names what the page shows (in the `?`; a change
  opens another resource). Write `role: filter` on a param that only
  narrows or re-views it (a search, a range): it rides in the `#` and a
  change replaces the entry. Never write `role: selection`: a column's
  `select` makes it (in the `#`; a pick pushes). A `filter` or
  `selection` found in a `?` is refused. Read `references/page-layout.md`
  before you add params to a page.
- **`columns[].select: <param>`** makes a row clickable: a click sets
  that param to the column's value, and the page shows the picked row as
  its selection (never an input). In a document, the param is the
  document's, so the other sections that read it run again — master and
  detail with no code. Give the detail section an `empty:` that says
  what to click.
- **`columns[].highlight: <param>`** marks the param's current value in
  the cell's text (every case-insensitive occurrence; nothing when the
  param is empty). It must name a param the lens declares. Use it on the
  columns of a search lens, with its filter param.
- **`link.kind`** makes cells clickable:
  - `page` takes any page's ref — a work item's (`v_work_item.uri`),
    `oxplow:oxplow/commit?sha=<sha>`, `oxplow:oxplow/page?page=git-dashboard`, ….
  - `file` takes a repo-relative path; add `line: <column>` to open at a
    line.
  - `wiki` takes a slug.
  - `effort-diff` takes an effort ref (`v_effort.uri`).
  - `commit` takes a sha.
  - `metric` takes a metric key.
  
  `from` names the result column that holds the id; it defaults to the
  column itself.
- **`launcher: { category: Activity }`** lists the lens under that
  launcher heading (Work, Code, Git, Activity, Knowledge, Data, Lenses,
  System; default Lenses). **`hidden: true`** keeps it out of the
  launcher, for lenses only a slot shows.
- **Something to run from the launcher** is a command with a `ui` (`ui:
  { label, group, input? }`): it can run another command with fixed input
  (a `scope:` + `op:` over the same operation, or a script composing
  it), or put a prompt in the agent's input (`scope:
  agent_input.write`, `op: draft`, `ui.input: { text: "…" }`). A page is a
  `pages:` entry or a lens.

- **Row styling** on `list` / `table` (each names a result column):
  `group: { by: bucket, link?: { kind, from } }` puts the rows under a
  heading per `by` value, in the order each first appears (the heading
  links from the group's first row); `emphasis: is_current` highlights a
  row whose value is truthy (also on `tree`); `depth: level` indents a
  row by its whole-number value. On a column, `icon: status` draws an
  icon named by that column's value — `ready`, `todo`, `in_progress`,
  `blocked`, `done`, `canceled`, `archived`, a file's status (`added`, `modified`, `deleted`, `renamed`, `untracked`, `conflicted`), `epic`, `task`, `wiki`,
  `file`, `folder`, `commit`, `diff`, `lens`, `metric`, `dashboard`,
  `comment` or a resource icon name (`bug`, `git-pull-request`, …) — and
  `tone: hue` colours the cell: `accent`, `success`, `warning`, `danger`,
  `muted`. Unknown values draw nothing. Rows that link somewhere drag
  onto the agent terminal to add the page to its context.
- **`alert:`** says when a lens needs attention: `{ min_rows: 1 }` (the
  run returned at least that many rows) or `{ column: pct, below: 80 }` /
  `above:` (the first row's value), with an optional `label`. `oxplow_run_lens`
  returns the alert state.
- **`needs:`** (a lens or a hint) names the capabilities or features it
  can't run without, e.g. `needs: [work_items, snapshots.contents]`. A
  lens that reads a file's text from a past snapshot declares
  `snapshots.contents`: under "Track changes only" the snapshots keep no
  text, and the lens shows as inactive with that reason instead of failing.
- **`actions:`** are commands the lens offers:
  `{ id, label, command, input, row? }`. `input` is the command's input;
  `"{{param.x}}"` binds a lens param and, with `row: true` (a row's
  right-click action), `"{{row.col}}"` binds that row's column — which
  the query must return even when `columns:` doesn't show it (select a
  ref for the action and leave it out of `columns:`) —
  e.g. `{ id: finish, label: Finish, command: oxplow.work_item.transition,
  row: true, input: { uri: "{{row.uri}}", to: done } }`,
  or `{ id: sync, label: Sync PRs, command: oxplow.collector.sync, input:
  { collector: "github/prs" } }`. With `group: <value>` (on a lens with
  `group`), the action is a button in that group's heading instead — it
  shows in a compact panel too; it can't be a row action. Copy and Add
  to Agent Context are on every lens already — don't declare them.

  An action runs as the lens acting for whoever pressed it, so it can't do
  anything they couldn't: you can press one with `oxplow_run_lens_action`, but a
  command you can't run (or that needs a person's confirmation, like
  deleting a file) is refused.
- **Unknown keys are errors.** Only the keys shown here exist today.

## Document and dashboard lenses

A `document` or `dashboard` lens needs no `query`. It shows other lenses
(and prose) as parts. A query it has is one row: each column is a param
its parts are given (a value the page holds wins), e.g. `SELECT id AS
change_id FROM v_change WHERE kind = 'commit' AND target = :sha`.

```yaml
# lenses/overview.yaml: a document
title: Project overview
params:
  - { name: stream_id, label: Stream }
viz: document
sections:
  - id: intro
    markdown: |
      What needs attention in this stream.
  - lens: waiting-on-me
  - lens: review/recent-commits
    title: Latest commits
```

```yaml
# lenses/health.yaml: a dashboard
title: Health
params:
  - { name: stream_id, label: Stream }
viz: dashboard
tiles:
  - lens: waiting-on-me
  - lens: token-trend
    size: wide
```

Each entry is `{ id?, title?, lens?, markdown?, size? }`:

- Give exactly one of `lens` and `markdown`. `lens` is a slug in this
  extension, or `<ext>/<slug>`.
- `id` defaults to the lens slug. A markdown part needs an `id`. Ids are
  unique in the lens. To show one lens twice, give one of them an `id`.
- `title` defaults to the lens's title.
- Each lens part runs with the params it declares, taken from the
  document's or dashboard's params.
- A `document` folds the sections with no rows into one line:
  "Nothing found: ...".
- `size` is `small` (default), `wide`, `tall` or `full`. A markdown tile
  defaults to `full`. `size` is refused on a section.
- `tiles` on a document, and `sections` on a dashboard, are refused.
- A standalone lens spec (an agent answer from `oxplow_show_lens`) can't
  be a `document` or `dashboard`.

### Adding parts from another extension

Another extension adds to a document or dashboard through a slot named
`<extension>/<slug>.sections` (document) or `<extension>/<slug>.tiles`
(dashboard), in `ui.slots`:
`{ slot, lens, at?, title?, size? }`.

```yaml
# extension.yaml of the "audit" extension
ui:
  slots:
    - slot: project/overview.sections
      lens: stale-reviews
      at: { after: intro }
      title: Stale reviews
```

- `at` is `start`, `end` (default), `{ after: <part id> }` or
  `{ before: <part id> }`. An `at` that names a part id the target
  doesn't have is a load warning. The mount goes at the end.
- The target's params are the slot's params. The mounted lens must
  declare at least one of them (when the target has any).
- The mounted part shows marked with the extension that added it.
- Foundation pages are document lenses, so they have parts slots like any:
  `oxplow-foundation/new-session.sections` (part ids `start` and
  `earlier`; binds `thread_id`).
- `at` is refused on a core region slot, such as `thread.plan.header`.
  `size` is allowed only on a `.tiles` slot.
