---
name: oxplow-extension
description: Build oxplow lenses and extensions on request: custom pages over oxplow's data (tasks, efforts, threads, comments, wiki, snapshots, metrics facts) that the user sees in the app and can share with their team. Loads when the user asks to "make/build/show me a lens/page/view/list/report of X", "a place where I can see X", "track X across threads/efforts", "what's waiting on me", or asks to change an existing lens. Teaches the semantic layer (`describe_schema`, `query_sql` over `v_*` views), the `oxplow/extensions/<name>/` file format, and the validate → run_lens loop.
---

# Building oxplow lenses

A **lens** is a saved query over oxplow's data plus how to show it. The user
opens it as a page (Cmd+P → Lenses), and it updates as the work changes.
Lenses live in **extensions**: folders of plain files in the repo. The user
can use a lens themselves, share it with their team by committing it, or
publish the extension for anyone.

You build them with your normal file tools. `oxplow plugin new lens
<name> --origin <your effort ref>` scaffolds the folder with a v2 manifest,
an intent, one example and fixture, and a starter lens; then edit.

## 1. Find the data

- `describe_schema` lists every queryable view (`v_stream`, `v_thread`,
  `v_task`, `v_effort`, `v_comment`, `v_wiki_page`, `v_snapshot`,
  `v_measure`, `v_capture`, `v_fact`, and anything extensions add), with a
  doc for every column. **Read it first**; don't guess column names.
- `query_sql` runs one read-only `SELECT`/`WITH`. Iterate on the query
  here until the rows are what the user wants, before writing any files.
- Only query `v_*` views. The physical tables behind them aren't a stable
  contract.

## 2. Write the files

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
sharing: private        # private is the default; `shared` = for the team, stable kinds only + `engine`
intent:                 # required: what it is for, and how to know it works
  purpose: Show what is waiting on the reviewer in this stream
  origin: effort:eff42  # the effort you are working under, or null
  examples:
    - name: a stream with one blocked task
      input: { lens: waiting-on-me, params: { stream_id: 1 } }
      expect: one row for the blocked task
slot_mounts:            # optional: mount a lens into a core page
  - { slot: rail, lens: waiting-on-me }
```

A manifest without `manifest: 2` is read as the old v1 shape with a
warning; write v2 (`oxplow plugin migrate <name>` rewrites a v1 file in
place). Unknown keys are errors, and `check` reports every problem as
`file:line: what — fix`.

`lenses/<slug>.yaml`:

```yaml
title: Waiting on me
description: Blocked tasks and open follow-up comments in this stream.
params:                         # optional; bound as :name in the query
  - { name: stream_id, label: Stream }   # filled in: the viewer's stream
query: |
  SELECT id, title, status, thread_id
  FROM v_task
  WHERE status = 'blocked' AND stream_id = :stream_id
  ORDER BY updated_at DESC
viz: table                      # table | list | number | markdown | bar | line | treemap | grid
columns:                        # optional; controls headers, order and links
  - { key: title, label: Task, link: { kind: task, from: id } }
  - { key: status }
