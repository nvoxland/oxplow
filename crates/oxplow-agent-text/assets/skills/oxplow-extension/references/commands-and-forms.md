# Commands, custom components and forms

- **A command on a URI's menus** (its page's Commands, a row's
  right-click) says so in its own `ui`: `ui: { label, group?, about:
  oxplow/task, input?: { uri: "{{uri}}" } }` (`about` is a resource
  name, `<namespace>/<name>`; `"{{uri.id}}"` binds the URI's key param, such as
  the id, alone). In a page's ☰ it sits at the top level among the page's
  own actions, sorted by label (`group` is the palette's heading and the
  row menu's grouping, not a ☰ submenu). Add `about_when: <column>` to
  offer it only on a ref whose row has that column truthy (not null, 0,
  `""`, false) — a column of the dataset the resource resolves through,
  `closed` or `has_task`: a ref in another state is not offered the
  command, so use it where the operation refuses some states. To offer
  another command there with a fixed input, declare your own over the
  same operation.
- **A command on a link's Open In ▸** (a web URL, a file or a directory
  anywhere the app shows one: the terminal, a markdown link, a page row, a
  tab) has `group: Open In` and is about the link: `about: https` (a web
  link, by its scheme) or `about: oxplow/file` / `oxplow/dir`. Narrow it
  with `about: { resource, accepts: { url: <pattern> } }` (the whole URL,
  anchored `^…$`, a class instead of `.`:
  `"^https://github[.]com/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+/pull/[0-9]+$"`) or
  `accepts: { path: "**/*.ipynb" }` (globs over the worktree path; `*`
  stops at `/`). It gets `{ uri }`: the URL, or the file's URI. Test it
  with a fixture `when: { item_commands: "https://…" }` →
  `expect: { commands: [<ids>] }`.
- **A command on a file's Git ▸** (beside oxplow's blame, history, diff,
  compare and rollback) is the same, with `group: Git` and `about:
  oxplow/file` (or `oxplow/dir`).
- **Deciding with your own logic:** `ui.about_if: <function>` names a
  function of the command's `entry:` script. It gets `{ uri, kind, url?,
  path? }` for each link that passes `accepts` and returns `True` where
  the command is offered (`def is_mine(link): return
  link["url"].startswith("https://mine.example/")`). Each call is a script
  run, so keep a `url` or `path` filter in front of it. It runs with the
  command's `needs`: with `sql.read` (and `permissions: [sql.read]`) it may
  `scope("sql.read", {...})`, e.g. `v_vcs_remote` (the repository's
  remotes) to see which forge the project is on.
- **Leaving the window:** a command a person runs may compose
  `oxplow.window.open_url { url }` (their browser) or
  `oxplow.window.copy_text { text }` (their clipboard). An agent is refused
  both.
- **Making text a link** (an issue key, a ticket number) in the terminal
  and in markdown: a top-level `links: [{ id, pattern, uri }]`. The pattern
  is a param pattern without `^…$` (classes, no `.`), matched as a whole
  word; the uri is `https://…` or an oxplow URI with `{{match}}`:
  `{ id: issue, pattern: "[A-Z]{2,10}-[0-9]+", uri:
  "https://linear.app/acme/issue/{{match}}" }`. The link gets Open In ▸
  and the commands about its URI. Test it: `when: { link_text: "fixes
  PROJ-12" }` → `expect: { links: [{ text: PROJ-12, uri: "…" }] }`.
