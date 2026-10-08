---
name: oxplow-extension
description: Build oxplow lenses and extensions on request: custom pages over oxplow's data (tasks, efforts, threads, comments, wiki, snapshots, metrics facts) that the user sees in the app and can share with their team. Loads when the user asks to "make/build/show me a lens/page/view/list/report of X", "a place where I can see X", "track X across threads/efforts", "what's waiting on me", or asks to change an existing lens. Teaches the semantic layer (`v_model`, `query_sql` over `v_*` views), the `oxplow/extensions/<name>/` file format, and the validate → run_lens loop.
---

# Building oxplow lenses

A **lens** is a saved query over oxplow's data plus how to show it. The user
opens it as a page (Cmd+P → Lenses), and it updates as the work changes.
Lenses live in **extensions**: folders of plain files in the repo. The user
can use a lens themselves, share it with their team by committing it, or
publish the extension for anyone.

You build them with your normal file tools. `oxplow extension new <kind>
<name> --origin <your effort ref>` scaffolds the folder with a v2 manifest,
an intent, one example and its fixture, and a working starter for the
kind; then edit. Each passes `check` and `extension test` as written, so a
failure after your edit is yours:
- `lens` — a lens over open tasks with a row action (Start);
- `collector` — a Starlark collector of an entity, a model over it
  (`ref('item')`) and a lens over the model;
- `command` — a command whose script comments on a work item, on a work
  item's Commands menu;
- `provider` — an external work-items provider's declarations and a stub program
  (red until you write the program);
- `effect` — a script reacting to a logged event by composing commands
  (private; runs only once a person approves it);
- `component` — a custom component (private): its `viz: custom` lens and
  a web bundle on oxplow's client library;
- `extension` — the manifest only.

## 0. Answering with a lens (`show_lens`)

When the user asks a question whose answer is a table, chart or list —
"which files changed most this week?" — **show** it rather than pasting
rows into the terminal: call `show_lens` with either `lens` (an existing
lens id, plus `params`) or `spec` (a lens of your own: `title`, `query`,
`viz` and what the viz needs, the same keys as a lens file). oxplow runs
the query read-only (as `query_sql` does), stores the answer on the
thread and shows it beside the conversation; the tool returns the
answer's ref and its text rendering, which is what you tell the user.
An answer is never a `form` (its submit would run a command as the
user), a `grid` or a `custom` component.

Nothing is written to the repo. The user presses **Keep This** to turn an
answer into a private lens (in `my-lenses`), and shares it from there. Build
a lens file (below) only when they ask for a lasting page.

## 1. Find the data

- `v_model` lists every queryable view (`v_stream`, `v_thread`, `v_work_item`,
  `v_effort`, `v_comment`, `v_wiki_page`, `v_snapshot`, `v_measure`,
  `v_capture`, `v_fact`, …) with its doc, and `v_model_column` documents
  every column: `SELECT view, name, sql_type, doc FROM v_model_column WHERE
  view = 'v_work_item'`. **Read them first**; don't guess column names.
- Work items are `v_work_item` (with `v_work_item_link` and
  `v_work_item_comment`): the active work list's, whichever it is — never
  a particular list's own tables.
- `query_sql` runs one read-only `SELECT`/`WITH`. Iterate on the query
  here until the rows are what the user wants, before writing any files.
- Only `v_*` views can be read; a physical table is refused, naming the
  view to read instead. Metrics are read with `metric_grid()` and
  `MEASURE('<key>')` (see the `oxplow-metrics` skill).

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
  - `effort-diff` takes an effort id.
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
- **Slots** mount a lens into a core page (`ui: { slots: [{ slot, lens }] }`
  in `extension.yaml`). The lens must declare at least one param the slot
  binds, and gets only the ones it declares:
  - `effort.review.details` (an effort's diff) → `effort_id`, `change_id`;
  - `vcs.commit.details` (a commit page) and `vcs.status.details` (the
    working tree) → `change_id` (the `v_change*` analysis);
  - `vcs.status.header` (a strip above the uncommitted changes) and
    `vcs.history.sidebar` (beside Git History) → `stream_id`;
  - `work_item.detail.body` and `work_item.detail.sidebar` (a work item's
    page, whichever list) → `ref`;
  - `thread.plan.header` (the Work panel, compact) → `thread_id`;
  - `settings.section` → no params; Settings shows a section named after
    the extension with its mounted lenses (its status or setup views).

  The old names (`task-detail`, `commit`, …) are errors naming the new
  one.
- **A command on a ref's menus** (its page's Commands, a row's
  right-click) says so in its own `ui`: `ui: { label, group?, about:
  work_item, input?: { ref: "{{ref}}" } }` (`about` is a ref kind;
  `"{{ref.id}}"` binds the id alone). To offer another command there with
  a fixed input, declare your own over the same operation.
