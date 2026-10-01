# Lenses

A lens is a page you (or your agent) build: a SQL query over oxplow's data
plus how to show it. It lives as a small YAML file in your repo, shows up in
the launcher (++cmd+p++, under **Lenses**), and re-runs as the data changes.

The quickest way to get one is to ask your agent: "make me a lens of the
tasks blocked in this stream". It knows the format.

## The data

Everything oxplow tracks is exposed as read-only SQL views: `v_task`,
`v_effort`, `v_thread`, `v_comment`, `v_wiki_page`, `v_effort_file`,
`v_agent_turn`, `v_token_usage`, `v_fact`, and more.

**Explore Data** (launcher → Data) lists every view with a description of each
column. Pick one to see sample rows, edit the SQL (++cmd+enter++ runs it), and
**Save as Lens** when it shows what you want.

## Writing one by hand

Lenses live in `oxplow/extensions/<extension>/lenses/<slug>.yaml`. The
extension folder needs an `extension.yaml`:

```yaml
# oxplow/extensions/review/extension.yaml
name: review          # must match the folder name
description: Lenses for reviewing agent work
```

```yaml
# oxplow/extensions/review/lenses/blocked.yaml
title: Blocked Tasks
params:
  - { name: stream_id, label: Stream }
query: |
  SELECT id, title, updated_at FROM v_task
  WHERE status = 'blocked' AND stream_id = :stream_id
viz: table            # table | list | number | markdown | bar | line | treemap | grid
columns:
  - { key: title, label: Task, link: { kind: task, from: id } }
  - { key: updated_at, label: Updated }
empty: Nothing is blocked.
```

- `params` become inputs on the page and bind as `:name`.
- A param named `stream_id` or `thread_id` is filled in with the stream
  you're on and its selected thread, so "my stream" needs no hardcoded
  id. Type a value to look at another one.
- `link.kind` is `task`, `file` (add `line: <column>` to open at a line),
  `wiki`, `effort-diff`, `commit`, `metric`, `diff-at` or `compare`.
  - `diff-at` opens a file's diff within a change:
    `{ kind: diff-at, line: start_line, base: base_revision, head: head_revision }`
    (join `v_change` for the two revisions).
  - `compare` opens two line ranges side by side; the column holds
    `path:start-end|peer:start-end`, and `head:` names a revision column.
  - A revision is `working` (the files on disk), `snap:<id>` (a
    local-history snapshot) or `git:<rev>` (a sha, branch or `HEAD`).
- Unknown keys are errors, so typos show up instead of being ignored.

Charts name the columns they draw:

```yaml
viz: bar              # or line
chart: { x: day, y: visits }          # line also takes series: <column>
```

```yaml
viz: treemap
chart: { label: path, size: churn, group: zone }
```

```yaml
viz: grid             # other lenses, stacked
children: [visits, tokens-by-day]
```

`launcher: { category: Activity }` files a lens under that launcher
heading; `hidden: true` leaves it out (for lenses only a slot shows).
`actions:` runs commands. Each action names a command and its input,
and becomes a button above the result, or, with `row: true`, an item in
each row's menu (right-click, or Menu key / Shift+F10 on a row):

```yaml
actions:
  - id: finish
    label: Finish
    command: work_item.transition
    input: { ref: "{{row.ref}}", to: done }
    row: true
  - id: sync
    label: Sync PRs
    command: source.sync
    input: { extension: github, source: prs }
```

`{{param.x}}` and `{{row.col}}` fill the input from the lens's params or
the row. A command that asks for confirmation asks you before it runs.
An action runs as the lens acting for you, so it can't do anything you
couldn't. Copy and Add to Agent Context are on every lens; you don't
declare them.

A lens action never approves a source. The first run of a source that
executes a program is still approved in Settings → Data.

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

### Showing a lens on a core page

`slots:` in `extension.yaml` mounts a lens into a page. The lens gets the
page's id as a param, so it must declare it:

| Slot | Page | Param |
|---|---|---|
| `effort-review` | an effort's diff view | `effort_id`, `change_id` |
| `task-detail` | a task's page | `task_id` |
| `thread` | the Work panel (compact) | `thread_id` |
| `commit` | a commit's page | `change_id` |
| `uncommitted` | Uncommitted Changes | `change_id` |
| `settings` | Settings, in a section named after the extension | none |

A lens declares the params its slot passes (a `settings` lens needs none). `change_id` points at
the page's change in `v_change`, with its analysis in `v_change_file`,
`v_change_function`, `v_change_import`, `v_change_co_change` and
`v_change_duplicate`.

```yaml
slots:
  - { slot: task-detail, lens: task-tokens }
```

### Turning an extension off

Settings → Extensions → **Disable** turns an extension off for the
project, including the ones that ship with oxplow. `oxplow-analytics`
is one of those: it holds the Usage, Planning, Review, and Quality
lenses, Change Analysis, the effort's tests and coverage, and the token
panels. With it off, oxplow is the plain workbench: the pages keep
their file lists and diffs, and the Metrics pages and dashboards still
work. It writes
`extensions: { disabled: [name] }` to `.oxplow/project.yaml`; commit that
to turn it off for your team.

Problems show under Settings → Extensions. Agents check their work with
`validate_extension`.

## On the page

- Right-click a row → **Add Row to Agent Context**.
- **Improve with Agent** puts `[oxplow lens <id>]` in the agent's input so you
  can ask for changes.
