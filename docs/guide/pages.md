# Pages

A page is a view you (or your agent) build: named SQL queries over oxplow's
data plus a tree of tables, charts and forms that shows them. It lives as a
small YAML file in your repo, shows up in the launcher (++cmd+p++), and
re-runs as the data changes. A dashboard, a settings screen and a commit
view are all pages.

The quickest way to get one is to ask your agent: "make me a page of the
tasks blocked in this stream". It knows the format. For a one-off answer
it shows a page right beside the conversation (nothing is stored) and you
can **Save to wiki** or **Save as Page** to keep it.

## The data

Everything oxplow tracks is exposed as read-only SQL views: `v_work_item`
(the active work list's items, whichever tracker holds them), `v_effort`,
`v_thread`, `v_comment`, `v_wiki_page`, `v_effort_file`, `v_agent_turn`,
`v_token_usage`, `v_fact`, and more.

**Explore Data** (launcher → Data) lists every view in its rail. Pick one to
see its columns and sample rows, edit the SQL (++cmd+enter++ runs it), and
**Save as Page** when it shows what you want. It is itself an extension's
custom page: oxplow-foundation's.

## Writing one by hand

Pages live in `.oxplow/extensions/<extension>/pages/<slug>.yaml`. The
extension folder needs an `extension.yaml`:

```yaml
# .oxplow/extensions/review/extension.yaml
manifest: 2
name: review          # must match the folder name
description: Pages for reviewing agent work
intent:
  purpose: Show what's blocked, so it gets unblocked
  examples:
    - { name: blocked, input: { page: blocked } }
```

`intent` is required: what the extension is for, and examples that
`oxplow extension test` runs. `oxplow extension new extension review` writes
this file for you. The [Extensions reference](../reference/extensions.md)
covers everything else an extension can declare.

```yaml
# .oxplow/extensions/review/pages/blocked.yaml
title: Blocked Work
params:
  - { name: thread_id, label: Thread }
answers:
  - { question: "What is blocked?" }
data:
  blocked:
    sql: |
      SELECT uri, title, updated_at FROM v_work_item
      WHERE state = 'blocked' AND thread_id = :thread_id
view:
  viz: table            # table | list | tree | number | markdown | bar | line | treemap | board | custom | ...
  from: blocked
  columns:
    - { key: title, label: Item, link: { kind: page, from: uri } }
    - { key: updated_at, label: Updated, format: relative, width: 120 }
  empty: Nothing is blocked.
```

A page has three parts: its **metadata** (title, params, `answers:`), its
**data** (named queries, called loaders, that run in order; a loader reads
only the ones before it, and an earlier loader reads as a table of its
name), and its **view** (a tree of nodes: `stack`, `grid`, `switch`, `viz`,
`embed`, `slot`, `markdown`, `component`).

- `params` become inputs on the page and bind as `:name`. A param named
  `stream_id` or `thread_id` is filled in with the stream you're on and its
  selected thread, so "my stream" needs no hardcoded id.
- A loader is `sql`, a read-only `command`, another page's loader (`page`),
  or inline `rows`.
- `answers:` are the questions the page answers. Your agent finds pages by
  them (`oxplow_find_page`), and `oxplow extension test` opens the page with
  each answer's params.
- Unknown keys are errors, so a typo fails loudly. `oxplow extension check`
  reports each problem as `file:line: what, and how to fix it`.

### Typed params

A param has a `type`, which picks its control: `text` (the default),
`choice` (`options: [{ value, label }]`, or `options_from:` a page whose rows
are the choices), `bool`, `search` (applies as you type), `branch`, and
`date_range` (a preset `1d 2d 3d 7d 30d 90d all`, or `<from>..<to>`; it
gives the query `:range_from` and `:range_to`).

```yaml
params:
  - { name: range, label: When, type: date_range, default: 30d, presets: [7d, 30d, 90d, all] }
  - { name: q, label: Search subjects, type: search }
  - { name: merges, label: Merges only, type: bool, default: false }
```

### Where a param lives in the address

A param's `role` says where its value rides in the page's address. The
default, `resource`, names what the page shows (the `?` part; a change
opens another resource). `role: filter` marks one that only narrows or
re-views it (a search, a range): it rides in the `#` part and a change
replaces the entry. A row you click that a column `select`s is the page's
selection (also in the `#`).

### Columns and links

A table or list takes `columns:` for headers, order and links. `format`
(`date`, `datetime`, `relative`, `bytes`, `duration`, `integer`, `percent`,
`sparkline`, `badge`) changes only what you see. `link: { kind, from }`
makes a cell clickable: `page` (any oxplow URI, such as a work item's),
`file` (with `line:`), `wiki`, `commit`, `metric`. `select: <param>` makes
a click set that param, so a detail section that reads it re-runs: master and
detail with no code.

### Grouping and styling rows

`group: { by: <column> }` puts rows under a heading per value;
`emphasis: <column>` highlights rows whose value is truthy; `depth:
<column>` indents. A column's `icon: <column>` draws an icon named by that
column's value, and `tone: <column>` colours the cell (`accent`, `success`,
`warning`, `danger`, `muted`).

### Actions

`actions:` on a page or a viz node are commands it offers: `{ id, label,
command, input }`. `"{{param.x}}"` binds a param and, on a row action
(`row: true`), `"{{row.col}}"` binds that row's column. An action runs as
the page acting for whoever pressed it, so it can't do anything you
couldn't.

### Alerts

`alert:` makes a page raise an alert when it needs attention: `{ min_rows:
1 }`, or `{ column: pct, below: 80 }`.

### Embedding and mounting pages

An `embed` node shows another page inline, handing it params by name. A
`slot` node is a place other extensions mount their pages at. To mount yours
into a foundation page, list it under `ui.slots` in `extension.yaml`:

```yaml
ui:
  slots:
    - { slot: oxplow-foundation/work-item.sections, page: item-tokens }
```

The mounted page gets the host's params by name, so it declares the ones it
needs:

| Slot | Page | Param |
|---|---|---|
| `oxplow-foundation/diff-view.sections` | an effort's or a range's diff | `effort_id`, `change_id` |
| `oxplow-foundation/work-item.sections` | a work item's page | `uri` |
| `oxplow-foundation/tasks.sections` | the Tasks page; `at: { before: list }` puts a strip above the list | `thread_id`, `stream_id` |
| `oxplow-foundation/commit.sections` | a commit's page | `change_id` (the page's row) |
| `oxplow-foundation/uncommitted-changes.sections` | Uncommitted Changes; `at: { before: commit }` puts a strip above everything | `change_id` (the page's row) |
| `oxplow-foundation/new-session.sections` | the New Session page (sections `start`, `earlier`) | `thread_id` |

`at` is `start`, `end` (the default) or `{ before | after: <sibling id> }`.
`change_id` points at the page's change in `v_change`, with its analysis in
`v_change_file`, `v_change_function`, `v_change_import`, `v_change_co_change`
and `v_change_duplicate`.

### Dashboards

A dashboard is a page whose view is a `grid` of tiles, in the Dashboards
category: see [Dashboards](dashboards.md).

### One-off pages in the wiki

A fenced ` ```oxplow-page ` block in a wiki page (or an agent's
`oxplow_show_page { spec }`) is an inline page: the same YAML, read-only (no
actions, forms or components), drawn live.

### Turning an extension off

Settings → Extensions → **Disable** turns an extension off for the
project, including the ones that ship with oxplow. Oxplow ships the required
`oxplow-foundation` extension and the optional `oxplow-bundled` extension.
The latter holds the Usage, Planning, Review, and Quality pages, Change
Analysis, the effort's tests and coverage, the review packet, and the token
panels. With it off, oxplow is the plain workbench: the pages keep their
file lists and diffs, and the Metrics pages and dashboards still work.
Turning it off in Settings → Extensions changes your own choice
(`disableExtensions` in `.oxplow/personal.yaml`). To turn it off for your
team, remove it from `extensions:` in `.oxplow/project.yaml` and commit that.

Problems show under Settings → Extensions. Agents check their work with
`oxplow_validate_extension`.

## On the page

Every page has the same layout:

- **The title row** holds what acts on the whole page: Copy, the page's own
  actions and, if the page says `refresh: true`, Refresh. The **☰** menu has
  the rest, including **Show Definition**: the page's YAML and where each
  part of it comes from.
- **The body** is the content: the title, the description, then the nodes.
- **The side panel** has three groups. **Filters** lists each param beside
  its value; click a value to change it. **Selection** shows the row you
  clicked, when a column `select`s a param. **About this page** names the
  extension and the file to edit.

oxplow warns when a page has too many filters, sections or columns, or a
param no query reads. The warnings show in `oxplow extension check` and on
the Extensions page, each with what to do instead.

- Right-click a row → **Add Row to Agent Context**.
- **Pin to Dashboard** adds the page as a [dashboard](dashboards.md) tile.
- The agent can see what you're looking at: `oxplow_get_open_page` returns the
  page's tree (what it shows), your filters and picks, and the focused node.

## Custom components

When none of the built-in views fit, an extension can ship its own web
component and a `component` node that shows it. It runs in a
sandboxed frame: scripts and forms, no storage, no access to the app,
and no way to send data out — it can't fetch or open a socket, and a
form submits nowhere: handle `submit` in your script and run a command
with `invoke`.
If it navigates itself away, the frame can only go to this machine, and
the app ends the component and shows its table instead. It can only run
the commands it declares, and it is fed its page's data.

```yaml
# extension.yaml
custom_components:
  - id: burndown
    commands: [oxplow.work_item.transition]    # commands it may run
```

```yaml
# pages/burndown.yaml
title: Burndown
data:
  burndown:
    sql: SELECT day, remaining FROM v_my_ext_burndown
view: { component: { id: burndown, props: { color: accent } } }
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
  const render = (data) => {
    document.getElementById("out").textContent = `${data.days.rows.length} days`;
  };
  render(component.data);                // each loader's rows, by name
  component.onUpdate(({ data }) => render(data)); // a loader changed
});
// Elsewhere, with `component` in hand:
//   await component.invoke("oxplow.work_item.transition", { uri, to: "done" })
//   await component.navigate("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")   // oxplow pages only
//   await component.navigate(uri, { newTab: true })    // open it in a new tab
//   await component.navigate(uri, { replace: true })   // take the place of this step in Back
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
somewhere else, oxplow shows the page's table instead. Agents always
read the table.

