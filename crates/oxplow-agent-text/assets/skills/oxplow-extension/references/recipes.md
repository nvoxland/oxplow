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
     items: { sql: "SELECT title, state FROM v_work_item WHERE state = :state" }
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
  states: { rows: [{ value: todo, label: To do }, { value: done, label: Done }] }
view:
  viz: form
  form:
    command: oxplow.work_item.create
    defaults: { title: "{{param.t}}" }
    choices: { state: acme/new-item#states }
```

A `form` viz has no `from`. `command` is the (write) command the form
submits, as the person. `defaults` bind params with `{{param.x}}`;
`choices: { field: <ext>/<page>#<loader> }` reads a loader's value/label
rows (`<ext>/<page>` alone takes its first loader). Field and `fixed`
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
`hidden`, `keywords` or `badge`, and it is not listed. For a file, `content:
{ text: true }` adds the file's text. Test it with `temp:` and an `oxplow:temp`
file URI (`testing.md`).

## A dashboard

See `dashboards.md`: start one with `oxplow.extension.new_dashboard { title }`,
or write a page in the Dashboards category whose view is a `grid` of `viz`
and `embed` nodes with `oxplow.extension.write_page`. Test it with a fixture
whose `expect.nodes` lists the tile ids, and give it an `answers:` entry.

## A slot mount (add a section to another page)

Mount a page of yours into a page that has a `slot` node (foundation's
Commit, Uncommitted Changes, Diff View, Work Item, Tasks and New Session
pages have a `sections` slot):

```yaml page
# pages/stale-reviews.yaml
title: Stale reviews
params: [{ name: uri }]            # the params the host passes, by name
answers:
  - { question: "Which reviews of this work item are stale?", about: oxplow/work_item }
loaders:
  stale: { sql: "SELECT title FROM v_review WHERE item = :uri AND age_days > 7" }
view: { viz: list, from: stale, empty: No stale reviews. }
```

```yaml
# extension.yaml
ui:
  mounts:
    - { page: stale-reviews, at: oxplow-bundled/work-item.sections, position: { after: links }, title: Stale reviews }
```

`slot: <extension>/<page>.<name>` (the work item page is oxplow-bundled's, so an extension that mounts there declares `requires: [oxplow-bundled]`); `at` is `start`, `end` (default) or `{
before | after: <sibling id> }`. The mounted page gets the host's params by
name, so declare the ones you need. Check the mount with
`oxplow_validate_extension`, and see it on the host page with `oxplow_run_page
{ page: oxplow-bundled/work-item, draft: true }` (its text includes your
section, marked with your extension).