- **Pin to Dashboard** adds it as a [dashboard](dashboards.md) tile.
- The agent can see what you're looking at: `get_open_page` returns the lens
  with the exact rows on your screen.

## Custom components (experimental)

When none of the built-in views fit, a private extension can ship its own
web component and a `viz: custom` lens that renders it. It runs in a
sandboxed frame: scripts only, no storage, no access to the app, and no
way to send data out — it can't fetch, open a socket or submit a form.
If it navigates itself away, the frame can only go to this machine, and
the app ends the component and shows its table instead. It can only ask
for the lenses and commands it declares.

```yaml
# extension.yaml (sharing: private)
custom_components:
  - id: burndown
    assets: [open-tasks]                # lenses it may query
    commands: [work_item.transition]    # commands it may run
```

```yaml
# lenses/burndown.yaml
title: Burndown
query: SELECT day, remaining FROM v_my_ext_burndown
viz: custom
custom: { component: burndown, props: { color: accent } }
```

The bundle lives in `components/burndown/` and needs an `index.html`.
oxplow hands it a `MessagePort` in an `init` message; everything goes
over that port:

```html
<!doctype html>
<div id="out"></div>
<script src="app.js"></script>
```

```js
// app.js
let port, next = 0;
const pending = new Map();
const call = (msg) => new Promise((resolve, reject) => {
  const id = String(next++);
  pending.set(id, { resolve, reject });
  port.postMessage({ id, ...msg });
});
window.addEventListener("message", (e) => {
  if (e.data?.type !== "init") return;
  port = e.ports[0];
  port.onmessage = ({ data }) => {
    if (data.type === "update") return render(data.run);
    const p = pending.get(data.id);
    pending.delete(data.id);
    data.ok ? p.resolve(data.result) : p.reject(data.error);
  };
  render(e.data.run);              // the lens's own rows
  port.postMessage({ type: "ready" });
});
function render(run) {
  document.getElementById("out").textContent = `${run.result.rows.length} days`;
}
// Elsewhere:
//   await call({ method: "query", asset: "open-tasks", params: {} })
//   await call({ method: "invoke", command: "work_item.transition", input: { ref, to: "done" } })
//   await call({ method: "navigate", ref: "work_item:oxplow:tsk42" })
```

`init` also carries `props`, the theme's CSS variables (`tokens`) and
`kitCss`, a small stylesheet built from them. If the component doesn't
say `ready` within 3 seconds, or navigates itself somewhere else, oxplow
shows the lens's table instead. Agents always read the table.

A command that needs confirmation is confirmed by you in oxplow, not
inside the frame. There's no client library yet; the snippet above is
the whole protocol.

## Sharing

- **Your team:** `oxplow/extensions/` is ordinary project files. Commit it.
- **Anyone:** put an extension in its own git repo (with `extension.yaml` at
  the root) and install it from Settings → Extensions → paste the git URL.
  **Update** pulls the latest. Don't edit installed extensions in place;
  updates overwrite them.

## Sources: bringing in outside data

An extension can declare a **source**: a script that fetches records from
somewhere else (GitHub, Linear, your CI) and prints them as JSON. Oxplow
stores them as a view like `v_github_pr` that lenses can join with tasks and
efforts.

```yaml
sources:
  - id: prs
    runtime: exec
    entry: sync.sh               # inside the extension folder
    schedule: every 15m          # or: manual
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

A source runs code, so **nothing runs until you approve it** in Settings →
Data → **Approve & Run**. Approval is per machine and per version of the
script: a teammate who pulls it approves it themselves, and a changed script
needs approving again. Agents can run approved sources but can't approve
them.

Known limits:

- It only gets the environment variables it declares, plus `PATH` and `HOME`.
- It can only reach the hosts it lists under `network:` (for example
  `network: [api.github.com]`). The list is part of what you approve, so a
  source that adds a host needs approving again. On macOS this is
  enforced, and anything else the script tries to reach fails. Other
  systems don't enforce it yet, and the approval says so.
- Secrets go in `credentials:`. Set each one under the source in Settings →
  Extensions; the value goes to your OS keychain and the script gets it as
  that environment variable. Values are per extension, and agents can't
  read or set them.
- Sources run from the primary stream's worktree. Their data is shared by
  every stream.

### Sources over your own data

A source can also compute new data from data oxplow already has, with a
Starlark or jq script instead of a program. It gets the rows of a SQL query
and returns entities in the same shape:

```yaml
sources:
  - id: hot
    runtime: starlark            # or: jaq
    entry: sources/hot.star
    input: "SELECT id, title FROM v_task WHERE priority = 'high'"
    entities:
      - { name: hot_task, key: id, columns: { id: int, title: text } }
```

```python
def transform(input):
    return {"entities": {"hot_task": input["rows"]}}
```

These scripts can't reach the network, files, environment or keychain, so
they run without approval.

### Syncing only what changed

By default each run replaces a source's data. With `sync: upsert`, a run
returns only new and changed rows, plus the keys it removed:

```json
{"entities": {"pr": [{"number": 12, "title": "Fix login"}]}, "deleted": {"pr": [7]}}
```

Anything the run doesn't mention stays as it was.

A complete example, which pulls this repo's pull requests and a lens that
matches them to tasks, is in
[`examples/extensions/github`](https://github.com/nvoxland/oxplow/tree/main/examples/extensions/github).