- **A provider's own operations** are your commands too: `commands: [{
  name: item.estimate, summary, provider: <your provider id>, op:
  estimate, ui? }]` — the operation's input (plus an optional
  `instance`) and access come from its declarations; it runs on the
  instance its `ref` (or `instance`) names. Its capability verbs aren't
  declared: they run as `oxplow.work_item.<verb>`.
- **`ui.decorators`** add a label to refs:
  `{ model, kind, placement: ref-chip | row-badge, label, color? }` — the
  model (this extension's) needs a `ref` column plus the `label` (and
  `color`) columns; a chip shows on that ref's page, a badge after a
  linked cell in lens rows.
- **`commands:`** register commands of your own, as
  `<namespace>.<area>.<verb>`: `{ name: area.verb, summary, input_schema,
  entry: handlers/x.star, needs?: [sql.read], confirm?, access?,
  invokers?, examples? }`. The Starlark `transform(x)` gets `{ input }`,
  reads with `scope("sql.read", { "sql": "SELECT … WHERE ref = :ref",
  "params": { "ref": x["input"]["ref"] } })` (a list of row dicts; it must
  list `sql.read` in `needs`) and returns `{ commands: [{ name, input }],
  result? }`
  — or `{ refuse: "why" }` to decline (the caller sees the reason).
  Splice text an agent wrote (a claim, a decision) into a comment or
  body with `md_text(text)`: one inert line, so it can't add headings,
  links or items;
  those core commands run as the caller, each one's own policy and
  confirmation checked first — in one transaction with one undo when they
  all touch oxplow's own records, or in order through the provider when
  one touches another provider's item (then what ran before a failure
  stays, and there's no undo). It does no I/O, has 5 s,
  and `examples` (at most 10) are dry-run by `check`: give each
  `answers: { sql.read: [[rows of the 1st read], …] }` so it doesn't
  depend on the project's data, and `expect_commands: [...]` or `refuses:
  <part of the reason>`. `access: read` makes a command that only reads
  and returns a `result` (it composes nothing). A command on a ref's menu (`ui.about`) gets `{ ref }` unless its
  `ui.input` says otherwise, so name its input field `ref`. `extensions/oxplow-bundled/` (bundled) is a
  working example.
- **`custom_components:`** (stable; not in a bundled extension) are web bundles a
  `viz: custom` lens renders: `{ id, title?, bundle?: components/<id>,
  assets: [lens ids], commands: [names] }`. The bundle (an `index.html`
  and its files, at most 256 files / 5 MiB) runs sandboxed — no network,
  no storage, no token — and reaches oxplow only by asking the host to
  run one of its `assets` lenses, invoke one of its `commands` (a
  component that declares any is a program a person approves on
  Settings → Data → Programs before its `invoke` runs; a command that asks
  still asks) or navigate. Its `index.html` loads
  oxplow's client library with a plain `<script
  src="/component-lib/oxplow-component.js">` before its own plain script
  (no modules: a sandboxed frame can't load them), and the script calls
  `oxplow.connect().then((component) => …)` for `component.run`,
  `onUpdate`, `query`, `invoke` and `navigate`. `oxplow extension new
  component <name>` scaffolds a working one; `docs/guide/lenses.md` has
  the reference.
- **Pages** give a lens a place of its own: `pages: [{ id, title, icon?,
  category, lens }]` opens it full-page at `page:ext.<extension>.<id>` and
  lists it in the launcher under `category` (Work, Code, Git, Activity,
  Knowledge, Data, Lenses, System).
- **Panels** put a lens in the left nav (`panels: [{ id, title, icon?,
  scope, body, badge?, open?, collapsed?, count? }]`): `body` renders
  compact; `badge` (a lens with an `alert`) gives the panel's count and
  lists in the Alerts panel while it fires; `collapsed` is a lens shown
  compact while the panel is collapsed (its summary); `count` is a lens
  whose row count (a `number` lens: its value) is the header count
  without an alert — it wins over the badge's. `scope` is `project`, `stream` or `thread` — a `stream` /
  `thread` panel's lenses must declare `stream_id` / `thread_id`, which the
  nav binds. The person arranges, hides and collapses panels.
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
  `above:` (the first row's value), with an optional `label`. `run_lens`
  returns the alert state.
- **`actions:`** are commands the lens offers:
  `{ id, label, command, input, row? }`. `input` is the command's input;
  `"{{param.x}}"` binds a lens param and, with `row: true` (a row's
  right-click action), `"{{row.col}}"` binds that row's column — which
  the query must return even when `columns:` doesn't show it (select a
  ref for the action and leave it out of `columns:`) —
  e.g. `{ id: finish, label: Finish, command: oxplow.work_item.transition,
  row: true, input: { ref: "work_item:oxplow:tsk{{row.id}}", to: done } }`,
  or `{ id: sync, label: Sync PRs, command: oxplow.collector.sync, input:
  { owner: github, id: prs } }`. With `group: <value>` (on a lens with
  `group`), the action is a button in that group's heading instead — it
  shows in a compact panel too; it can't be a row action. Copy and Add
  to Agent Context are on every lens already — don't declare them.

  An action runs as the lens acting for whoever pressed it, so it can't do
  anything they couldn't: you can press one with `run_lens_action`, but a
  command you can't run (or that needs a person's confirmation, like
  approving an exec collector) is refused.
- **Unknown keys are errors.** Only the keys shown above exist today.

## 3. Check it

**Run `check` after every edit, before anything else.** Not once at the
end: after each file you write or change. It is cheap, and the message
names the file and line and says what to change.

1. `validate_extension(name, stream_id)` (MCP), or `oxplow extension check
   <name>` from the worktree, is the same check and the same report:
   - manifest errors (YAML, unknown keys, a name/folder mismatch, a
     missing `intent`, an experimental kind in a `shared` extension, a
     Starlark collector script that doesn't parse or define `transform`);
   - cross-references that don't resolve (a slot mount naming a lens
     that doesn't exist, a link kind nobody registered, a command that
     doesn't exist or an input that doesn't fit it);
   - its models compiled without publishing (SQL errors, a `ref()` or
     `source()` that doesn't resolve, columns that differ from the
     declared contract, a changed contract at a published version);
   - a dry run of every lens, advisory and command `input` with its
     default params (SQL errors, and `columns` keys the query doesn't
     return), and of every command example.

   It **always** dry-runs, and what you declared but haven't run yet is
   there for it: a collector's entities stand in empty, and your own
   models are compiled for the check. So collector → model → lens checks
   clean before the first sync — don't sync just to make `check` pass.
   (The CLI uses the project's database when it has one, else an empty
   one.) `ok: true` (exit 0) means it works. Fix every `error`; fix a
   `warning` unless you can say why not.
2. `run_lens(id, params?, stream_id, thread_id?)` returns exactly the
   rows the user will see. Check that they answer the question.
3. Pass **your own `stream_id`** to both when you're in a worktree
   stream. Extensions are read from the stream's worktree, so the primary
   stream won't see yours until it's merged.
4. An extension with an **external provider** (`providers:` — a program that
   connects an outside system, such as an issue tracker; private
   extensions only) is tested with `oxplow extension test <name>`:
   `oxplow extension new provider <name>` scaffolds one (its `provider.json`
   declarations, a stub `bin/provider` to replace, and the fixtures).
   The test runs `check`, then the provider: its `initialize` must equal
   `provider.json`, its `check` must accept `fixtures/provider-<id>.yaml`'s
   `config`, each intent example's fixture (`input: { command, input }`,
   `expect`, `$any` matching anything) is invoked, every message must
   match the protocol, the session must match the golden
   `fixtures/transcripts/<id>.jsonl` (`--bless` writes it when a change is
   intended — commit it), and the work-items conformance suite must pass.
   A `ref_kinds:` entry may add `searchable: <model>` (a model of yours
   with `ref`, `title`, `body`): its rows are then found by the launcher's
   search and open the kind's page.
   An effect's `on:` (and a collector's `trigger: { on: [...] }`) may
   name another extension's event type (`acme_pr.merged`): `check` warns,
   and the extension shows an error while no enabled extension registers
   that type. The payload you get is at the owner's newest version.
   A provider's `credentials:` entry is a name (the person pastes its
   value) or `{ name, oauth: { authorize_url, token_url, client_id,
   scopes } }` (the person signs in on Settings → Integrations; oxplow
   runs the flow and renews the token, and your program reads the access
   token from the env var of that name — answer `Auth` (-32002) when the
   service refuses it and oxplow renews it and calls again). You can't
   sign in or set a credential yourself.
   A person approves the provider in Settings → Data → Programs and
   enables it in Settings → Integrations; you can't do either.
   A work-items provider is a **backend for the project's own work**,
   with the visibility the person wants (a local tracker like beads), not
   a team's tracker: when it's the active one, every new item goes there.
   In the oxplow repo, `tests-e2e/fixtures/extension/` (its manifest and
   `provider.json`) and `crates/oxplow-provider-fake` (the program) are a
   complete one: copy their shape.
   An existing **MCP server** can be the provider instead of a program:
   `adapter: { mcp: { command: [bin/server] }, mapping: mcp/x.star,
   tools: mcp/tools.json }` in place of `entry` — oxplow's adapter runs
   the server, `check` refuses it unless its tools equal the pinned
   `tools.json`, and `transform(x)` in the mapping turns each verb into a
   tool call (`x.phase` `invoke`) and the tool's output into the answer
   (`invoked`; `read` / `records` for a collector). A server that
   already runs elsewhere is `mcp: { url: https://…, auth: NAME }`
   instead of `command`: https only (http on loopback), its host in
   `network`, and `auth` one of the provider's `credentials` (sent as the
   bearer token).
