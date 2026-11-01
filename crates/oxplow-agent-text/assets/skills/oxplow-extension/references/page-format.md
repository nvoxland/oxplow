# Extensions and page files

## The files

In the worktree of the stream you're working in:

```
.oxplow/extensions/<name>/extension.yaml
.oxplow/extensions/<name>/pages/<slug>.yaml      # a page: a tab, a URL
.oxplow/extensions/<name>/parts/<slug>.yaml      # a part: drawn only inside a host page
.oxplow/extensions/<name>/queries/<slug>.yaml    # a query: rows other pages read, no view
.oxplow/extensions/<name>/fixtures/<example>.yaml
```

`<name>` and `<slug>` are lowercase-with-dashes; the id is `<name>/<slug>`
whichever folder it is in, and a slug lives in one folder only. A part
takes no `category`, `keywords`, `label`, `icon`, `for`, `answers`, `tool`,
`see_also`, `rail`, `comments` or `refresh`; a query takes only `title`,
`description`, `params`, `loaders`, `tool` and `needs`, and its last loader
is what a reference to it means. Which kind do I write?

- A tab or launcher entry: a page (`pages/`).
- Drawn inside something else: a part (`parts/`).
- Rows only, read by two or more pages: a query (`queries/`).
- Rows only, for one page: a loader of that page.

Reuse an extension when the page fits it; start a new one
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
    - { page: task-history, at: oxplow-bundled/work-item.sections }   # also add requires: [oxplow-bundled]
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
loaders:
  failing: { sql: "SELECT name, status FROM v_test_case WHERE status = 'failed'" }
view: { viz: table, from: failing }
```

`loaders.md` is `loaders:`; `view-nodes.md` is `view:` (every section of a `stack`, `grid` or `switch` needs a `title`, and `hide_title: true` keeps its heading off the page); `recipes.md` has worked
examples.

## Metadata

- `title` (required), `description`, `label` (the tab's name:
  `{{param.x}}`, `{{row.col}}` of the first loader's first row (a `for:`
  page's first loader is the one you declare first, not `content`). The
  first loader is the first one declared, and a loader can read only
  loaders declared before it, so it cannot be moved after the ones it
  reads: have it return the column the label names, or check warns
  that `row.<col>` is not one of its columns. Filters
  `|short`, `|basename`, `|url`), `icon`, `category` (the launcher
  section: `Work`, `Code`, `Git`, `Activity`, `Knowledge`, `Data`,
  `Dashboards`, `System`; none means opened only from elsewhere),
  `keywords`. (A page with `for:` takes none of `category`,
  `keywords` or `badge`: see "Renderers".)
- `description` says what the page is: what it shows, for what, over which
  scope, in which order. A page whose purpose is an action (a form that
  files an item, hands off work or verifies a claim) may name it. Never
  list UI mechanics or the actions on offer: the page shows its own
  controls, and Definition in About this page lists them. Good: "The stream's closed
  threads, newest first, with the work each finished." Bad: "...
  Reopen puts one back on the rail; drag to reorder."
- A page with no `category` opens only from elsewhere (a link, a form, an agent). Something never opened on its own is a part or a query, not a page.
  `refresh: true`: a Refresh button, for pages reading outside oxplow.
  `comments: true`: the person may comment on its text. `needs:
  [capability]`: what it can't run without (it says so instead of failing).
- `tool: <name>`: also offered to agents as a typed MCP tool.
- `badge: { loader, column? }`: a count on its nav entry from that loader's
  first row (else its row count; on a part a panel names as `count` or `badge`, the header count). `alert`: `{ min_rows: 1 }` or `{ column,
  below | above }`, with a `label`: marks the page as an alert while it fires.
- `rail:` a second node tree drawn in the page's rail (`view-nodes.md`).
- `see_also:` related pages, drawn as "See also" in the page's rail after
  Contents: each entry is `{ page: <page id>, params? }` (a link titled with
  that page's title; `params` bind as an embed's) or `{ from: <loader> }` (a
  link per row, from its `title` and `uri` columns, `description` as the
  hover). Use it for pages about the same thing and the list a thing belongs
  to, not for the page's own content, and not for the page's other views of
  the same URI (the shell lists those below these links). A loader that fails or a page that is
  not there gives no link.

## Params

`params: [{ name, label?, default?, type?, role?, options?, options_from?,
presets? }]`. A param binds by name into SQL (`:name`), command args
(`{ param: name }`) and `{{param.name}}` templates. Its value is the
caller's, else the viewer's context, else `default`. **`stream_id`,
`thread_id`, `effort_id` and `uri` are filled in from what the person is
looking at.** Don't hardcode ids.

- `type`: `text` (default), `choice` (`options: [{ value, label }]` with a
  `default` that is one of them, or `options_from`: a query of `queries/`
  (`<query>`, its last loader, or `<query>#<loader>`) whose rows are
  `value`, `label`, optional `group` — or `'#<loader>'`, a loader of this
  page, for a list only this page uses), `bool` (bound 1 or 0), `search`
  (trimmed text, NULL when empty), `branch` (a branch, NULL for all),
  `date_range` (a preset `1d 2d 3d 7d 30d 90d all` or `<from>..<to>`;
  `presets:` picks the quick ones). A `date_range` named `range` gives the
  query `:range_from` and `:range_to` (RFC 3339, NULL for an open end).
