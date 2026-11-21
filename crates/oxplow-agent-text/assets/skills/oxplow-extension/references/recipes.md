# Page recipes

Each recipe: write the file, check it, deploy, test.

## Data → page

1. Find the columns: `v_dataset`, `v_field`, `oxplow_query_sql`.
2. Write `.oxplow/extensions/<name>/pages/<slug>.yaml`:

   ```yaml page
   title: Open items
   params: [{ name: state, default: todo }]
   answers:
     - { question: "What work is open?", params: { state: todo } }
   loaders:
     items: { sql: "SELECT title, state FROM v_oxplow_task WHERE state = :state" }
   view: { viz: table, from: items }
   ```

   One loader, one table, one answer.
3. Check: `oxplow_validate_extension`, then **`oxplow_run_page { page:
   "<name>/<slug>", draft: true }`** in a worktree stream: it runs your
   draft over live data. From the main stream, `oxplow.extension.deploy`
   first, then `oxplow_run_page`.
4. Add a fixture (`testing.md` "Pages"): `fixtures/<name>.yaml` with
   `given:` steps (`{ command, input }` makes data), `when: { page: <slug> }`
   and `expect: { loaders: { items: { rows: [{ title: T }] } } }`. The
   file is the example: it runs with no `intent.examples` entry. `$OXPLOW_BIN extension test
   <name>` runs it and every `answers:` entry.
5. Deploy from the main stream (`development-cycle.md`), tell the person
   the title.

## A page with a form

```yaml page
title: New item
params: [{ name: t, default: "" }]
loaders:
  states: { rows: [{ value: ready, label: Ready }, { value: done, label: Done }] }
view:
  viz: form
  form:
    command: oxtask.task.create
    defaults: { title: "{{param.t}}" }
    choices: { stage: acme/new-item#states }
```

A `form` viz has no `from`. `command` is the (write) command the form
submits, as the person. `defaults` bind params with `{{param.x}}`;
`choices: { field: <namespace>/<query>[#<loader>] }` reads a query's rows (`value`,
`label`, optional `group`); `<namespace>/<query>` alone means its last loader. A list
two pages share is a query. Field and `fixed`
rules are in `commands-and-forms.md`. A form is not allowed in an inline
page.

## A page for one kind of thing (a resource page)

```yaml page
# pages/pr.yaml: what `oxplow:acme/pr?id=12` opens
title: Pull request
label: '{{row.title}}'
for: acme/pr
params: [{ name: uri }]            # the URI it shows
loaders:
  # `content` is first and yours to read: the row of the dataset that
  # resolves `acme/pr`, found by `uri`. Nothing to query for it.
  summary: { sql: "SELECT title, state FROM content" }
view: { viz: detail, from: summary }
```

A page with `for:` is a display mode of the resource: no `category`,
`keywords` or `badge`, and it is not listed. For a file, `content:
{ text: true }` adds the file's text. Test it with `temp:` and an `oxplow:temp`
file URI (`testing.md`).

## A dashboard

See `dashboards.md`: start one with `oxplow.extension.new_dashboard { title }`,
or write a page in the Dashboards category whose view is a `grid` of `viz`
and `embed` nodes with `oxplow.extension.write_page`. Test it with a fixture
whose `expect.nodes` lists the tile ids, and give it an `answers:` entry.

## A slot mount (add a section to another page)

Mount a part of yours into a page that has a `slot` node (foundation's
Commit, Uncommitted Changes, Diff View, Task, Tasks and New Session
pages have a `sections` slot):

```yaml part
# parts/stale-reviews.yaml
title: Stale reviews
params: [{ name: uri }]            # the params the host passes, by name
loaders:
  stale: { sql: "SELECT title FROM v_review WHERE item = :uri AND age_days > 7" }
view: { viz: list, from: stale, empty: No stale reviews. }
```

```yaml
# extension.yaml
ui:
  mounts:
    - { page: stale-reviews, at: oxplow/task-detail.sections, position: { after: links }, title: Stale reviews }
```