5. **`oxplow extension test <name>`** runs every intent example on a
   throwaway oxplow (empty data, your extension's entities published
   empty and its models and commands loaded — never the project's
   database). Give each example a fixture, `fixtures/<example>.yaml`, with
   `input` and `expect` (`$any` matches anything):
   - a lens: `input: { lens: <slug>, params? }`, `expect: { columns?,
     rows: n }` — data is empty, so usually `rows: 0`;
   - a collector: `input: { collector: <id>, rows: [...] }` (`rows`
     stands in for its `input` query), `expect: { entities: { <name>: n
     } }` — it runs your script and types its rows; an exec collector's
     example isn't run (a person approves it);
   - a fact collector (one with `facts:`): `input: { collector: <id>,
     files: { "src/a.ts": "…" } }` (the tree its `files()` sees),
     `expect: { facts: [{ measure, value, path? }] }` — each fact is
     compared on the keys you name, in order;
   - a command: `input: { command: <id>, input: {...}, answers? }`,
     `expect: { commands: [names] }` or `{ refuses: <part of the reason>
     }` — a dry run; nothing changes;
   - a provider: `input: { command, input }` (below).
   Give a command's own `examples:` `answers:` too, so they don't depend
   on the project's data.
6. `oxplow extension test` also runs the extension's `questions.yaml`, if it
   has one: questions an agent should be able to answer with it, each
   `{ question, skill, reaches: { sql } or { command, input }, shape:
   { columns } }`. `skill` is a markdown file in the extension (a
   README); it must name every view the SQL reads and the command it
   runs (core's, the extension's own, or a provider's), the SQL must
   return exactly `shape.columns`, and the input must fit the command.
   Write them for what the extension is *for*, whatever its kind.

## 4. Hand it over

Tell the user the lens title and how to open it (Cmd+P, type the title).
Offer to commit it if they want their team to have it. Lens files are
ordinary project files, so they go through the normal task/effort flow
(the files you write are observed).

## Working with the user's lenses

- `list_lenses` / `get_lens` show what already exists. Prefer improving an
  existing lens over adding a near-duplicate.
- When the user says "this lens", "what I'm looking at" or pastes
  `[oxplow lens <id>]`:
  1. Call `get_open_page(thread_id)`. Its `lens` is what is on their
     screen as text, with their current params (`run_lens` with
     `format: "json"` gives you the raw rows if you need them).
  2. Read the definition with `get_lens`.
  3. Change the file.
  4. Validate and run it again.

## Publishing models

When other lenses, extensions or agents should read a shaped dataset —
not just one lens — publish it as a **model**: a documented, versioned
view. Declare it in `extension.yaml` and write its SQL in
`models/<name>.sql`:

```yaml
models:
  - name: blocked
    version: 1
    description: Blocked tasks, oldest first.
    columns:                       # the contract: every column, in order
      - { name: id, type: INTEGER, doc: Task id. }
      - { name: title, type: TEXT, doc: Title. }
    tests:
      - { not_null: id }
      - { unique: id }
```

```sql
-- models/blocked.sql
SELECT ref, title FROM ref('work_item') WHERE state = 'blocked'
```

- It publishes as `v_<extension>_<name>` (dashes as underscores):
  `v_late_work_blocked` for extension `late-work`.
- Read through `ref()` only: `ref('work_item')` is a core model (`v_work_item`),
  `ref('blocked')` your own model or synced entity, and
  `ref('other-ext/name')` another extension's. `source('<table>')` is only
  for your own source tables (`ext__<ext>__<entity>`).
- `type` is what SQLite reports for the column (`PRAGMA table_info`);
  leave it out for a computed column (`count(*)`, an expression).
- **Changing the columns is a breaking change:** bump `version`. To keep
  the old shape for readers, keep its SQL in another file and list it:
  `deprecated: [{ version: 1, file: blocked.v1.sql, until: 2027-01-01 }]`
  publishes `v_<ext>_blocked_v1` until then.
- Check runs the compile without publishing; oxplow publishes after
  the files change. A failing declared test shows in the extension's
  errors and `v_model_test`, and the view stays up.

## Contributing metrics

An extension can add to the metric catalog with `measures:`, `metrics:`
and `dimensions:` in `extension.yaml`, plus **fact collectors** (a
`collectors:` entry with `facts:` instead of `entities:`) that record the
facts — in exactly the `.oxplow/project.yaml` schema (the `oxplow-metrics`
skill and `/oxplow:new-metric` cover it).

```yaml
measures:
  - { key: acme.todo, title: TODO comments }
metrics:
  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum, direction: lower-better }
collectors:
  - id: acme.todo_scan
    runtime: starlark
    entry: collectors/todo.star
    trigger: { on: [snapshot.taken] }
    facts: [acme.todo]
```

- `entry` is inside the extension folder.
- An extension's fact collector runs `starlark` or `jaq` only. `exec` is
  refused (only the project's own fact collectors may be `exec`); it gets
  no `env`, `credentials` or `network`. Its script reads the snapshot
  with `files()` / `ast_query()` and returns `{"facts": [...]}`.
- A collector has `entities:` or `facts:`, never both.
- A `gauges:` block is an unknown key; write `collectors:`.
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

## Bringing in outside data (collectors)

When the user wants data oxplow doesn't have (GitHub PRs, a team's issues,
CI runs), add a **collector** to the extension:

- Write a script that prints
  `{"entities": {"<name>": [ {col: value, …} ]}}`. It may add
  `"events": [{"type": "<your_ns>.<name>", "payload": {…}}]` — only event
  types your extension declares (`event_types:`), at most 100 a run, and
  never a type the collector itself runs on. Use it to say what the
  collector saw; put what should *happen* in an effect that reacts to it.
- Declare it under `collectors:` in `extension.yaml`, with its entities'
  typed columns and key, and when it runs: `trigger: { every: 15m }` or
  `trigger: manual` (the default).
- `examples/extensions/github/` in the oxplow repo is a complete, working
  example; copy its shape.
- The entity becomes `v_<extension>_<entity>`. Join it to core views in
  lenses.

Rules:

- Declare secrets as `credentials: [NAME]`; the script reads them as
  environment variables. The user sets the values in Settings → Extensions
  (they go to the keychain). You can't set or read them, and never
  hard-code them. `list_collectors` shows which are set. Use `env: [NAME]`
  only for non-secret settings from the user's environment.
- Declare every host the script talks to: `network: [api.github.com]`
  (`*.example.com` for subdomains). On macOS nothing else is reachable
  (the script goes through a proxy; `curl`, `gh` and most HTTP clients
  pick it up from `HTTPS_PROXY`). A missing host shows up as a `403` from
  the proxy or a connection failure.
- **You can't approve a collector.** Tell the user to approve it in
  Settings → Data → Approve & Run. After that, `run_collector`
  (MCP) re-runs it.
- `list_collectors` shows each collector's last run and error.
- **Check a collector before it's merged** with
  `preview_collector(owner, id, stream_id)`: it runs your worktree's
  version and returns the rows it would store, storing nothing.
  `run_collector` always runs the primary's copy (collected data is
  shared by the whole project), so in a worktree stream it won't see your
  changes. An exec collector still needs a person's approval for the
  preview.
