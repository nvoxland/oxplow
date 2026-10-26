# Commands, custom components and forms

- **A command on a URI's menus** (its page's Commands, a row's
  right-click) says so in its own `ui`: `ui: { label, group?, about:
  oxplow/work_item, input?: { uri: "{{uri}}" } }` (`about` is a resource
  name, `<extension>/<name>`; `"{{uri.id}}"` binds the URI's key param, such as
  the id, alone). To offer another command there with
  a fixed input, declare your own over the same operation.
- **A provider's own operations** are your commands too: `commands: [{
  name: item.estimate, summary, provider: <your provider id>, op:
  estimate, ui? }]` — the operation's input (plus an optional
  `instance`) and access come from its declarations; it runs on the
  instance its `uri` (or `instance`) names. Its capability verbs aren't
  declared: they run as `oxplow.work_item.<verb>`.

- **`commands:`** register commands of your own, as
  `<namespace>.<area>.<verb>`: `{ name: area.verb, summary, input_schema,
  entry: handlers/x.star, needs?: [sql.read], confirm?, access?,
  job?, examples? }`. The Starlark `transform(x)` gets `{ input }`,
  reads with `scope("sql.read", { "sql": "SELECT … WHERE uri = :uri",
  "params": { "uri": x["input"]["uri"] } })` (a list of row dicts; it must
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
  <part of the reason>`. An agent is shown an `example:` input beside the
  schema (a script's first example that runs, unless it declares one;
  give one to a command over a `scope:` op). `access: read` makes a command that only reads
  and returns a `result` (it composes nothing). A command on a URI's menu (`ui.about`) gets `{ uri }` unless its
  `ui.input` says otherwise, so name its input field `uri`. `extensions/oxplow-bundled/` (bundled) is a
  working example.
- **`custom_components:`** (stable; not in a bundled extension) are web bundles a
  `viz: custom` lens renders: `{ id, title?, bundle?: components/<id>,
  assets: [lens ids], commands: [names] }`. The bundle (an `index.html`
  and its files, at most 256 files / 5 MiB) runs sandboxed — no network,
  no storage, no token — and reaches oxplow only by asking the host to
  run one of its `assets` lenses, invoke one of its `commands` (a
  component that declares any runs them under the extension's
  permissions, approved when it was deployed; a command that asks
  still asks) or navigate. Its `index.html` loads
  oxplow's client library with a plain `<script
  src="/component-lib/oxplow-component.js">` before its own plain script
  (no modules: a sandboxed frame can't load them), and the script calls
  `oxplow.connect().then((component) => …)` for `component.data` (each
  loader's rows by name), `component.run` (a `viz: custom`'s own rows),
  `onUpdate(({ data, changed }) => …)`, `query`, `invoke` and `navigate`. `oxplow extension new
  component <name>` scaffolds a working one; `docs/guide/lenses.md` has
  the reference. A component written in TypeScript or with a library is
  built at deploy: a `build:` block, with `bundle:` under its `output`
  (`repair-and-sharing.md`).

## Forms

- **A form lens** (`viz: form`) is one command's input:
  `form: { command, defaults?, fixed?, fields? }`. `defaults` takes
  `{{param.x}}`; a `query` prefills from its first row; `fixed` fields
  show their default and can't be changed (an item's `uri`); `fields`
  says which to show.
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
- **Several forms on one page:** a document whose parts are form lenses;
  each submits its own command.
- **After a submit**, a command with `ui.open_after` opens that page;
  `then:` on the action (or `form.then`) says what becomes of the lens's
  page: `open` (default; Back returns), `replace` (it leaves history —
  a create form giving way to what it made), `stay` (nothing opens) or `close` (the page closes: it showed what the command removed).
- **A form action** on any lens: `form: true` on an action opens the
  command's form starting from its `input`, instead of running at once
  (a side panel; one text field alone is a prompt strip above the rows);
  `fixed` keeps fields to the bound values:
  `{ id: edit, label: Edit…, command: oxplow.work_item.update, row: true,
  form: true, fixed: [uri], input: { uri: "{{row.uri}}", title: "{{row.title}}" } }`.
- **Drag, bulk and key actions** (row actions): `drag: true` runs on a
  drop, binding `{{before.x}}` / `{{after.x}}` (the rows below / above
  where it landed; a missing one is left out), `{{onto.x}}`,
  `{{group.value}}` and `{{order.x}}` (a list: column `x` of every row in
  its new order, for a command that takes a whole order; only as a
  field's whole value) — `viz: board` with `group: { by, values }` drags
  cards between columns; `bulk: true` runs once per marked row; `key: f`
  runs on the cursor row. You run the command yourself; these are the
  person's gestures.
- **In a component**, a `<form>` is fine: handle `submit` in your script
  (`preventDefault`) and run the command with `invoke`. A refusal rejects
  with `{ code, message, field? }`; show `message` under the field it
  names. The kit has `ox-form`, `ox-field`, `ox-label`, `ox-input`,
  `ox-select`, `ox-textarea`, `ox-error`.
- **Workflow state** no provider owns (a triage decision) lives in the
  extension's private state: a script command reads the stamp with
  `scope("state.read", …)` and composes your `state.write/compare_exchange`
  command with it; a read command returns it for a view to show. The
  triage example's TODO board does both.