- `role` says where it rides in the page's URI. `resource` (default) names
  what the page shows (`?sha=…`; a change opens another resource). `filter`
  narrows how it shows it (`#…`; a change replaces the view; a `switch`'s
  param is a filter). `view` is state the page or the system sets, not a
  person: it rides the `#` like a filter but the rail never draws it (a
  display-mode page, `for:`, may take one). Never write `role: selection`: a column's `select`
  makes it.
- Every param is drawn in the page's rail. Put a param on a page only when
  a person would change it.

## Renderers

`for: oxplow/commit` (or a list; your own `<extension>/<name>`, another
extension's, or `https`/`http`) makes the page a **resource page**: a
display mode of that kind of URI (a commit, a file), opened from the
resource and never listed on its own. So it takes no `category`,
`keywords` or `badge`, and it must declare `params: [{ name: uri }]` (the
URI it shows). A page with no `for:` is **standalone**: its own loaders,
listed by the launcher and search.

The system hands a resource page the record its URI names, as a loader
called `content` that you never declare. Read it as `FROM content` or `{
from: content.<col> }`; do not query for what the page renders. For a file
(`oxplow/file`) it is one row, `uri, path, filesystem, rev, kind, size,
modified_at, is_text, language`; for a resource a dataset resolves (a
commit, a work item) it is that dataset's row; for any other it is `{ uri }`.
`content: { text: true, max_bytes: 262144 }` in the page also reads a file's
text (`text`, `truncated`; `text` is NULL for a binary file; `max_bytes` at
most 4 MiB, default 64 KiB). No such file or row: `content` has no rows.
`content` is reserved: no loader or param of yours is named that, and a
standalone page can't read it. `accepts: {
<param>: present | absent | <anchored pattern> }` limits it to URIs whose
params fit; `default: true` (only for a resource your extension declares)
makes it the page a plain open uses; `pinned: true` pins its tabs. Params
named like the URI's (`uri`, `sha`, `path`) are filled from it.

A resource with both a data view and a definition view: make the data page
(a simple table of its records or facts) the `default: true` one and the
definition page the other mode. Don't link them to each other: the rail's
"See also" lists every other renderer of the tab's URI by itself, as
`View as <title>`. A page that is about the definition (a list of an
extension's contributions, a catalog) links to it as the same URI with
`#oxplow.renderer=<extension>/<page>`, pinning the definition renderer,
because the bare URI opens the data page.

`accepts: { content: <loader> }` also looks at what the URI names: the
loader (one of your page's own, or `content` itself) must yield at least
one row, or the page is not offered for that URI. Example: `accepts: { path:
'^[\w./-]+\.yaml$', content: is_page }` with `is_page: { sql: "SELECT 1 AS n
FROM content WHERE text LIKE '%loaders:%'" }` (and `content: { text: true }`).
The daemon decides this alone, and runs only that loader and what it reads:
`sql`, `rows`, a read-only `command`, `content`. Not a `page` loader (the load
refuses it). It has 2 seconds and 1 MiB; a loader that fails or is slow means
the page is not offered, so keep it cheap. `oxplow.page.renderers { uri }`
says which pages a URI is offered to.

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
that must name the datasets and commands the page reads.

## Whole-page actions

`actions: [{ id, label, command, input, confirm?, when? }]`: commands the
page offers, run as the page acting for whoever pressed it, so they can do
only what that person could. `{{param.x}}` reads a param; `{{row.x}}` and
`when:` read the **first loader's one row**. An action on a row (`row:
true`) belongs on the viz node that shows the row (`view-nodes.md`). An agent
presses one with `oxplow_run_page_action`; a command it may not run, or that
needs a person's confirmation, is refused.

An action can instead **open a URI**: `{ id, label, open: <column>, when? }`
(`command` or `open`, never both). It runs no command: `<column>` of the
first loader's one row (of the row, for a row action) holds the URI, and
pressing it opens that page, as a link does. Use `when: <column>` so it is
offered only when the column has a value (a CASE that is NULL otherwise). It
takes no `input`, `form`, `drag`, `bulk` or `edit`. An agent pressing it is
told `{ opened: <uri> }` and nothing runs. e.g. `{ id: open-current, label:
Open current version, open: current_uri, when: current_uri }`.