- A declared entity's view exists once its collector first runs.

**Deriving data you already have (starlark / jaq collectors).** When the
entity can be computed from views that already exist (reshaping tasks,
combining two extensions' data), use `runtime: starlark` (a script
defining `def transform(input): ...`) or `runtime: jaq`, with `input:` set
to a read-only SQL query:

- **Input and output.** The script defines `def transform(x):` at the top
  level (`check` says so at the collector's line otherwise); it gets
  `{"rows": [...]}` and returns the same `{"entities": ...}` shape. It
  must handle **no rows** — return an empty list, don't index `rows[0]`
  — because a fresh project, a `extension test` throwaway and a filter that
  matches nothing all give it none; with `sync: replace` an entity it
  leaves out is emptied.
- **Approval.** It runs sandboxed (no network, files, env or credentials),
  so it needs no approval and you can `run_collector` it yourself —
  unless it calls a model (`ai_*`, below): that spends the user's key on
  every run, so the user approves it first (Settings → Data → Approve &
  Run), and each changed version again. Until then a run or a preview is
  refused, and `extension test` skips its examples.
- **Input limits.** `input` can't read the collector's own views, and more
  than 10,000 input rows fails the run.
- **Asking a model.** A starlark collector can call `ai_classify(text,
  labels)` → `{label, probabilities}`, `ai_score(text, levels)` →
  `{level, score, probabilities}` (levels lowest first),
  `ai_summarize(text, focus = None)` → text, and `ai_extract(text,
  schema, instructions = "")` → JSON matching `schema`. Each is recorded
  (`v_ai_result`): the same question on the same text is one model call,
  ever, so a re-sync costs nothing for rows that didn't change. They run
  on the project's AI roles (`decide`, `summarize`, `main`); with no model
  assigned the run fails saying which role. Fact collectors and report
  parsers can't call them. For example, what kind of work each turn was:

  ```yaml
  collectors:
    - id: turn_kind
      runtime: starlark
      entry: turn_kind.star
      input: "SELECT id, prompt FROM v_agent_turn WHERE prompt <> ''"
      entities:
        - { name: turn_kind, key: id, columns: { id: int, kind: text } }
  ```

  ```python
  KINDS = ["feature", "bug fix", "refactor", "question", "chore"]

  def transform(input):
      return {"entities": {"turn_kind": [
          {"id": r["id"], "kind": ai_classify(r["prompt"], KINDS)["label"]}
          for r in input["rows"]
      ]}}
  ```

**Incremental sync.** For a big or slow upstream, set `sync: upsert`:

- **What a run returns.** Only new and changed rows, plus
  `"deleted": {"<name>": [key, …]}` for removed ones.
- **Unmentioned entities.** One the run doesn't return is left as it was.
- **Default.** `sync: replace` makes each run restate everything.

## Health and repair

oxplow keeps each provider's, collector's and effect's health on this machine
in `v_contribution_health` (`extension`, `contribution`, `kind`, `state` = `ok` |
`failing` | `disabled`, `reason`, `last_error`, `dead_letters`, `fresh`,
`repair_item`). Each collector's last run is in `v_collector_run`
(`status`, `last_run_at`, `error`).

- **Three failures in a row disable it.** A disabled collector doesn't
  run, `oxplow.collector.sync` refuses naming the reason, and a lens over its
  view carries the reason as a warning.
- **A disable files a repair work item** (`repair_item`). Its body is the
  whole repair brief: what the extension is for, the declaration, the
  last errors, the failing fixture and the `check` report. A repeat
  failure comments on the open item. When the user asks you to repair an
  extension, read that item first.
- **Fix it, then prove it.** Change the script or declaration, run
  `oxplow extension check` and `oxplow extension test`, then `oxplow.collector.sync`
  (`run_collector`) — a refusal while it's disabled is expected.
- **You can't enable it again.** `oxplow.contribution.enable` is the user's: tell them
  to press **Enable Again** in Settings → Extensions once your fix is in.
- **Undelivered events** (a consumer that failed on an event) are in
  `v_event_dead_letter` (`state = 'pending'`); the user retries or
  discards them in Settings → Data → Delivery.

## Sharing and installing

- **Team:** extensions are ordinary committed files under
  `oxplow/extensions/`. Commit them and the team has them.
- **World:** to publish, put an extension in its own git repo with
  `extension.yaml` at the root. To use someone else's, **only when the
  user asks**: call `review_extension(git_url, git_ref?, stream_id)`, show
  the user what it declares (the programs its collectors run, the hosts they
  reach, the credentials they read, advisories) and any `errors` /
  `problems`, and when they say go, run `oxplow.extension.install { git_url,
  git_ref?, reviewed_sha: <its sha> }` (`mcp__oxplow__run_command`) — it
  comes back as a proposal the user approves. It records the source in
  `source.yaml`. Updating is the same: `review_extension(name)`, then
  `oxplow.extension.update { name, reviewed_sha }`. Never
  hand-edit an installed extension's files; they're overwritten on update.
  Copy it into a new extension instead.