- **A provider's own operations** are your commands too: `commands: [{
  name: item.estimate, summary, provider: <your provider id>, op:
  estimate, ui? }]` — the operation's input (plus an optional
  `instance`) and access come from its declarations; it runs on the
  instance its `uri` (or `instance`) names.

- **`commands:`** register commands of your own, as
  `<namespace>.<area>.<verb>`: `{ name: area.verb, summary, input_schema,
  entry: handlers/x.star, needs?: [sql.read], confirm?, access?,
  job?, examples? }`. The Starlark `transform(x)` gets `{ input }`,
  reads with `scope("sql.read", { "sql": "SELECT … WHERE uri = :uri",
  "params": { "uri": x["input"]["uri"] } })` (a list of row dicts; it must
  list `sql.read` in `needs`) and returns `{ commands: [{ name, input }],
  result? }`
  — or `{ refuse: "why" }` to decline (the caller sees the reason).
  Splice text an agent wrote into a comment or
  body with `md_text(text)`: one inert line, so it can't add headings,
  links or items;
  those core commands run as the caller, each one's own policy and
  confirmation checked first — in one transaction with one undo when they
  all touch oxplow's own records, or in order through the provider when
  one touches another provider's item (then what ran before a failure
  stays, and there's no undo). It does no I/O, has 5 s,
  and `examples` (at most 10) are dry-run by `check`: give each
  `answers: { sql.read: [[rows of the 1st read], …] }` so it doesn't
  depend on the project's data, and `expect_commands: [...]` or `refused:
  <part of the reason>`. An agent is shown an `example:` input beside the
  schema (a script's first example that runs, unless it declares one;
  give one to a command over a `scope:` op). `access: read` makes a command that only reads
  and returns a `result` (it composes nothing). A command on a URI's menu (`ui.about`) gets `{ uri }` unless its
  `ui.input` says otherwise, so name its input field `uri`. The `oxplow-bundled` extension (see it in
  Settings → Extensions) is a working example.
- **`custom_components:`** (stable; not in a bundled extension) are web bundles a
  component node renders: `{ id, title?, description?, bundle?: components/<id>,
  commands: [names] }`. The bundle (an `index.html`
  and its files, at most 256 files / 5 MiB) runs sandboxed — no network,
  no storage, no token — and reaches oxplow only by asking the host to
  invoke one of its `commands` (a
  component that declares any runs them under the extension's
  permissions, approved when it was deployed; a command that asks
  still asks), run an **action point** it lists in `action_points: [<id>]`
  (`invokePoint(point, context, command?)` runs the one command that
  answers the point for the context, as the person; a drag of a card to
  another column is "move this item to stage X" — the store that joined
  the point answers, so the component names no store's command and needs
  no `commands:` for it; `command` picks one when several answer) or
  navigate. Its `index.html` loads
  oxplow's client library with a plain `<script
  src="/component-lib/oxplow-component.js">` before its own plain script
  (no modules: a sandboxed frame can't load them), and the script calls
  `oxplow.connect().then((component) => …)` for `component.data` (each
  loader's rows by name), `onUpdate(({ data, changed }) => …)`, `invoke`, `invokePoint` and `navigate`; it never
  fetches (a page loader feeds it). `$OXPLOW_BIN extension new
  component <name>` scaffolds a working one; `docs/guide/pages.md` has
  the reference; `dashboards.md` has an example. A component written in TypeScript or with a library is
  built at deploy: a `build:` block, with `bundle:` under its `output`
  (`repair-and-sharing.md`).

## Forms

- **A form node** (`viz: form`, no `from`) is one command's input:
  `form: { command, defaults?, fixed?, fields? }`. `defaults` takes
  `{{param.x}}`; a `query` prefills from its first row; `fixed` fields
  are the host's: never drawn, sent with their default (an item's `uri`);
  `fields` says which to show, and how each reads: `{ name, label, help }`
  (below).
- **Editing what is saved:** `mode: edit` (and list its `fields`). Save
  is enabled only when changed, Discard puts the saved values back, and
  what the person typed survives Back and a restart. Without it the form
  is for creating something new.
- **A field by another's value:** a `fields` entry
  `{ name: url, visible_if: { kind: [http, https] } }` shows only while
  `kind` is one of those; hidden, it is neither required nor sent. New
  Stream's three modes are this.
- **A secret** (a token, a key): `{ name: token, secret: true }`. It is
  masked, never kept, and blank leaves it out (the saved one stays). The
  command must keep it out of its record: a script command declares
  `unrecorded: [token]` on its `commands:` entry, or the form won't open.
  A schema field with `writeOnly: true` is masked the same way and kept
  out of the record by itself.
- **Several forms on one page:** a `stack` of form nodes; each submits its
  own command.
- **After a submit**, a command with `ui.open_after` opens that page;
  `then:` on the action (or `form.then`) says what becomes of the
  page: `open` (default; Back returns), `replace` (it leaves history —
  a create form giving way to what it made), `stay` (nothing opens) or `close` (the page closes: it showed what the command removed).
- **A form action** on any page: `form: true` on an action opens the
  command's form starting from its `input`, instead of running at once
  (a dialog over the page's tab; one text field alone is a prompt strip
  above the rows). The form is headed and its button labelled with the
  action's `label` less a trailing `…`. `fixed` keeps fields to the bound
  values and **never draws them** (an id means nothing to the person); say
  what the form is about in words instead: a row action's `about: <column>`
  is its subtitle (a page action's form is about the page, its title).
  `input` values that aren't `fixed` are where the fields start (bind the
  text a person sees, e.g. a `display_name` column, not an empty one).
  `{ id: edit, label: Edit…, command: oxtask.task.update, row: true,
  form: true, about: title, fixed: [id], input: { id: "{{row.native_id}}", title: "{{row.title}}" } }`.
- **How a form's fields read** (an action's `fields`, or a form node's):
  `fields: [{ name: title, label: Name, help: What the tab shows. }, native]`
  lists the only fields shown, in order (the rest keep `input`'s value),
  each with the `label` over it (else the schema `title`, else the name
  humanized: `native_rename` → "Native rename") and a `help` line under
  it. A command's schema `description` is written for agents and is never
  shown to a person: say what a person should read in `help`. An unknown
  field is refused where the page loads. A boolean is a checkbox with its
  label to the right and its help under that. A field in `choices` is a
  select showing each row's `label` and sending its `value`, starting on
  the `input` value. `choices: { model: { from: <loader>, params: {
  provider: "{{row.id}}" }, open: true } }` runs the loader with params
  bound from the row, and `open` makes its rows suggestions under a text
  field that takes any value (a model id no list knows). A field's `from:` names a query (or a loader of this page) for those rows.
- **Drag, bulk and key actions** (row actions): `drag: true` runs on a
  drop, binding `{{before.x}}` / `{{after.x}}` (the rows below / above
  where it landed; a missing one is left out), `{{onto.x}}`,
  `{{group.value}}` and `{{order.x}}` (a list: column `x` of every row in
  its new order, for a command that takes a whole order; only as a
  field's whole value) — `viz: board` with `group: { by, values }` drags
  cards between columns; `bulk: true` runs once per marked row; `key: f`
  runs on the cursor row. You run the command yourself; these are the
  person's gestures. A drag action runs only on a drop, so it can't also
  be `primary`, `form`, `group`, `key` or `bulk`: the loader refuses it.
- **An action point instead of a command** (`point: oxplow/task.move,
  context: { uri: "{{row.uri}}", stage: "{{option.value}}" }`): whichever
  command a store joined to the point (and accepts that item) runs, so the
  page names no store's command. `context:` binds like `input:`; `command:`
  beside it picks one when several answer; `bulk`, `drag`, `row` and
  `options_from` work as usual. A row nothing answers is skipped with a
  reason, not failed. No `form` with it.
- **One action, many choices** (a row action): `options_from: <loader>`
  offers each row of that loader of this page (`value`, `label`, an
  optional `group`) as an entry of a submenu, and binds the one picked as
  `{{option.value}}` / `{{option.label}}`; `current: <column>` is the
  row's column holding the option it is in now, shown checked and not
  offered. A Mark ▸ of a list's stages:
  `{ id: mark, label: Mark, point: oxplow/task.move, row: true,
  options_from: stages, current: stage, context: { uri: "{{row.uri}}", stage: "{{option.value}}" } }`.
  With `bulk: true` the marked rows get a select. No `form`, `drag`,
  `edit`, `group` or `key` with it.
- **In a component**, a `<form>` is fine: handle `submit` in your script
  (`preventDefault`) and run the command with `invoke`. A refusal rejects
  with `{ code, message, field? }`: when it names a `field`, show `message`
  under that field. Anything else oxplow records and shows under your
  frame by itself, so catch it and draw nothing (`alert()` is blocked in
  the frame anyway). A `CANCELLED` is the person closing a form or
  declining a confirmation: nothing to show. The kit has `ox-form`,
  `ox-field`, `ox-label`, `ox-input`, `ox-select`, `ox-textarea`,
  `ox-error`. A page's own `actions:` need no error handling at all.
- **Workflow state** no provider owns (a triage decision) lives in the
  extension's private state: a script command reads the stamp with
  `scope("state.read", …)` and composes your `state.write/compare_exchange`
  command with it; a read command returns it for a view to show. The
  triage example's TODO board does both.


## Registered settings

An extension's `configuration.schema` declares settings in the shared registry.
Fixed object fields become independent fully qualified keys, such as
`oxtask.tasks.workflow`; arrays and open maps are atomic settings. Put `title`,
`description` and `x-category` on fields for the Settings inventory, and give
your category a title in `configuration: categories:`
(`tasks: { title: Tasks, description: …, order: 15 }`; core's categories
`project`, `agents`, `ai`, … are core's to declare, a field may still name one). Optional
annotations are `x-ui`, `x-order`, `x-secret`, `x-mask` and `x-atomic`
(`x-mask: { prefix, suffix }`, on a secret only, is how many characters, 0 to 8, of each end an agent sees); any other `x-`
annotation is an error at `extension check`.
`x-ui` has one hint: `x-ui: { editor: multiline }` makes a string field a text
area. A choice comes from `enum`, or, to label each choice, from `oneOf` of
`{ const, title, description }` (the Settings editor shows the titles and the
chosen one's description; a page can read them from `v_config_value.schema`).
A secret comes from `x-secret`, Markdown from `contentMediaType: text/markdown`; `extension check` refuses anything else in
`x-ui`. Secret defaults are prohibited.

### Editing your settings on a page

A page lets a person edit your settings with the `settings` viz over your
keys. The label, control (a labeled choice, a number, a text area) and
validation come from your `configuration:` declaration, and a change is an
`oxplow.config.set`. No dataset, command, handler or form is needed:

```yaml
loaders:
  settings:
    sql: |
      SELECT * FROM v_config_value
      WHERE key IN ('oxtask.tasks.workflow', 'oxtask.tasks.definitionOfDone')
      ORDER BY setting_order, key