`slot: <namespace>/<page>.<name>` (the task page is oxplow-bundled's, so an extension that mounts there declares `requires: [oxplow-bundled]`); `at` is `start`, `end` (default) or `{
before | after: <sibling id> }`. The mounted page gets the host's params by
name, so declare the ones you need. Check the mount with
`oxplow_validate_extension`, and see it on the host page with `oxplow_run_page
{ page: oxplow/task-detail, draft: true }` (its text includes your
section, marked with your extension).

## Customize the change review

oxplow-bundled's **Review** page (`change-review`) shows one analysed
change. The one-line brief on each change page links to it. It has the
Signals, Tests and Code pages. A **signal** is one row: a thing to look at in
the change. Every signal is a row of the open dataset `oxplow/review_signal`
(`v_oxplow_review_signal`). The pages read `v_oxplow_review_signal_shown`,
which is the same rows with the project's severity (recipe "Rank the
signals"). A dataset with `adds_to:` needs no `requires:`. When
oxplow-bundled is off, it joins nothing and is no error.

### A review signal of your own

A dataset that says `adds_to: oxplow/review_signal` is one more source. Its
SQL selects the list's columns by name. A column it lacks is NULL.

```yaml
# extension.yaml (namespace: acme)
datasets:
  - name: review_signal_schema
    version: 1
    adds_to: oxplow/review_signal
    description: "A schema file the change touched."
    fields:
      - { name: change_id, type: "", doc: "The change (v_change.id)." }
      - { name: source, type: "", doc: "This dataset, acme/review_signal_schema." }
      - { name: area, type: "", doc: "other." }
      - { name: signal, type: "", doc: "schema-changed." }
      - { name: severity, type: "", doc: "medium." }
      - { name: path, type: "", doc: "The file." }
      - { name: title, type: "", doc: "One line for a list." }
      - { name: why, type: "", doc: "The evidence." }
```

```sql
-- datasets/review_signal_schema.sql
SELECT f.change_id, 'acme/review_signal_schema' AS source, 'other' AS area,
       'schema-changed' AS signal, 'medium' AS severity, f.path AS path,
       'Schema changed' AS title, f.path || ' is a schema file.' AS why