empty: Nothing is waiting on you.
```

- **"Mine" without hardcoding ids**: a param named `stream_id` or
  `thread_id` is filled in with the viewer's current stream / thread
  (numeric, like the `v_*` columns) unless a value is given. The lens
  page uses the stream the person is on and its selected thread;
  `run_lens` uses the `stream_id` / `thread_id` you pass (default: the
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
- **`link.kind`** makes cells clickable:
  - `task` takes a task id (a bare `v_task.id` number works).
  - `file` takes a repo-relative path; add `line: <column>` to open at a
    line.
  - `wiki` takes a slug.
  - `effort-diff` takes an effort id.
  - `commit` takes a sha.
  - `metric` takes a metric key.
  
  `from` names the result column that holds the id; it defaults to the
  column itself.
- **`launcher: { category: Activity }`** lists the lens under that
  launcher heading (Work, Code, Git, Activity, Knowledge, Data, Lenses,
  System; default Lenses). **`hidden: true`** keeps it out of the
  launcher, for lenses only a slot shows.
- **Slots** mount a lens into a core page (`slot_mounts: [{ slot, lens }]`
  in `extension.yaml`). The lens must declare at least one param the slot
  binds, and gets only the ones it declares:
  - `effort-review` (an effort's diff) → `effort_id`, `change_id`;
  - `commit` (a commit page) and `uncommitted` (the working tree) →
    `change_id` (the `v_change*` analysis);
  - `task-detail` (task page) → `task_id`;
  - `thread` (the Work panel, compact) → `thread_id`;
  - `rail` → no params; the lens must have an `alert`, and shows in the
    rail's Alerts section while it fires;
  - `settings` → no params; Settings shows a section named after the
    extension with its mounted lenses (its status or setup views).
- **`alert:`** says when a lens needs attention: `{ min_rows: 1 }` (the
  run returned at least that many rows) or `{ column: pct, below: 80 }` /
  `above:` (the first row's value), with an optional `label`. `run_lens`
  returns the alert state.
- **`actions:`** adds buttons from a fixed set:
  - `copy` copies the result (a markdown lens's text, else a markdown
    table);
  - `add-to-context` hands the lens to the agent;
  - `{ action: run-source, source: <ext>/<id>, label: Sync PRs }` syncs a
    source.

  You can press them too with `run_lens_action`. You can't approve an exec
  source that way; the person approves it in Settings → Data.
- **Unknown keys are errors.** Only the keys shown above exist today.

## 3. Check it

**Run `check` after every edit, before anything else.** Not once at the
end: after each file you write or change. It is cheap, and the message
names the file and line and says what to change.

1. `validate_extension(name, stream_id)` (MCP), or `oxplow plugin check
   <name>` from the worktree, is the same check and the same report:
   - manifest errors (YAML, unknown keys, a name/folder mismatch, a
     missing `intent`, an experimental kind in a `shared` extension);
   - cross-references that don't resolve (a slot mount naming a lens
     that doesn't exist, a link kind nobody registered);
   - a dry run of every lens and advisory with its default params (SQL
     errors, and `columns` keys the query doesn't return).

   `ok: true` (exit 0) means it works. Fix every `error`; fix a `warning`
   unless you can say why not. (The CLI dry-runs SQL only when the project
   has been opened in oxplow, so `.oxplow/local.sqlite` exists; the MCP
   tool always does.)
2. `run_lens(id, params?, stream_id, thread_id?)` returns exactly the
   rows the user will see. Check that they answer the question.
3. Pass **your own `stream_id`** to both when you're in a worktree
   stream. Extensions are read from the stream's worktree, so the primary
   stream won't see yours until it's merged.

## 4. Hand it over

Tell the user the lens title and how to open it (Cmd+P, type the title).
Offer to commit it if they want their team to have it. Lens files are
ordinary project files, so they go through the normal task/effort flow:
file or pick up a task before editing, and list the files in
`touched_files` when you close it.

## Working with the user's lenses

- `list_lenses` / `get_lens` show what already exists. Prefer improving an
  existing lens over adding a near-duplicate.
- When the user says "this lens", "what I'm looking at" or pastes
  `[oxplow lens <id>]`:
  1. Call `get_open_page(thread_id)`. Its `lensRun` holds the rows on
     their screen, with their current params.
  2. Read the definition with `get_lens`.
  3. Change the file.
  4. Validate and run it again.

## Contributing metrics

An extension can add to the metric catalog with `measures:`, `metrics:`,
`gauges:` and `dimensions:` in `extension.yaml`, in exactly the `.oxplow/project.yaml`
schema (the `oxplow-metrics` skill and `/oxplow:new-metric` cover it).

```yaml
measures:
  - { key: acme.todo, title: TODO comments }
metrics:
  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum, direction: lower-better }
gauges:
  - key: acme.todo_scan
    emits: [acme.todo]
    compute: { runtime: starlark, entryFile: gauges/todo.star }
