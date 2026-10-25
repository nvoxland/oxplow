# Page recipes

Each recipe: write the file, check it, deploy, test.

## Data → page

1. Find the columns: `v_model`, `v_model_column`, `oxplow_query_sql`.
2. Write `.oxplow/extensions/<name>/pages/<slug>.yaml`:

   ```yaml page
   title: Open items
   params: [{ name: state, default: todo }]
   answers:
     - { question: "What work is open?", params: { state: todo } }
   data:
     items: { sql: "SELECT title, state FROM v_work_item WHERE state = :state" }
   view: { viz: table, from: items }
   ```

   One loader, one table, one answer.
3. Check: `oxplow_validate_extension`, then **`oxplow_run_page { page:
   "<name>/<slug>", draft: true }`** in a worktree stream: it runs your
   draft over live data. From the main stream, `oxplow.extension.deploy`
   first, then `oxplow_run_page`.
4. Add a fixture (`testing.md` "Pages"): `fixtures/<name>.yaml` with
   `given:` steps (`{ command, input }` makes data), `input: { page: <slug> }`
   and `expect: { loaders: { items: { rows: [{ title: T }] } } }`, and
   an `intent.examples` entry that names it. `oxplow_preview_extension`
   runs it and every `answers:` entry.
5. Deploy from the main stream (`development-cycle.md`), tell the person
   the title.

## A page with a form

```yaml page
title: New item
params: [{ name: t, default: "" }]
data:
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

## A dashboard page

See `dashboards-as-pages.md`: a `grid` of `viz` and `embed` nodes written
with `oxplow.extension.write_page`. Test it with a fixture whose
`expect.nodes` lists the tile ids, and give it an `answers:` entry.
