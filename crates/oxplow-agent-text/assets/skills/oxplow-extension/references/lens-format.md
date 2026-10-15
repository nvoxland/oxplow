# Lens and manifest format

## Write the files

In the worktree of the stream you're working in:

```
oxplow/extensions/<name>/extension.yaml
oxplow/extensions/<name>/lenses/<slug>.yaml
```

`<name>` and `<slug>` are lowercase-with-dashes. The lens id is
`<name>/<slug>`. Reuse an existing extension when the lens fits it; start a
new one for a new area.

`extension.yaml` (manifest v2):

```yaml
manifest: 2
name: review            # must equal the folder name
description: Lenses for reviewing agent work
sharing: private        # private is the default; `shared` = for the team, `engine` + `version` required
intent:                 # required: what it is for, and how to know it works
  purpose: Show what is waiting on the reviewer in this stream
  origin: effort:eff42  # the effort you are working under, or null
  examples:
    - name: a stream with one blocked task
      input: { lens: waiting-on-me, params: { stream_id: 1 } }
      expect: one row for the blocked task
  prompts:              # optional: questions it helps answer, offered with Ask
    - { prompt: "What's waiting on me?" }
    - { prompt: "Who should review this effort?", about: effort }  # also on effort pages
ui:                     # optional: what it adds to core pages
  slots:                #   mount a lens into a core page
    - { slot: work_item.detail.body, lens: task-history }
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
  - { name: stream_id, label: Stream }   # filled in: the viewer's stream
  # ref / effort_id / thread_id / stream_id are pickers on the lens page
query: |
  SELECT w.ref, w.title, w.state, t.title AS thread
  FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id
  WHERE w.state = 'blocked' AND t.stream_id = :stream_id
  ORDER BY w.updated_at DESC
viz: table                      # table | list | number | markdown | bar | line | treemap | grid | custom
columns:                        # optional; controls headers, order and links
  - { key: title, label: Item, link: { kind: page, from: ref } }
  - { key: thread }
  # - { key: took, unit: unit }  # a number shown in another column's unit (ms, %, lines)
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
  - `markdown` renders the first cell as markdown.
  - `bar` draws bars: `chart: { x: <label column>, y: <value column> }`.
  - `line` draws a trend: `chart: { x: <time or number>, y: <value>,
    series: <optional column, one line each> }`.
  - `treemap` draws nested rectangles: `chart: { label: <column>, size:
    <column>, group: <optional column> }`. A tile click follows the
    lens's first column link.
  - `grid` stacks other lenses: `children: [slug, other-ext/slug]`. Each
    child gets the params it declares from this lens's params.
  - `custom` renders one of the
    extension's `custom_components` in a sandboxed frame: `custom: {
    component: <id>, props: { … } }`. It still needs a `query` — its rows
    are what the component gets and what an agent reads (as a table).
    Reach for it only when no built-in viz fits.
- **`link.kind`** makes cells clickable:
  - `page` takes any page's ref — a work item's (`v_work_item.ref`),
    `commit:<sha>`, `page:git-dashboard`, ….
  - `file` takes a repo-relative path; add `line: <column>` to open at a
    line.
  - `wiki` takes a slug.
  - `effort-diff` takes an effort ref (`v_effort.ref`).
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
  `blocked`, `done`, `canceled`, `archived`, `epic`, `task`, `wiki`,
  `file`, `folder`, `commit`, `diff`, `lens`, `metric`, `dashboard`,
  `comment` or a ref-kind icon name (`bug`, `git-pull-request`, …) — and
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
  row: true, input: { ref: "{{row.ref}}", to: done } }`,
  or `{ id: sync, label: Sync PRs, command: oxplow.collector.sync, input:
  { collector: "collector:github/prs" } }`. With `group: <value>` (on a lens with
  `group`), the action is a button in that group's heading instead — it
  shows in a compact panel too; it can't be a row action. Copy and Add
  to Agent Context are on every lens already — don't declare them.

  An action runs as the lens acting for whoever pressed it, so it can't do
  anything they couldn't: you can press one with `oxplow_run_lens_action`, but a
  command you can't run (or that needs a person's confirmation, like
  approving an exec collector) is refused.
- **Unknown keys are errors.** Only the keys shown here exist today.