FROM ref('change_file') f
WHERE f.path LIKE 'db/schema/%'
```

- Columns: `change_id, source, area, signal, severity, path, line, end_line,
  title, why, uri`. `source` is `<namespace>/<dataset>`, the dataset's own name.
  `path` is repo-relative, or NULL for the whole change. `line` and `end_line`
  are on the head side. `uri` is what the row links to.
- `area` groups rows on the By Area table: `tests`, `verification`, `code`,
  `dependencies`, `history`, `scope` or `other`.
- `severity`: `high` and `medium` put the file in Look Here. `info` is
  counted and listed under its file only.
- `signal` is short and stable (`schema-changed`). The project ranks by it.
- The list's fields are untyped. If your dataset reads the list, declare the
  types `check` reports. Do not give your dataset a `key:` or a `tests:`.
- Test it with a fixture. `observe: { change: { kind: commit, target: c1,
  files: [{ path: db/schema/a.sql, additions: 2 }] } }` makes a change.
  Then `when: { sql: "SELECT signal, severity, path FROM
  v_oxplow_review_signal WHERE source = 'acme/review_signal_schema'" }` and
  `expect: { rows: [...] }`.

### A wider check

A signal that plain SQL over the stored analysis cannot give (a text search,
a parse) comes from a **check**: an entity collector on `change.analyzed`,
and a dataset that adds its rows. The full recipe is "A review check" in
`references/collectors.md`. In short:

- `on: [change.analyzed]`, `where: { phase: files }`, `sync: upsert`,
  `needs: [tree.read, sql.read]`.
- The script reads `change_diff(change_id)` (the hunks, with their added and
  removed lines), `grep_tree(pattern)`, `language_of(path)` and
  `code_metrics(text, language, path)`.
- Its tree is the change's **head**, not the worktree: `files()` and
  `grep_tree()` read the snapshot or commit the change is headed at.
- Key its rows by change. Return the keys it no longer makes in `deleted`.
- `v_oxplow_review_check` says if the check is `pending`, `failed` or `ok`
  for a change. `oxplow.review.unused` is oxplow's own check.

### A lint rule that reaches the review

A **fact collector** that records a `rule` (and a `severity`) at a `line`
gives a signal when the line is one the change touched. No dataset is needed:
oxplow-bundled's `review_signal_findings` reads the facts.

```yaml
collectors:
  - id: acme.no_print
    doc: "print calls left in Python files."
    runtime: starlark
    entry: collectors/no_print.star
    on: [snapshot.taken]
    facts: [oxplow.ast_hit]       # foundation's measure for "a hit of a rule"
```

```python
def transform(input):
    facts = []
    for f in files("**/*.py"):
        for i, text in enumerate(f["text"].split("\n")):
            if "print(" in text:
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "no_print",
                              "severity": "warning", "path": f["path"], "line": i + 1})
    return {"facts": facts}
```

- The review needs the hits' lines: a fact at a `line`, or a fact per file
  whose `detail` is `{"lines": [3, 17]}`. Prefer the second for a check that
  hits often (one fact per file keeps the metric cube small); a count with
  neither can't be matched to a changed line.
- The review shows `lint-<rule>`. `severity` is `error`, `warning`, `info`
  or `note` (another word fails the run): `error` is `high`, `warning`
  `medium`, `info` and `note` `info`; none is `medium`. A project can still
  set the kind's severity with `review.signals` (recipe "Rank the signals").
- Facts are as of the change's head snapshot. A change headed at a commit
  has no snapshot, so it gets no lint signal from facts.
- Test the collector with a `when: { collector: <id>, files: {...} }`
  fixture (`references/testing.md`). To test the signal, use `observe: {
  facts: { measure: oxplow.ast_hit, items: [{ value: 1, path, line, rule }] } }`
  after an `observe: { analyze: … }`, then read
  `v_oxplow_review_signal WHERE source = 'oxplow/review_signal_findings'`.

### Rank the signals

The setting `oxplow.review.signals` is a list. Each item gives a signal kind
a severity: `high`, `medium`, `info` or `off`. The last item for a kind wins.
It applies to every extension's signals.

```yaml
# .oxplow/project.yaml
oxplow.review.signals:
  - { signal: ci-changed, severity: info }
  - { signal: schema-changed, severity: high }
  - { signal: lint-no_print, severity: "off" }
```

Write `"off"` in quotes. Set it with `oxplow.config.set { key:
oxplow.review.signals, level: project, value: [...] }`. In a dataset,
`config('/review/signals')` reads it as JSON text.

### A section on the Review page

Use a section only for something that is not a signal. Mount a part at
`oxplow/change-review.sections`. The part gets `change_id`.

```yaml
# parts/schema-notes.yaml
title: Schema notes
params: [{ name: change_id }]
loaders:
  notes: { sql: "SELECT path, note FROM v_acme_schema_note WHERE change_id = CAST(:change_id AS INTEGER)" }
view: { viz: table, from: notes, empty: No notes. }
```

```yaml
# extension.yaml
requires: [oxplow-bundled]
ui:
  mounts:
    - { page: schema-notes, at: oxplow/change-review.sections, position: { after: look-here }, title: Schema notes }
```

The sections are `asked`, `look-here` and `by-area`. Cast `change_id`
to an integer in SQL.

## Add a language

A **language pack** is an extension. Its `languages:` entry names a language,
its files and its grammar. Core names no language. The pack's `namespace:` is
its own. Packs that ship with oxplow are `oxplow-<language>`. They are off by
default, except json, yaml and web. A project turns one on in Settings →
Extensions, or from the Alerts suggestion "Turn on the X language pack".

1. Scaffold: `$OXPLOW_BIN extension new extension my-lang`. Then add the
   `languages:` entry (`references/scaffolding.md` "A language pack").
2. Build the grammar: a tree-sitter grammar compiled to wasm with `$OXPLOW_BIN
   extension grammar build <name>`. Commit the `.wasm`. A language known only
   by its language server needs no grammar (`lsp: { package: <name> }`).
3. Write `languages/<id>/analysis.yaml`: the node kinds oxplow reads.
   `functions { kinds }`, `decisions { kinds }`, `visibility`, `doc`,
   `duplication`. `check` refuses a node kind the grammar does not have.
4. Write `languages/<id>/symbols.scm`: tree-sitter queries with
   `@definition.<kind>` captures and a `@name`.
5. Add `test_conventions:` (below).
6. Test with fixtures. A symbol fixture is `when: { language: <id>, path:
   <p>, body: "<source>" }` and `expect: { symbols: [{ kind, id }] }`. A
   fixture for complexity, a test file and the rest runs the same way.

```yaml
test_conventions:
  - id: mylang-tests
    languages: [mylang]
    paths: ['**/*_test.my']
    containers: ['*Test']        # a whole-name pattern: * ? [A-Z]
    functions: ['test_*']
    assertions: ['\bassert\w*\(']   # regular expressions
    skips: ['@skip\b']
```

A worked example: oxplow ships an Elixir pack as an example extension
(`$OXPLOW_BIN extension new --list-examples`, then `new --from elixir-pack`).
It is a third-party pack with its own namespace and a wasm grammar. It
has `forms:` analysis, a symbols file, ExUnit test conventions and
three fixtures. Copy it and change it.

Limits of `forms:` (for a grammar where `def` and `if` are generic calls, as
in Elixir or Clojure):

- `forms:` was written for Clojure. A call-based language gets no parameter
  count (it reads a vector of arguments).
- `visibility` knows only `defn-` and `^:private` here. Others are `unknown`.
- A doc is read from a string literal, not an attribute such as `@doc`.
- A decision kind cannot filter a `binary_operator` by its operator, so
  `and` and `or` add nothing.
- A grammar with a C++ scanner cannot be built.

A language needs a new core strategy when none of the named visibility, doc,
import or merge strategies fits. Say so to the person: that is a core change.

## A technology pack with no language

A database, CI or infrastructure pack has no grammar. It declares only what
oxplow reads from its files: `file_kinds:` (what a path is) and
`run_patterns:` (what a shell command is).

```yaml
manifest: 2
name: acme-postgres
namespace: acme
file_kinds:
  - { id: pg-migration, kind: migration, label: Migration, globs: ['db/migrate/**/*.sql'] }
  - { id: pg-config, kind: sensitive, label: Database config, globs: ['**/pg_hba.conf'] }
run_patterns:
  - { id: pg-test, kind: test, patterns: ['pg_prove', 'pgtap'] }
  - { id: pg-lint, kind: analysis, patterns: ['sqlfluff lint'] }
```

- `kind` of a file kind: `dependency-manifest`, `lockfile`, `test-harness`,
  `ci`, `lint-config`, `generated`, `vendored`, `migration`, `sensitive`, or
  any lowercase-dashed name. The review reads these kinds, so a changed
  `migration` file is a signal. `v_file_kind_rule` lists the rules.
- `globs`: `*` stays in a folder, `**` crosses folders.
- `run_patterns` `kind` is `test` or `analysis`. A pattern is a substring of
  a shell command. A shell call that matches is read as a test or analysis
  run.
- Give each entry an `id`. Without one, the id is the `kind`, and entries
  of extensions that share a namespace clash.
- Test it with a fixture: `when: { sql: "SELECT kind FROM v_file_kind_rule
  WHERE glob = 'db/migrate/**/*.sql'" }`.
