# Lenses

A lens is a page you (or your agent) build: a SQL query over oxplow's data
plus how to show it. It lives as a small YAML file in your repo, shows up in
the launcher (++cmd+p++, under **Lenses**), and re-runs as the data changes.

The quickest way to get one is to ask your agent: "make me a lens of the
tasks blocked in this stream". It knows the format.

## The data

Everything oxplow tracks is exposed as read-only SQL views: `v_work_item`
(the active work list's items, whichever tracker holds them), `v_effort`, `v_thread`, `v_comment`, `v_wiki_page`, `v_effort_file`,
`v_agent_turn`, `v_token_usage`, `v_fact`, and more.

**Explore Data** (launcher → Data) lists every view with a description of each
column. Pick one to see sample rows, edit the SQL (++cmd+enter++ runs it), and
**Save as Lens** when it shows what you want.

## Writing one by hand

Lenses live in `.oxplow/extensions/<extension>/lenses/<slug>.yaml`. The
extension folder needs an `extension.yaml`:

```yaml
# .oxplow/extensions/review/extension.yaml
manifest: 2
name: review          # must match the folder name
description: Lenses for reviewing agent work
intent:
  purpose: Show what's blocked, so it gets unblocked
  examples:
    - { name: blocked, input: { lens: blocked } }
```

`intent` is required: what the extension is for, and examples that
`oxplow extension test` runs. `oxplow extension new extension review` writes this
file for you. The [Extensions reference](../reference/extensions.md) covers
everything else an extension can declare.

```yaml
# .oxplow/extensions/review/lenses/blocked.yaml
title: Blocked Work
params:
  - { name: thread_id, label: Thread }
query: |
  SELECT ref, title, updated_at FROM v_work_item
  WHERE state = 'blocked' AND thread_id = :thread_id
viz: table            # table | list | number | markdown | bar | line | treemap | document | dashboard | custom
columns:
  - { key: title, label: Item, link: { kind: page, from: ref } }
  - { key: updated_at, label: Updated, format: relative, width: 120 }
empty: Nothing is blocked.
```

- `params` become inputs on the page and bind as `:name`.
- A param named `stream_id` or `thread_id` is filled in with the stream
  you're on and its selected thread, so "my stream" needs no hardcoded
  id. Type a value to look at another one.
- `link.kind` is `page` (any oxplow page by its ref, such as a work
  item's), `file` (add `line: <column>` to open at a line), `wiki`,
  `effort-diff`, `commit`, `metric`, `diff-at` or `compare`.
  - `diff-at` opens a file's diff within a change:
    `{ kind: diff-at, line: start_line, base: base_revision, head: head_revision }`
    (join `v_change` for the two revisions).
  - `compare` opens two line ranges side by side; the column holds
    `path:start-end|peer:start-end`, and `head:` names a revision column.
  - A revision is `working` (the files on disk), `snap:<id>` (a
    local-history snapshot) or `git:<rev>` (a sha, branch or `HEAD`).
- Unknown keys are errors, so typos show up instead of being ignored.

### Typed params

A param's `type` decides what it holds and which control the page draws:

```yaml
params:
  - { name: range, type: date_range, default: 7d, presets: [1d, 7d, 30d, all] }
  - { name: q, type: search }
  - { name: branch, type: branch }
  - { name: merged, type: bool, default: false }
  - { name: state, default: open, options: [{ value: open, label: Open }, { value: done, label: Done }] }
query: |
  SELECT sha, subject FROM v_commit
  WHERE (:range_from IS NULL OR committed_at >= :range_from)
    AND (:range_to IS NULL OR committed_at < :range_to)
    AND (:q IS NULL OR subject LIKE '%' || :q || '%')
```

- `text` (the default) is a text box; with `options` a param is a
  `choice`, drawn as a select.
- `bool` is a checkbox and binds 1 or 0.
- `search` applies as you type; empty binds NULL.
- `branch` picks one of the repository's branches; "All branches" binds NULL.
- `date_range` is a preset (`1d`, `2d`, `3d`, `7d`, `30d`, `90d`, `all`)
  or a custom `from..to`. The query gets `:<name>_from` and
  `:<name>_to` as times, NULL for an open end; a custom range includes
  its last day.

Every param shows in the page's side panel. You don't choose where.
The values you change are kept in the page's tab: a bookmark, a
reopened tab or a restart shows the same filters and the same row
picked. Changing a filter doesn't add a step to Back; picking a row
does, so Back un-selects it.
Give each one a short `label`, and declare them in the order people use
them: the one changed most first, toggles last. Six show; the rest fold
behind "More".

A column's `select: <param>` makes a row clickable: clicking it sets the
param to that column's value. In a document, the other sections that
read the param re-run, so one section can be the list and another the
detail of the row picked. The document declares the param too.

Charts name the columns they draw:

```yaml
viz: bar              # or line
chart: { x: day, y: visits }          # line also takes series: <column>
```

```yaml
viz: treemap
chart: { label: path, size: churn, group: zone }
```

A column can also set `width` (in pixels). A long value in that column
is cut to one line, and the whole value shows when you hover over it.
A column can also set `format`: `date`,
`datetime`, `relative` (how long ago), `bytes`, `duration`
(milliseconds), `integer` or `percent` (a 0-1 fraction). `format`
changes only what you see. Agents read the raw values.

A column can set `highlight: <param>` to mark that param's current value
in its text, wherever it occurs (any case). A search lens uses it on its
title and snippet columns, with its filter param. The param must be one
the lens declares.

`launcher: { category: Activity }` files a lens under that launcher
heading; `hidden: true` leaves it out (for lenses only a slot shows).
`actions:` runs commands. Each action names a command and its input,
and becomes a button above the result, or, with `row: true`, an item in
each row's menu (right-click, or Menu key / Shift+F10 on a row):

```yaml
actions:
  - id: finish
    label: Finish
    command: oxplow.work_item.transition
    input: { ref: "{{row.ref}}", to: done }
    row: true
  - id: sync
    label: Sync PRs
    command: oxplow.collector.sync
    input: { owner: github, id: prs }
```

`{{param.x}}` and `{{row.col}}` fill the input from the lens's params or
the row. A command that asks for confirmation asks you before it runs.

With `form: true` an action opens the command's form in a side panel
instead of running at once, its fields starting from `input`; you edit
them and submit (Enter). `fixed` lists the fields you see but can't
change — they keep `input`'s values, so an `Edit…` on a row always edits
that row's item:

```yaml
actions:
  - id: edit
    label: Edit…
    command: oxplow.work_item.update
    input: { ref: "{{row.ref}}", title: "{{row.title}}" }
    row: true
    form: true
    fixed: [ref]
```

A `viz: form` lens takes the same `fixed`, and `fields` to show only some
of the command's fields. When the command refuses a field (an empty
title), the message shows under that field.

A form that edits something already saved says `mode: edit`: Save is
enabled only when you changed something, Discard (or Escape) puts back
what is saved, and what you typed stays through Back and a restart. A
`fields` entry can show a field only for some values of another, and can
be secret:

```yaml
viz: form
form:
  command: acme.source.configure
  mode: edit
  fields:
    - kind
    - { name: url, visible_if: { kind: [http, https] } }
    - { name: token, secret: true, visible_if: { kind: https } }
```

A secret is masked and never kept; left blank, the saved one stays. The
command must keep it out of its record — `unrecorded: [token]` on its
`commands:` entry — or the form won't open. For several forms on one
page, make a document whose parts are form lenses; each submits on its
own.
An action runs as the lens acting for you, so it can't do anything you
couldn't. Copy and Add to Agent Context are on every lens; you don't
declare them.

A lens action never approves a permission. A source that executes a
program needs its extension to declare `process`, which you approve at
deploy.

### Documents and dashboards

A `document` lens is a wiki-style page of sections. A `dashboard` lens is
a responsive grid of titled cards. Neither needs a `query`. Each part is
another lens, or prose.

```yaml
# lenses/overview.yaml
title: Project overview
params:
  - { name: stream_id, label: Stream }
viz: document
sections:
  - id: intro
    markdown: What needs attention in this stream.
  - lens: waiting-on-me
  - lens: review/recent-commits
    title: Latest commits
```

```yaml
# lenses/health.yaml
title: Health
params:
  - { name: stream_id, label: Stream }
viz: dashboard
tiles:
  - lens: waiting-on-me
  - lens: token-trend
    size: wide
```

- Each part has exactly one of `lens` (a slug in this extension, or
  `<ext>/<slug>`) and `markdown`.
- `id` defaults to the lens slug. A markdown part needs an `id`. Ids
  are unique in the lens, so give one of two copies of a lens an `id`.
- `title` defaults to the lens's title.
- Each lens part gets the params it declares from the page's params.
- A document folds sections with no rows into one line: "Nothing found:
  ...".
- A tile has a `size`: `small` (default), `wide`, `tall` or `full`. A
  markdown tile defaults to `full`. A section takes no `size`.
- A document has `sections`, not `tiles`. A dashboard has `tiles`, not
  `sections`.
- A lens your agent shows as an answer can't be a document or a
  dashboard.

Another extension can add to your document or dashboard. It mounts a lens
in a slot named `<extension>/<slug>.sections` (document) or
`<extension>/<slug>.tiles` (dashboard):

```yaml
# the "audit" extension's extension.yaml
ui:
  slots:
    - slot: project/overview.sections
      lens: stale-reviews
      at: { after: intro }
      title: Stale reviews
```

- `at` is `start`, `end` (default), `{ after: <part id> }` or
  `{ before: <part id> }`. If the part id does not exist, you get a load
  warning and the mount goes at the end.
- The target's params are the slot's params. The mounted lens must
  declare at least one of them (when the target has any).
- The added part shows the name of the extension that added it.
- `size` works only on a `.tiles` slot.

### Grouping and styling rows

A list or table can put its rows under headings, one per value of a
column, in the order each value first appears:

```yaml
viz: list
group: { by: bucket, link: { kind: page, from: bucket_page } }   # link is optional
emphasis: is_current     # a true / non-zero value highlights the row
depth: level             # a whole number indents the row
columns:
  - { key: title, link: { kind: page, from: ref }, icon: state, tone: hue }
actions:
  - { id: add, label: "+", command: oxplow.work_item.create, group: Ready, input: { title: New task } }
```

The `group`, `emphasis` and `depth` columns aren't shown as cells. An
action with `group:` is a button in that group's heading. `icon` names a
column whose value picks an icon: `ready`, `todo`, `in_progress`,
`blocked`, `done`, `canceled`, `archived`, `epic`, `task`, `wiki`,
`file`, `folder`, `commit`, `diff`, `lens`, `metric`, `dashboard`,
`comment`, or a ref-kind icon such as `bug`. `tone` names a column whose
value colours the cell: `accent`, `success`, `warning`, `danger` or
`muted`. Values outside those lists are ignored.

Drag a row that links somewhere onto the agent terminal to add that page
to the agent's context.

### Alerts

A lens can say when it needs your attention:

```yaml
alert: { min_rows: 1, label: Waiting on you }   # any rows at all
# or
alert: { column: pct, below: 80, label: Coverage low }   # the first row's value
```

Make it a panel's badge and the panel shows the alert's count in the
left nav while it fires; the **Alerts** panel lists every firing badge,
and clicking one opens the lens. Agents see the same alert state when
they run the lens.

```yaml
panels:
  - { id: waiting, title: Waiting on You, scope: project, body: waiting-on-me, badge: waiting-on-me }
```

A panel can also name a `collapsed:` lens, shown as its summary while the
panel is collapsed, and a `count:` lens, whose number of rows is the
panel's count without raising an alert.

### Showing a lens on a core page

`slots:` in `extension.yaml` mounts a lens into a page. The lens gets the
page's id as a param, so it must declare it:

| Slot | Page | Param |
|---|---|---|
| `effort.review.details` | an effort's diff view | `effort_id`, `change_id` |
| `work_item.detail.body` | a work item's page, below its body | `ref` |
| `work_item.detail.sidebar` | a work item's page, its side rail | `ref` |
| `thread.plan.header` | a thread's plan, as a compact strip | `thread_id` |
| `vcs.commit.details` | a commit's page | `change_id` |
| `vcs.status.header` | Uncommitted Changes, above the files | `stream_id` |
| `vcs.status.details` | Uncommitted Changes, below the files | `change_id` |
| `vcs.history.sidebar` | Git history's side column | `stream_id` |
| `diff.file.header` | a file diff, above the editor | `path`, `left_revision`, `right_revision`, `stream_id` |
| `settings.<area>.sections` | A Settings area's page, as sections: `overview` (part `every`), `agents` (`agents`, `new-threads`, `prompt`), `lsp` (`servers`), `extensions` (`installed`), `data` (`data`), `integrations` (`capabilities`, `integrations`), `ai` (`ai`) | none |

A lens declares the params its slot passes (a Settings area's lens needs none). `at` works only on a parts slot such as `settings.agents.sections`, or a foundation page's own (`oxplow-foundation/new-session.sections`, part ids `start`, `earlier`, binding `thread_id`). A region slot refuses it. `change_id` points at
the page's change in `v_change`, with its analysis in `v_change_file`,
`v_change_function`, `v_change_import`, `v_change_co_change` and
`v_change_duplicate`.

```yaml
slots:
  - { slot: work_item.detail.sidebar, lens: item-tokens }
```

### Turning an extension off

Settings → Extensions → **Disable** turns an extension off for the
project, including the ones that ship with oxplow. Oxplow
ships the required `oxplow-foundation` extension and the optional
`oxplow-bundled` extension. The latter holds the Usage, Planning, Review, and
Quality lenses, Change Analysis, the effort's tests and coverage, the
review packet, and the token panels. With it off, oxplow is the plain workbench: the pages keep
their file lists and diffs, and the Metrics pages and dashboards still
work. Turning
it off in Settings → Extensions changes your own choice
(`disableExtensions` in `.oxplow/personal.yaml`). To turn it off for your
team, remove it from `extensions:` in `.oxplow/project.yaml` and commit that.

Problems show under Settings → Extensions. Agents check their work with
`oxplow_validate_extension`.

## On the page

Every lens page has the same layout:

- **The title row** holds what acts on the whole page: Refresh, Copy,
  Add to Agent Context, Improve with Agent and the lens's own actions.
- **The body** is the content: the title, the description, then the
  sections, tables and charts.
- **The side panel** has three groups. **Filters** lists each param
  beside its value; click a value to change it. A control too wide for
  the panel, like a custom date range, opens over the page. Escape puts
  the value back. **Selection** shows the row you clicked, when a column
  `select`s a param. **About this page** names the extension and the
  file to edit.

oxplow warns when a page has too many filters, sections or columns, or
a param no query reads. The warnings show in `oxplow extension check`
and on the Extensions page, each with what to do instead.

- Right-click a row → **Add Row to Agent Context**.
- **Improve with Agent** puts `[oxplow lens <id>]` in the agent's input so you
  can ask for changes.
- **Pin to Dashboard** adds it as a [dashboard](dashboards.md) tile.
- The agent can see what you're looking at: `oxplow_get_open_page` returns the lens
  with the exact rows on your screen.

## Custom components

When none of the built-in views fit, an extension can ship its own web
component and a `viz: custom` lens that renders it. It runs in a
sandboxed frame: scripts and forms, no storage, no access to the app,
and no way to send data out — it can't fetch or open a socket, and a
form submits nowhere: handle `submit` in your script and run a command
with `invoke`.
If it navigates itself away, the frame can only go to this machine, and
the app ends the component and shows its table instead. It can only ask
for the lenses and commands it declares.

```yaml
# extension.yaml
custom_components:
  - id: burndown
    assets: [open-tasks]                # lenses it may query
    commands: [oxplow.work_item.transition]    # commands it may run
```

```yaml
# lenses/burndown.yaml
title: Burndown
query: SELECT day, remaining FROM v_my_ext_burndown
viz: custom
custom: { component: burndown, props: { color: accent } }
```

The bundle lives in `components/burndown/` and needs an `index.html`.
It loads oxplow's client library, then its own script, and can link
oxplow's kit stylesheet:

```html
<!doctype html>
<link rel="stylesheet" href="/component-lib/oxplow-kit.css">
<div id="out" class="ox-muted"></div>
<script src="/component-lib/oxplow-component.js"></script>
<script src="app.js"></script>
```

```js
// app.js
oxplow.connect().then((component) => {
  component.applyTheme();                // the theme the kit's classes read
  const render = (run) => {
    document.getElementById("out").textContent = `${run.result.rows.length} days`;
  };
  render(component.run);                 // the lens's own rows
  component.onUpdate(render);            // the lens re-ran
});
// Elsewhere, with `component` in hand:
//   await component.query("open-tasks", {})
//   await component.invoke("oxplow.work_item.transition", { ref, to: "done" })
//   await component.navigate("work_item:oxplow:tsk42")   // oxplow pages only
```

`oxplow extension new component <name>` writes all of this for you.

`component` also carries `props` and the theme's CSS variables
(`tokens`). The kit stylesheet has a few classes in oxplow's look:
`ox-muted`, `ox-small`, `ox-mono`, `ox-heading`, `ox-link`, `ox-card`,
`ox-badge`, `ox-button` (add `ox-primary` for the one main action),
`ox-table`, `ox-num`, forms (`ox-form` a stack of `ox-field`s, each an
`ox-label` over an `ox-input`, `ox-select` or `ox-textarea`, with an
`ox-error` under a field that is wrong), drag and drop (`ox-drag` for what
can be picked up, plus `ox-dragging` while it is; `ox-drop-target` for
where it may land), the states `ox-ok`, `ox-waiting`, `ox-running` and
`ox-bad`, and chart series `ox-series-1` to `ox-series-8` (a `fill` for
SVG, a `background` otherwise). They only work after `applyTheme()`, which also sets the app's `color-scheme` so the frame stays transparent. `invoke`
resolves with the command's result; a command your extension writes as
a script answers `{ result, children }` — its script's own `result`, and
the commands it composed. A
request that fails rejects with `{ code, message, field? }` — `field` is
the input a refused command names (`/title`), so your form can show the
message under it. Types for the
library are served beside it (`/component-lib/oxplow-component.d.ts`).

The frame refuses some things silently, so `oxplow extension check` reports
them as errors: an inline `<script>` or `onclick=` handler (put the code
in a `.js` file), a `type="module"` script (the scripts must be plain
scripts, since a sandboxed frame can't load modules), a script or
stylesheet from outside the bundle, and an `index.html` that doesn't
load the client library.

If the component doesn't connect within 3 seconds, or navigates itself
somewhere else, oxplow shows the lens's table instead. Agents always
read the table.

A component that declares `commands` outside its extension's namespace
runs them with your rights, so the extension declares `permissions:
{ commands: [...] }`, which you approve at deploy. The loader refuses a
component that needs a command the extension doesn't declare. A component
with no such `commands` needs no permission.

A command that needs confirmation is confirmed by you in oxplow, not
inside the frame.

### A custom page

A component can also be a whole page: name it in `pages:` instead of a
lens. It fills the page under oxplow's own bar, which still draws the
title, back and forward, bookmarks and the page's actions.

```yaml
# extension.yaml
permissions: { sql: read }              # it runs its own SQL
custom_components:
  - { id: timeline, bundle: dist/timeline, sql: read }
pages:
  - { id: timeline, title: Timeline, icon: history, category: Activity, params: [from, to], component: timeline }
```

A page's component has no lens run (`component.run` is null); instead
`component.page` carries the page's `params`, `streamId` and
`threadId`. It reads with its own read-only SQL, titles the page, puts
actions in the bar and opens a command's form in oxplow:

```js
oxplow.connect().then(async (component) => {
  component.applyTheme();
  const { rows } = await component.querySql(
    "SELECT ref, title FROM v_effort WHERE started_at >= ?1", [component.viewParams.from]);
  await component.setTitle(`Efforts since ${component.viewParams.from}`);
  await component.setActions([{ id: "refresh", label: "Refresh", primary: true }]);
  component.onAction((id) => { if (id === "refresh") location.reload(); });
  // Its view params live in its tab, so Back and a bookmark keep them:
  //   await component.setViewParams({ from: "2026-10-01" });
  //   component.onViewParams((params) => draw(params));
  // This step's own memory (scroll, what's open), back with Back:
  //   await component.setHistoryState({ scroll: 120 });
  // Its own memory for you, in every tab:
  //   await component.pageMemory.set("columns", ["title", "state"]);
  // A form oxplow shows; resolves with the result, or rejects CANCELLED:
  //   await component.openForm("oxplow.work_item.create", { title: "" })
});
```

`querySql` needs `sql: read` on the component and `permissions: { sql:
read }` on the extension; it reads oxplow's models only, never writes.
`setTitle`, `setActions` and `openForm` are a page's: a component a lens
shows can't use them. `viewParams`, `setViewParams` and `historyState`
are its host's view, where the host shares one: a page's component has
all of its page's params; a replacement has the keys its view's contract
lists (`v_replaceable.view`); a `viz: custom` lens on its own page has
the lens's params. In a dashboard tile or a slot `viewParams` is null and
the component keeps its filters itself. Each kind of state has a name that says how far
it reaches: **view params** (the tab's address: what the page shows, kept
by Back, a bookmark and a reopened tab, seen by an agent), **history
state** (this step of the tab: scroll, what's expanded) and **page
memory** (yours, in every tab of the component; JSON, 64 KB a value and
1 MB a component). Never browser storage: a component's frame has none.
`oxplow extension check` warns when a component's script uses
`localStorage` or `sessionStorage`. A component written in
TypeScript or with a library declares a `build:` ("Extensions" → "Builds"),
so only its sources are committed. oxplow-foundation's Timeline page is a
working example (`extensions/oxplow-foundation/src/timeline/`): it reads
its window from `from` and `to`, writes every change of it back with
`setViewParams`, and follows `onViewParams` when Back or Forward moves the
tab. A default it computes ("the last week") stays out of the address, so
a bookmark doesn't freeze it to one date.

## Replacing a view

An extension can put its own lens where one of oxplow's views is: the
Board's cards (`work_item.board`), the Tasks list (`work_item.list`) or a
task's editor (`work_item.detail`). It needs no work-items provider of its
own: a view reads `v_work_item` and changes items through the work-item
commands (`oxplow.work_item.update`, `transition`, `reorder`, `move`), so it
works with whichever list is active.

```yaml
# extension.yaml
ui:
  replacements:
    - { target: work_item.board, lens: swimlanes, version: 1 }
```

The lens gets the view's props as params and must declare all of them: the
Board gets `scope` (`thread`, `backlog` or `all`) and `thread_id`; the list
gets `scope` (`thread`, `stream` or `all`) and `thread_id`, and its lens may
declare `stream_id`; the editor gets `ref`. Settings → Integrations lists
each replaced view with a choice of whose to show (Default, oxplow's own, or
an extension); a small "replaced by" badge says whose it is, and if it can't
load, oxplow's own shows with a line saying why. Some views also share
their page's view params: Git History's graph (`vcs.history.graph`) has
the page's `q`, `author` and `branch`, so a custom graph reads them from
`component.viewParams`, writes them with `setViewParams` and follows
`onViewParams` — the same address oxplow's own list keeps. A `viz: custom` view that
changes items needs its commands declared like any component, and the
extension must declare the permissions for them.

[`examples/extensions/task-views`](../../examples/extensions/task-views) is a
working set of all three: swimlanes you drag cards between, a compact list
with row actions, and a plain editor form. Copy it and change it.

## Sharing

- **Deploy:** an extension's files are its source. Nothing runs a source.
  Run `oxplow.extension.deploy { name }` (or `oxplow extension deploy
  <name>`) to make your edits live.
- **Your team:** a new extension is personal; `.oxplow/.gitignore` keeps it
  out of git. `oxplow.extension.share { name }` un-ignores it and lists it
  in `.oxplow/project.yaml`. Then you commit it. `oxplow.extension.unshare`
  reverses this.
- **Someone else's extension:** clone it into `.oxplow/extensions/<name>/`,
  list it, and deploy it.

## Collectors: bringing in outside data

An extension can declare a **collector**: a script that fetches records from
somewhere else (GitHub, a team's tracker, your CI) and prints them as JSON. Oxplow
stores them as a view like `v_github_pr` that lenses can join with tasks and
efforts.

```yaml
collectors:
  - id: prs
    runtime: exec
    entry: sync.sh               # inside the extension folder
    trigger: { every: 15m }      # or: manual
    env: [GITHUB_REPOSITORY]     # passed through from your environment
    credentials: [GITHUB_TOKEN]  # secrets from your keychain
    network: [api.github.com]    # the only host it may reach
    entities:
      - name: pr
        key: number
        columns: { number: int, title: text, state: text, merged_at: time }
```

The script prints:

```json
{"entities": {"pr": [{"number": 12, "title": "Fix login", "state": "open", "merged_at": null}]}}
```

Column types are `text`, `int`, `real`, `bool` and `time` (an ISO timestamp).

A collector runs code, so its extension must declare `permissions: { process:
true }`, plus `network` and `secrets` for what it reaches. You approve them
when you deploy the extension (Settings → Extensions). A teammate who pulls
it deploys it themselves. A changed script needs a redeploy, which asks again
only if it adds a permission. An agent's deploy becomes a proposal you decide.
The first sync is just **Sync Now** in Settings → Data.

`process` is declared, not confined: the program runs with your authority.

Known limits:

- It only gets the environment variables it declares, plus `PATH` and `HOME`.
- It can only reach the hosts it lists under `network:` (for example
  `network: [api.github.com]`). The list is the extension's `network`
  permission, so a collector that adds a host asks again at deploy. On macOS
  this is enforced, and anything else the script tries to reach fails. Other
  systems don't enforce it yet.
- Secrets go in `credentials:`. Set each one under the collector in Settings →
  Extensions; the value goes to your OS keychain and the script gets it as
  that environment variable. Values are per extension, and agents can't
  read or set them.
- Collectors run from the primary stream's worktree. Their data is shared by
  every stream.

### Collectors over your own data

A collector can also compute new data from data oxplow already has, with a
Starlark or jq script instead of a program. It gets the rows of a SQL query
and returns entities in the same shape:

```yaml
collectors:
  - id: hot
    runtime: starlark            # or: jaq
    entry: collectors/hot.star
    input: "SELECT ref, title FROM v_work_item WHERE json_extract(native, '$.priority') = 'high'"
    entities:
      - { name: hot_item, key: ref, columns: { ref: text, title: text } }
```

```python
def transform(input):
    return {"entities": {"hot_item": input["rows"]}}
```

Pure scripts cannot directly reach the network, files, environment, or
keychain, so they need no permission. A collector that calls
`ai.complete` or `ai.decide` also needs `permissions: { ai: true }`, an
AI provider, and a configured role. Those model calls can send data
outside the project and use your provider account.

### Syncing only what changed

By default each run replaces a source's data. With `sync: upsert`, a run
returns only new and changed rows, plus the keys it removed:

```json
{"entities": {"pr": [{"number": 12, "title": "Fix login"}]}, "deleted": {"pr": [7]}}
```

Anything the run doesn't mention stays as it was.

### Saying that something happened

Rows tell you what is true now. To record that something *happened* (a
pull request merged), an extension declares an event type and its
collector returns events of it:

```yaml
event_types:
  types:
    - type: my_gh.merged         # <extension name with _>.<name>
      v: 1
      schema: event_types/merged.v1.json   # the payload's JSON Schema
      summary: A pull request merged.
```

```json
{"entities": {"pr": [...]}, "events": [{"type": "my_gh.merged", "payload": {"number": 12}}]}
```

The events land in the event log with the run that produced them, and you
can read them in `v_event`. Limits:

- A collector can only emit types its own extension declares.
- At most 100 events a run.
- It can't emit a type it also runs on (`trigger: { on: [...] }`).
- Once a `type` at a version has been recorded, its schema is fixed. A new
  shape is a new `v` with an `upcast` script.

A complete example, which pulls this repo's pull requests and a lens that
matches them to tasks, is in
[`examples/extensions/github`](https://github.com/nvoxland/oxplow/tree/main/examples/extensions/github).
