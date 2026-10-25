# Page files

A **page** is `pages/<slug>.yaml` in an extension: what a person opens as
a tab. It has three parts: metadata, named **loaders** that fetch rows
(`data:`), and a tree of **nodes** that shows them (`view:`). The page id
is `<extension>/<slug>`. Unknown keys are errors; `oxplow_validate_extension`
reports each as `file:line: what — fix`.

More: `loaders.md` (the `data:` block), `view-nodes.md` (the `view:` tree),
`markdown-blocks.md` (an inline page in the wiki or `oxplow_show_page`),
`dashboards-as-pages.md`, `recipes.md` (worked examples). Lenses still
work and are in `lens-format.md`; write new things as pages.

```yaml page
title: Failing tests
description: Test cases that failed in the last run.
params: [{ name: suite, default: all }]
answers:
  - { question: "Which tests are failing?", params: { suite: all } }
data:
  failing: { sql: "SELECT name, status FROM v_test_case WHERE status = 'failed'" }
view: { viz: table, from: failing }
```

## Metadata

- `title` (required), `description`, `label` (the tab's name: `{{param.x}}`,
  `{{row.col}}` of the first loader's first row; filters `|short`,
  `|basename`, `|url`), `icon`, `category` (the launcher section: `Work`,
  `Code`, `Git`, `Activity`, `Knowledge`, `Data`, `Lenses`, `System`;
  none means it is opened only from elsewhere), `keywords` (words the
  launcher matches).
- `hidden: true`: not in the launcher (a drill-down). `refresh: true`
  gives the page a Refresh. `comments: true` allows comments on it.
  `needs: [capability]` names what it needs; without it the page says so.
- `tool: <name>`: also offered to agents as an MCP tool, prefixed with the
  extension's namespace.

## Params

`params: [{ name, label?, default?, options?, options_from?, type?, role? }]`.
A param is bound by name into SQL (`:name`), command args (`{ param: name }`)
and `{{param.name}}` templates. A value comes from the caller, else the
viewer's context (`stream_id`, `thread_id`, `effort_id`, `uri`, …), else
`default`.

- `type`: `text` (default), `choice` (`options: [{ value, label }]` or
  `options_from: <page or lens whose rows are value/label[/group]>`),
  `bool`, `date_range`, `branch`, `search`.
- `role` says where it rides in the page's URI. `resource` (the default)
  names what the page shows (`?sha=…`). `filter` narrows how it shows it
  (`#…`; a `switch`'s param is a filter). A `selection` follows from a
  column that selects it and is never written.

## Renderers

`for: oxplow/commit` (or a list) makes the page the one that shows that
kind of thing; `accepts: { sha: <pattern> }` and `default: true` choose
which page a URI opens, `pinned: true` pins it. Params that match the
kind's URI parts (`uri`, `sha`) are filled from the URI.

## `answers:` — how agents and tests find a page

```yaml
answers:
  - { question: "What did this commit change?", about: oxplow/commit, params: { sha: abc1234 }, skill: oxplow-codebase }
```

`oxplow_find_page { question }` matches these first, then titles and
descriptions, so write the questions a person would ask in their words.
`about` is the kind of thing the page is about (a page for that kind
offers the question, and `this` in the question means that thing). `params`
are values that make the page answer it: `oxplow extension test` runs
every answer and fails the page when it does not open or a loader fails.
`skill` (oxplow's own extensions): the skill whose text must name every
model and command the page's loaders read. Give every page at least one.

## `badge`, `alert`, `actions`

- `badge: { loader, column? }`: a count on the page's nav entry, from that
  loader's first row (`column`, else the first); otherwise its row count.
- `alert`: marks the page as an alert (lens `alert` keys in `lens-format.md`).
- `actions:` are **whole-page** actions, the same shape as a lens's
  (`lens-format.md` "actions"): `{ id, label, command, input, confirm?, when? }`.
  `{{param.x}}` reads a param; `{{row.x}}` and `when:` read the **first
  loader's one row**. An action that acts on a row (`row: true`) belongs
  on the viz node that shows the row (`view-nodes.md`). The command runs
  as the page acting for the person.
