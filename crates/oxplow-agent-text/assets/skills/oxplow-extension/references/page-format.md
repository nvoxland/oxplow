# Extensions and page files

## The files

In the worktree of the stream you're working in:

```
.oxplow/extensions/<name>/extension.yaml
.oxplow/extensions/<name>/pages/<slug>.yaml
.oxplow/extensions/<name>/fixtures/<example>.yaml
```

`<name>` and `<slug>` are lowercase-with-dashes; the page id is
`<name>/<slug>`. Reuse an extension when the page fits it; start a new one
for a new area. `oxplow.extension.write_page { extension, page, spec,
deploy? }` writes a page file (creating the extension), refusing a spec that
doesn't parse.

`extension.yaml` (manifest v2):

```yaml
manifest: 2
name: review            # must equal the folder name
description: Pages for reviewing agent work
intent:                 # required: what it is for, and how to know it works
  purpose: Show what is waiting on the reviewer in this stream
  origin: oxplow:oxplow/effort?id=eff42  # the effort you are working under, or null
  examples:
    - name: one blocked task
      input: { page: waiting-on-me, params: { stream_id: 1 } }
      expect: one row for the blocked task
ui:                     # optional: what it adds to other pages
  mounts:
    - { page: task-history, at: oxplow-foundation/work-item.sections }
panels:                 # optional: a left-nav panel
  - { id: waiting, title: Waiting on Me, icon: bell, scope: stream, body: waiting-on-me, badge: waiting-on-me }
```

A manifest without `manifest: 2` doesn't load. Unknown keys are errors;
`oxplow_validate_extension` reports each as `file:line: what — fix`. A
source with a `lenses/` folder (the old UI format) doesn't load: rewrite each file as a page.

## A page

```yaml
title: Failing tests
description: Test cases that failed in the last run.
params: [{ name: suite, default: all }]
answers:
  - { question: "Which tests are failing?", params: { suite: all } }
data:
  failing: { sql: "SELECT name, status FROM v_test_case WHERE status = 'failed'" }
view: { viz: table, from: failing }
```

`loaders.md` is `data:`; `view-nodes.md` is `view:`; `recipes.md` has worked
examples.

## Metadata

- `title` (required), `description`, `label` (the tab's name:
  `{{param.x}}`, `{{row.col}}` of the first loader's first row; filters
  `|short`, `|basename`, `|url`), `icon`, `category` (the launcher
  section: `Work`, `Code`, `Git`, `Activity`, `Knowledge`, `Data`,
  `Dashboards`, `System`; none means opened only from elsewhere),
  `keywords`.
- `hidden: true`: not in the launcher (a drill-down, an embedded page).
  `refresh: true`: a Refresh button, for pages reading outside oxplow.
  `comments: true`: the person may comment on its text. `needs:
  [capability]`: what it can't run without (it says so instead of failing).
- `tool: <name>`: also offered to agents as a typed MCP tool.
- `badge: { loader, column? }`: a count on its nav entry from that loader's
  first row (else its row count). `alert`: `{ min_rows: 1 }` or `{ column,
  below | above }`, with a `label`: marks the page as an alert while it fires.
- `rail:` a second node tree drawn in the page's rail (`view-nodes.md`).

## Params

`params: [{ name, label?, default?, type?, role?, options?, options_from?,
presets? }]`. A param binds by name into SQL (`:name`), command args
(`{ param: name }`) and `{{param.name}}` templates. Its value is the
caller's, else the viewer's context, else `default`. **`stream_id`,
`thread_id`, `effort_id` and `uri` are filled in from what the person is
looking at.** Don't hardcode ids.

- `type`: `text` (default), `choice` (`options: [{ value, label }]` with a
  `default` that is one of them, or `options_from: <page whose rows are
  value / label / optional group>`), `bool` (bound 1 or 0), `search`
  (trimmed text, NULL when empty), `branch` (a branch, NULL for all),
  `date_range` (a preset `1d 2d 3d 7d 30d 90d all` or `<from>..<to>`;
  `presets:` picks the quick ones). A `date_range` named `range` gives the
  query `:range_from` and `:range_to` (RFC 3339, NULL for an open end).
- `role` says where it rides in the page's URI. `resource` (default) names
  what the page shows (`?sha=…`; a change opens another resource). `filter`
  narrows how it shows it (`#…`; a change replaces the view; a `switch`'s
  param is a filter). Never write `role: selection`: a column's `select`
  makes it.
- Every param is drawn in the page's rail. Put a param on a page only when
  a person would change it.

## Renderers

`for: oxplow/commit` (or a list; your own `<extension>/<name>`, another
extension's, or `https`/`http`) makes the page the one that draws that kind
of URI; it has no launcher entry unless it has a `category`. `accepts: {
<param>: present | absent | <anchored pattern> }` limits it to URIs whose
params fit; `default: true` (only for a resource your extension declares)
makes it the page a plain open uses; `pinned: true` pins its tabs. Params
named like the URI's (`uri`, `sha`, `path`) are filled from it.

## `answers:` — how agents and tests find a page

```yaml
answers:
  - { question: "What did this commit change?", about: oxplow/commit, params: { sha: abc1234 } }
```

`oxplow_find_page { question }` matches these first (then titles and
descriptions), so write the questions a person would ask, in their words,
on one line. `about` is the kind of thing the page is about (a page for that
kind offers the question; "this" in the question means that thing).
`params` make the page answer it: `$OXPLOW_BIN extension test` runs every answer
and fails the page when it does not open or a loader fails (`testing.md`).
Give every page at least one. oxplow's own extensions also name a `skill`
that must name the models and commands the page reads.

## Whole-page actions

`actions: [{ id, label, command, input, confirm?, when? }]`: commands the
page offers, run as the page acting for whoever pressed it, so they can do
only what that person could. `{{param.x}}` reads a param; `{{row.x}}` and
`when:` read the **first loader's one row**. An action on a row (`row:
true`) belongs on the viz node that shows the row (`view-nodes.md`). An agent
presses one with `oxplow_run_page_action`; a command it may not run, or that
needs a person's confirmation, is refused.