```

- `entryFile` is inside the extension folder.
- Gauges run `starlark` or `jaq` only. `exec` is refused: running a
  program needs the user's approval, which is what sources are for.
- Metrics are `key:` definitions and are on while the extension is
  enabled. A project can still turn one off or change its target with a
  `use:` entry in `.oxplow/project.yaml`. `use:` isn't allowed in an
  extension.
- Disabling the extension drops its metrics from the catalog; its
  measures and their history stay.
- `dimensions:` work the same, including entity dimensions
  (`{ key: acme.team, entity: v_acme_issue, expr: e.team }`), which
  slice entity metrics over the same view. `promote:` isn't allowed in an
  extension.

## Bringing in outside data (sources)

When the user wants data oxplow doesn't have (GitHub PRs, Linear issues,
CI runs), add a **source** to the extension:

- Write a script that prints
  `{"entities": {"<name>": [ {col: value, …} ]}}`.
- Declare it under `collectors:` in `extension.yaml`, with its entities'
  typed columns and key.
- `examples/extensions/github/` in the oxplow repo is a complete, working
  example; copy its shape.
- The entity becomes `v_<extension>_<entity>`. Join it to core views in
  lenses.

Rules:

- Declare secrets as `credentials: [NAME]`; the script reads them as
  environment variables. The user sets the values in Settings → Extensions
  (they go to the keychain). You can't set or read them, and never
  hard-code them. `list_sources` shows which are set. Use `env: [NAME]`
  only for non-secret settings from the user's environment.
- Declare every host the script talks to: `network: [api.github.com]`
  (`*.example.com` for subdomains). On macOS nothing else is reachable
  (the script goes through a proxy; `curl`, `gh` and most HTTP clients
  pick it up from `HTTPS_PROXY`). A missing host shows up as a `403` from
  the proxy or a connection failure.
- **You can't approve a source.** Tell the user to approve it in
  Settings → Data → Approve & Run. After that, `run_source`
  (MCP) re-runs it.
- `list_sources` shows each source's status and last error.
- **Check a source before it's merged** with
  `preview_source(extension, source_id, stream_id)`: it runs your
  worktree's version and returns the rows it would store, storing
  nothing. `run_source` always runs the primary's copy (source data is
  shared by the whole project), so in a worktree stream it won't see your
  changes. An exec source still needs a person's approval for the preview.
- `describe_schema` lists declared entities with `available: false`
  until the source first syncs.

**Deriving data you already have (starlark / jaq sources).** When the
entity can be computed from views that already exist (reshaping tasks,
combining two extensions' data), use `runtime: starlark` (a script
defining `def transform(input): ...`) or `runtime: jaq`, with `input:` set
to a read-only SQL query:

- **Input and output.** The script gets `{"rows": [...]}` and returns the
  same `{"entities": ...}` shape.
- **Approval.** It runs sandboxed (no network, files, env or credentials),
  so it needs no approval and you can `run_source` it yourself.
- **Input limits.** `input` can't read the source's own views, and more
  than 10,000 input rows fails the run.

**Incremental sync.** For a big or slow upstream, set `sync: upsert`:

- **What a run returns.** Only new and changed rows, plus
  `"deleted": {"<name>": [key, …]}` for removed ones.
- **Unmentioned entities.** One the run doesn't return is left as it was.
- **Default.** `sync: replace` makes each run restate everything.

## Sharing and installing

- **Team:** extensions are ordinary committed files under
  `oxplow/extensions/`. Commit them and the team has them.
- **World:** to publish, put an extension in its own git repo with
  `extension.yaml` at the root. To use someone else's, **only when the
  user asks**: call `review_extension(git_url, git_ref?, stream_id)`, show
  the user what it declares (the programs its sources run, the hosts they
  reach, the credentials they read, advisories) and any `errors` /
  `problems`, and when they say go, call `install_extension(git_url,
  git_ref?, reviewed_sha: <its sha>, stream_id)`. It records the source in
  `source.yaml`. Updating is the same: `review_extension(name)`, then
  `update_extension(name, reviewed_sha)`. Never
  hand-edit an installed extension's files; they're overwritten on update.
  Copy it into a new extension instead.