A component that declares `commands` outside its extension's namespace
runs them with your rights, so the extension declares `permissions:
{ commands: [...] }`, which you approve at deploy. The loader refuses a
component that needs a command the extension doesn't declare. A component
with no such `commands` needs no permission.

A command that needs confirmation is confirmed by you in oxplow, not
inside the frame.

### A custom page

A component can also be a whole page: a page whose view is one `component`
node. It fills the page under oxplow's own bar, which still draws the
title, back and forward, bookmarks and the page's actions.

```yaml
# extension.yaml
permissions: { sql: read }              # it runs its own SQL
custom_components:
  - { id: timeline, bundle: dist/timeline, sql: read }
# pages/timeline.yaml
title: Timeline
icon: history
category: Activity
params: [{ name: from }, { name: to }]
view: { component: { id: timeline } }
```

A whole-page component's `component.page` carries the page's `params`, `streamId` and
`threadId`. It reads with its own read-only SQL, titles the page, puts
actions in the bar and opens a command's form in oxplow:

```js
oxplow.connect().then(async (component) => {
  component.applyTheme();
  const { rows } = await component.querySql(
    "SELECT uri, title FROM v_effort WHERE started_at >= ?1", [component.viewParams.from]);
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
`setTitle`, `setActions`, `openForm` and `showView` are a whole page's: a
component inside another page's layout can't use them. `showView(yaml)` hands oxplow an
inline page — `title`, read-only `sql` loaders under `data`, a `view`
(JSON is YAML too) — to draw under the page with its own components,
live, with Save as Page and Pin to Dashboard when it is one viz over one
`sql` loader. It resolves with its first loader's rows, and a loader that
fails rejects `INVALID` with `field: "/data/<loader>/sql"`. A component is
fed its page's data: `component.data` holds each loader's `{ columns,
rows, truncated }` by name, `component.params` what they ran with, and
`onUpdate(({ data, params, changed }) => …)` hears each change. While one shows, the frame takes its content's height:
leave `html` and `body` at `height: auto`. A page can also have a `rail:` node tree: oxplow
draws it in the page's rail, and a row it picks sets the page's view
param, which the component hears through `onViewParams`. Explore Data is
built this way. `viewParams`, `setViewParams` and `historyState`
are its host's view, where the host shares one: a page's component has
all of its page's params; a replacement has the keys its view's contract
lists (`v_replaceable.view`); a component on a page of its own has
the page's params. In a dashboard tile or a slot `viewParams` is null and
the component keeps its filters itself. Each kind of state has a name that says how far
it reaches: **view params** (the part of the address after `#`: how the page shows it, kept
by Back, a bookmark and a reopened tab, seen by an agent), **history
state** (this step of the tab: scroll, what's expanded) and **page
memory** (yours, in every tab of the component; JSON, 64 KB a value and
1 MB a component). Never browser storage: a component's frame has none.
`oxplow extension check` warns when a component's script uses
`localStorage` or `sessionStorage`. A component written in
TypeScript or with a library declares a `build:` ("Extensions" → "Builds"),
so only its sources are committed. oxplow-foundation's Timeline page is a
working example: it reads
its window from `from` and `to`, writes every change of it back with
`setViewParams`, and follows `onViewParams` when Back or Forward moves the
tab. A default it computes ("the last week") stays out of the address, so
a bookmark doesn't freeze it to one date.

## Replacing a view

An extension can put its own page where one of oxplow's views is: the
Board's cards (`work_item.board`), the Tasks list (`work_item.list`) or a
task's description editor (`work_item.detail`). It needs no work-items provider of its
own: a view reads `v_work_item` and changes items through the work-item
commands (`oxplow.work_item.update`, `transition`, `reorder`, `move`), so it
works with whichever list is active.

```yaml
# extension.yaml
ui:
  replacements:
    - { target: work_item.board, page: swimlanes, version: 1 }
```

The page gets the view's props as params and must declare all of them: the
Board gets `scope` (`thread`, `backlog` or `all`) and `thread_id`; the list
gets `scope` (`thread`, `stream` or `all`) and `thread_id`, and its page may
declare `stream_id`; the editor gets `uri`. Settings → Integrations lists
each replaced view with a choice of whose to show (Default, oxplow's own, or
an extension); a small "replaced by" badge says whose it is, and if it can't
load, oxplow's own shows with a line saying why. Some views also share
their page's view params: Git History's graph (`vcs.history.graph`) has
the page's `q`, `author` and `branch`, so a custom graph reads them from
`component.viewParams`, writes them with `setViewParams` and follows
`onViewParams` — the same address oxplow's own list keeps. A `viz: custom` view that
changes items needs its commands declared like any component, and the
extension must declare the permissions for them.

The `task-views` example that oxplow ships is a working set of all three:
swimlanes you drag cards between, a compact list with row actions, and a
plain editor form. Copy it with `oxplow extension new --from task-views` and
change it.

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
stores them as a view like `v_github_pr` that pages can join with tasks and
efforts.

```yaml
collectors:
  - id: prs
    runtime: exec
    entry: sync.sh               # inside the extension folder
    on: { every: 15m }      # or: manual
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
    input: "SELECT uri, title FROM v_work_item WHERE json_extract(native, '$.priority') = 'high'"
    entities:
      - { name: hot_item, key: uri, columns: { uri: text, title: text } }
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
- It can't emit a type it also runs on (`on: [...]`).
- Once a `type` at a version has been recorded, its schema is fixed. A new
  shape is a new `v` with an `upcast` script.

A complete example, which pulls this repo's pull requests and a page that
matches them to tasks, is the `github` example that oxplow ships (`oxplow extension new --from github`).
