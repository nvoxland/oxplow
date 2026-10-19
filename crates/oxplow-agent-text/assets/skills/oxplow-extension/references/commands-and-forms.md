# Commands, custom components and forms

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
  <part of the reason>`. An agent is shown an `example:` input beside the
  schema (a script's first example that runs, unless it declares one;
  give one to a command over a `scope:` op). `access: read` makes a command that only reads
  and returns a `result` (it composes nothing). A command on a ref's menu (`ui.about`) gets `{ ref }` unless its
  `ui.input` says otherwise, so name its input field `ref`. `extensions/oxplow-bundled/` (bundled) is a
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
  `oxplow.connect().then((component) => …)` for `component.run`,
  `onUpdate`, `query`, `invoke` and `navigate`. `oxplow extension new
  component <name>` scaffolds a working one; `docs/guide/lenses.md` has
  the reference.

## Forms

- **A form lens** (`viz: form`) is one command's input:
  `form: { command, defaults?, fixed?, fields? }`. `defaults` takes
  `{{param.x}}`; a `query` prefills from its first row; `fixed` fields
  show their default and can't be changed (an item's `ref`); `fields`
  says which to show.
- **After a submit**, a command with `ui.open_after` opens that page;
  `then:` on the action (or `form.then`) says what becomes of the lens's
  page: `open` (default; Back returns), `replace` (it leaves history —
  a create form giving way to what it made) or `stay` (nothing opens).
- **A form action** on any lens: `form: true` on an action opens the
  command's form (a side panel) starting from its `input`, instead of
  running at once; `fixed` keeps fields to the bound values:
  `{ id: edit, label: Edit…, command: oxplow.work_item.update, row: true,
  form: true, fixed: [ref], input: { ref: "{{row.ref}}", title: "{{row.title}}" } }`.
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