view:
  from: settings
  viz: settings
  settings: { level: project }   # every change is the project's
```

Leave `settings:` out to let the person choose the level per setting. Your
settings are also on Settings, under their category.

A command script reads a setting with
`scope("config.read", {"key": "oxtask.tasks.workflow"})` (declare
`needs: [config.read]`). The result has `key`, `value`, `origin`, `isSet` and
`secret`. Add `level: "local"`, `"project"` or `"global"` to read only that
override. Omit the level to resolve local → project → global → default.

To write, compose `oxplow.config.set` with `{key, value, level}` or
`oxplow.config.unset` with `{key, level}`; the level is always explicit. Your
extension's own settings need no `commands:` permission when a script
composes the write; a key of another extension, or a core one, is refused. A
custom component that invokes `oxplow.config.set`, or an `inverse` that
composes it, still needs `commands:oxplow.config.set`. Write
each independent field separately so changing one preserves the others.
Settings do not use revision stamps, compare-and-swap, or whole-namespace
writes. Read a setting with `scope("config.read", {key, level?})` in a script,
`config('/pointer')` in SQL, or `v_config_value` in a page. Private runtime state still uses the
`state.read` / `state.write` protocol above.

Ordinary overrides are flat fully qualified keys in `.oxplow/local.yaml`,
`.oxplow/project.yaml` and the user-global `global.yaml`. Secret settings use
secure storage behind the same API; inventory SQL exposes only presence and
origin. No secret value belongs in a page, an event or a command's result.
