# Extensions: The page format

The page YAML: metadata, params, loaders, view nodes, actions, checks, custom component nodes, agents; plus change links and page layout. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Change links and page layout

- **Change links.** `diff-at` (`from` path, optional `line`, `base` and
  `head` columns holding the change's labels; join `v_change`) opens that
  file's diff between the two sides (commit shas / `HEAD` → git refs,
  `working tree` → disk, `snapshot N` → that snapshot, so an effort's
  change links too). `compare` takes
  a `path:start-end|peer:start-end` value (build it in SQL) plus an
  optional `head` version column and opens the side-by-side
  `duplicate-block` page, which is the general compare page (two `code`
  rows with `sync: true`).
  Both URIs carry their whole spec, so App's `handleOpenPage` and
  in-tab navigation register them directly.

- **A page keeps one shape whatever its data holds.** In a `stack`, every
  titled section stays, with or without rows: an empty viz draws its own
  `empty:` line under its heading ("No notices.", `viz-empty`), an
  embedded or mounted page the same under its title. Nothing folds into
  a closing "Nothing found" line and no section is left out for being
  empty, so the person finds each thing where it always is. Every titled
  section therefore carries an `empty:` worth reading (`empty: ''` is
  for a stand-in only: a hidden-title replaceable target). A whole-viz
  button that needs no rows (an Add, a New…) is drawn either way; one
  over every row it shows (`all: true`: a Clear, a Retry all) is offered
  only while there are rows (`offeredWholePage`). Those buttons close the
  section, under its rows, never over them (`PartButtons`).
  The effort page opens with its
  **verdict** above the slots: the `effort.verdict` replacement target
  (extensions-slots.md "Replacements"), which core leaves empty — what counts as tested,
  covered or backed is a review policy's. oxplow-bundled's
  `effort-verdict` page is the default (the last test run, diff coverage
  against the `oxplow.coverage.abs_pct` metric's `warnAt`, unverified
  claims, decisions to confirm); another extension may bring its own and
  the person chooses. The task page links to the effort page as
  **Review**: on each effort's activity card, and in the details rail for
  the newest effort.

## Pages

A **page** is a user- or agent-built way of looking at your work: named
SQL queries over the semantic layer, plus a tree of nodes that shows them.
(Not "view", which is taken by the SQL `v_*` views.) A page is one file in
an extension; its id is `<extension>/<slug>`. The extension manifest lists
no pages: the folders are the list. A source with the old `lenses/` folder
does not load.

**Three kinds, by folder** (`PageKind`, `pages/schema.rs`; one `Page`
struct with a `kind`, so the runner, the text rendering, open runs,
embeds and fixtures serve all three):

| Kind | Folder | What it is | Takes |
|---|---|---|---|
| **page** | `pages/` | a page with a tab and a URL; in the launcher when it has a `category` | everything below |
| **part** | `parts/` | drawn only inside a host — an `embed` node, a slot mount or replacement, a panel's body / collapsed / count / badge, a row's `expand`; no tab | `title`, `description`, `params`, `loaders`, `view`, `actions`, `alert`, `badge`, `needs` |
| **query** | `queries/` | rows other pages read — a param's `options_from`, a form's `choices` / field `from`, a `{ page, loader }` loader; no view | `title`, `description`, `params`, `loaders`, `tool`, `needs` |

A key a kind doesn't take is a load error naming what the file is and who
takes the key (`kind_problem`, the `KIND_KEYS` table: one row per key). A
query's loaded `view` is a `table` over its **last** loader, synthesized,
so a reference to it with no `#<loader>` means that loader
(`Page::default_loader`; a page's is its first). `Page::file()` is
`<folder>/<slug>.yaml`. A slug may live in one folder only: one in two is
neither (`page.slug_in_two_folders`, both dropped).

The reader's guide is `docs/guide/pages.md`; the agent's is the shipped
`oxplow-extension` skill (`references/page-format.md`, `loaders.md`,
`view-nodes.md`). `.context/pages-and-tabs.md` "Page files" covers how the
desktop draws one. This section is the contract: what a page file takes,
and what loads, runs and checks it.

**Three parts** (`pages/schema.rs`, `parse_page`; `deny_unknown_fields`, so
a typo is an error at the line):

- **Metadata**: `title` (required), `description`, `label`, `icon`,
  `category`, `keywords`, `params`, `for` / `accepts` / `default` /
  `pinned` (a renderer), `comments`, `refresh`, `needs`,
  `actions`, `alert`, `tool`, `answers`, `badge`, `see_also` (a part and a
  query take a subset: "Three kinds, by folder" above).
  A page's `description` says what the page is (what it shows, for
  what, over which scope and order), not what you can do on it. A page
  whose purpose is an action (a form to file an item, hand off work or
  verify a claim) may name that action; never list UI mechanics ("drag
  to reorder", "right-click for the rest") or the actions on offer.
- **`loaders:`**: named loaders ("Loaders" below).
- **`view:`**, and an optional `rail:`: a tree of nodes ("View nodes").

Where it lives, and what is loaded:

- **Origin** (`Page.origin`, `PageProvenance`): its `extension`, the
  `running` file (a deployment's copy in the store, or the source when
  loaded from it), the `deployment` hash, `bundled`, and `source` — the
  project-relative file to edit, none when no source is here (filled per
  extension by `locate`). An agent edits `source`, never `running`: the
  next deploy replaces the copy. `oxplow_get_open_page` adds `outOfDate`
  (`PageProvenance::out_of_date`, read from the files now: the source
  changed after the deployment was made). `v_page` has `source`, not the
  running path, and `running_uri` (V123; `PageProvenance.running_uri`, filled in `locate` by
  `filesystems::extensions::page_running_uri`, the one builder): the file URI of the running copy,
  `oxplow:oxplow/file?path=<extension>/pages/<file>&rev=<hash>&filesystem=oxplow:extensions`
  (built with `file_uri_at` from `origin.deployment`, so it pins the copy a
  redeploy replaces); NULL when the extension runs no deployment (read from
  its source, undeployed, not here). The same files are served by the
  `oxplow:extensions` filesystem (filesystems.md); `locate` and that
  filesystem both take the running hash from `Deployment::hash`. A deployed extension's errors and warnings name its source
  files, so they say what to edit.
- **Definition in About this page** opens the page's running file (`v_page.running_uri`)
  drawn as foundation's `definition` page, a display mode of a page's YAML
  file (`for: oxplow/file`, `accepts.content` over `oxplow.page.describe`):
  the page part by part (where it comes from, params, loaders, nodes,
  actions, answers), with its Source a link to the
  same running copy in the Text view (the raw YAML). A page with no running copy has no Definition row
  (pages-and-tabs.md).
- **An inline page** (`parse_inline_page`: a markdown ` ```oxplow-page `
  block) only reads: no `command` loaders, no actions, forms, terminals,
  editors, components or slots. An agent's `oxplow_show_page { spec }` is a
  **temp page** (`parse_temp_page`): the same rules, `for:` refused, kept as
  the page file `<id>.yaml` on `oxplow:temp` and run as `temp/<id>`.

### Metadata

- `label` is the tab's name: `{{param.x}}` and `{{row.col}}` (the first
  authored loader's first row, else `content`'s), filters `|short`, `|basename`, `|url`. Any other
  `{{…}}` is refused (`label_refs`).
  A label that shortens a path with `{{param.x|basename}}` (Editor, Directory, Diff) is
  also the tab's hover: the whole path (`tabHint`, `CenterTab.hint`).
  Check warns (`page.label_column`) when a `{{row.col}}` names a column the
  first loader's dry run doesn't return: a loader reads only the ones declared
  before it, so that loader can't move; it returns the column instead.
- `category` is the launcher section (`LauncherCategory`); none means the
  page opens only from elsewhere (a link, a form, an agent) — there is no
  `hidden` key: a page nothing should open on its own is a part or a
  query. `v_page.kind` says which; `search_page` indexes pages with a
  category.
  A page with `for:` is a resource page, a display mode of its resource:
  it takes none of `category`, `keywords`, `badge` (load
  errors) and must declare a `uri` param. It is handed a `content`
  loader first, configured by `content: { text, max_bytes }`
  ([pages-and-tabs.md](./pages-and-tabs.md) "Resource pages and standalone pages").
- `needs: [capability]`: a page whose needs are not met runs nothing.
  Its `PageRun.inactive` (`PageInactive { needs, message }`) says what it
  needs, and the page shows that instead of an error.
- `tool: <name>`: the page is also an MCP tool `<namespace>_<tool>`, its
  input the params, its call `oxplow.page.run`. Kept for the few questions
  agents ask every session (foundation's `thread_work`, `work_next`,
  `effort_summary`, `catalog`); everything else is `oxplow_find_page`,
  then `oxplow_run_page`.
- `answers: [{ question, about?, params, skill? }]` (`PageAnswer`): the
  questions the page answers. `oxplow_find_page` scores shared words:
  an answer's question x3 (the best one), the title x2, keywords and
  description x1; ties go by title; `limit` defaults to 10 (1 to 50).
  The tests are `find_*` in `agent_reads/page_tools.rs`. The
  pages of `about`'s resource offer them to the person; `oxplow extension
  test` opens the page with each answer's `params` and fails the page when
  it does not open or a loader fails. `skill` (oxplow's own extensions)
  names a shipped skill whose text must name what the page reads.
- `badge: { loader, column? }`: a count on the page's nav entry (that
  loader's first row, else its row count). `alert`: `{ min_rows }` or
  `{ column, below | above }`, with a `label`, and the page marks itself
  while it fires; it watches the badge's loader, else the first.
- `see_also: [...]`: related pages, drawn as "See also" in the page's Details
  rail after Contents (and ending `oxplow_run_page`'s text as `## See also`).
  An entry is `{ page: <page id>, params? }` (a link to that page, titled with
  its title; `params` bind as an embed's do: `{ param: x }`, `{ from:
  loader.col }` or a literal; a param the target page keeps in the `#` — a
  `filter`, `selection` or `view` one — goes there) or `{ from: <loader> }` (a
  link per row: `title` and `uri` columns, `description` as the hover). A
  `from:` loader the page doesn't have, a `page:` of its own extension that
  isn't in `pages/`, or a `from:` loader whose dry run returns no `title` or
  `uri` column is an error; another extension's page that isn't loaded is a
  warning. At run time a failed loader or an unknown page gives no link, never
  an error; links come in order, a uri once. A fixture's `expect.see_also`
  compares them. Use it for related pages and the list a thing belongs to
  (`.context/usability.md`). Do not declare the page's other views of the
  same item (`View as Data`, a link to the same `:uri` with
  `#oxplow.renderer=`): the rail's See also lists every other renderer of the
  tab's URI by itself, below the declared links. A link to a different
  resource stays declared.
- `for` / `accepts` / `default` / `pinned`: a renderer, see
  [pages-and-tabs.md](./pages-and-tabs.md) (`renderer_problem` checks
  them at load). `comments: true`: the person
  may comment on the page's text. `refresh: true`: a Refresh button.

### Params

A param (`PageParam`) takes `name`, `label`, `default`, `options`,
`options_from`, `type`, `presets` and `role`.

- **Types** (`ParamType`, `PageParam::take`): `text` (bound as-is; the
  default with no `options`), `choice` (one of `options`, or any text when
  `options_from` names a page that lists them; the default with either),
  `bool` (`true`/`false`/`1`/`0`, bound 1 or 0), `search` and `branch`
  (text, trimmed, NULL when empty; `branch` NULL means every branch), and
  `date_range`. A loaded page always says its type, so the desktop never
  infers it.
- **A `date_range`'s value** is one text: a preset (`1d 2d 3d 7d 30d 90d
  all`; `presets:` picks the quick picks, default `1d 7d 30d all`) or
  `<from>..<to>` of dates or RFC 3339 times, an end empty for open. Tab
  ids, `run.params` and agents see that one value. The runner binds
  `:<name>_from` / `:<name>_to` (RFC 3339, NULL for an open end; a date's
  `to` is the next day, so the day counts) just before a query runs, never
  as params.
- **Load checks** (`param_problem`): `options` or `options_from` off a
  choice; both on one; a choice with neither; a choice with fixed options
  and no `default`; `presets` off a date_range; an unknown preset; a
  default its type does not take; `role: selection` written; a param named
  like a view key (`RESERVED_VIEW_KEYS`: `section`). A run that supplies
  an unknown param, or a value its param does not take, is refused
  (`resolve_param_values`). Every param draws in the page's rail.
- **`role`** (`ParamRole`) says where its value rides in the page's URI.
  `resource` (the default) names what the page shows, so it is in the `?`
  and a change opens another resource. `filter` narrows or re-views what
  is shown, so it is in the `#` and a change replaces the entry; a
  `switch`'s param is a filter by default. `selection` is derived, never
  written: a column `select`s the param, the rail shows the picked row
  instead of an input, it is in the `#`, and a pick pushes an entry. `view`
  rides the `#` like a filter but is state the page or the system sets:
  the rail never draws it (not in Filters, not counted for Reset) and a
  person never edits it; a `for:` page may take one (`ParamRole::rides_view`
  is the one test for "rides the `#`"). A `filter`, `selection` or `view`
  param found in a `?` is refused.
- **`options_from`** makes a choice whose options rows list: `value`,
  `label`, optional `group`. It names a **query** (`<slug>` of the
  extension or `<extension>/<slug>`, its last loader; `…#<loader>` for
  another), or **a loader of the page itself**, `'#<loader>'` (a list only
  this page uses; qualified at load to `<page id>#<loader>`). Typed and
  checked at load (`pages/refs.rs`: a part or a page is refused; a loader
  the page lacks is named) and shaped by the dry run (`page.ref_shape`:
  the rows must have `value` and `label`, the error at the param's line).
  The daemon resolves it (`pages/options.rs::param_options`, IPC
  `page_options { page, param, params, stream_id }`): only that loader's
  chain runs, with the params the source declares, so an `end` picker's
  query can leave out what `start` is; the desktop's `OptionsSource.list`
  asks for the page and param and sends the page's values whole. A form's
  `choices` and field `from` resolve the same way (`source_rows`). A page's
  own options loader does not run in the page's run (below, "Running
  one"): only the rail's `page_options` and a form's open run it, and a
  fixture tests it as `when: { query: <page>#<loader> }`, `expect: { rows }`. No `default` is needed and none is
  checked; a run takes any text, and the query says what an unknown value
  means. An agent's page tool (`agent_tools::params_schema`) emits no
  `enum` for it, only words saying which query or loader lists them. A param named
  `uri`, `effort_id`, `thread_id` or `stream_id` draws as a picker over
  the matching dataset, by title (`viz/vizParams.ts`); any other is typed.
- **Implicit params.** A param named `stream_id` or `thread_id` is bound
  to the viewer's context (`ViewContext`, numeric ids) unless the caller
  supplies it. Precedence: supplied, then context, then `default`.
  `view_context(svc, stream?, thread?)` resolves it: the given stream
  (else the thread's, else the primary) and the given thread (else the
  stream's selected-or-first one). IPC `open_page` and `run_page_action`
  resolve it from their `stream_id`; MCP `oxplow_run_page` also takes the
  agent's `thread_id`; `oxplow_get_open_page` uses the viewer's thread.
  A dry run (a check, `validate`) uses no context.
- **`head_sha`.** A param so named is, unless supplied, the viewed
  stream's HEAD commit (`v_stream.head_sha`, `PageRunner::params_of`).
  The run reads `v_stream` for it (`context_reads`), so a commit or a
  branch switch re-runs the loaders that bind it.

### Loaders (`loaders:`)

Named loaders, run in order; a loader reads only the ones before it
(naming a later one is an error). A loader has exactly one of `sql`,
`command`, `page`, `rows` or `parse`, plus optional `columns` (keys, labels and
formats its nodes default to) and `empty`. Its name is lowercase letters,
digits and `_`, never a dataset's (`v_*`, `m_*`).

- `sql`: one read-only statement over the `v_*` datasets and the extension's
  own. `:param` binds a page param or a loader `params:` entry (`{ param
  }`, `{ from: <loader>.<column> }` (an earlier loader's first row) or a
  literal). An earlier loader it names reads as a temp table of that name
  (`SqlQuery.temp`). `config('/key')` reads the page's own extension's
  settings (`PageRunner::with_config`).
- `command`: a read-only command, run as the viewer
  (`Access::reads_only`). It says `reads` (datasets) or `refresh` (`manual`,
  `{ every }`). Its result is a `{ columns, rows }` table as is, an array
  of objects (a row each), else one `value` row; `items: <field>` takes a
  list inside the result as the rows.
- `page`: another page's loader (`{ page, loader, params }`). Pages nest 8
  deep (`MAX_DEPTH`). It may name another extension's page or query
  (`<ext>/<slug>`): a rule several pages share lives once, as a query, and
  each reads it — foundation's `thread-bookmarks` (the bookmarks a thread
  sees) is read by its Go To page and by oxplow-bundled's Go To panel.
- `rows`: inline rows, the first row's keys the columns.
- `parse`: one text as rows (`pages/parse.rs`): `{ parse: csv | tsv | json
  | yaml, text: <arg> }`, the text an arg (`{ from: content.text }` for the
  file a resource page shows, a param, a literal). A CSV or TSV is a table
  whose header names the columns (a blank header `column_<n>`, a repeat
  `<name>_<k>`; every cell text, a short record padded with nulls, a long
  one cut). A JSON or YAML document is a tree, a row per value in document
  order: `id` (its JSON pointer), `parent` (none at the top), `name` (key or
  index), `kind` (`object`, `array`, `string`, `number`, `boolean`,
  `null`), `value` (a scalar's text), `count` (a container's children); a
  scalar document is one row at `/`. A text that isn't the format is the
  loader's error. It reads nothing but its text, so an `accepts.content`
  chain may run it. Foundation's file pages over a CSV, a JSON or a YAML
  file are its users.
- `content` (never declared): a resource page's first loader, the record its
  `uri` names; read as `FROM content` / `{ from: content.<col> }`. The name
  is reserved: no loader or param is called `content`, and a standalone page
  can't read it.

**Running one** (`pages/runner.rs`, `PageRunner::run`): params resolve as
above. Loaders run in order. A loader that fails carries its `error` and
the rest of the page still shows. Within one run, embeds included, a
loader with the same definition and inputs (its `key`) runs once. A loader
only a `switch`'s hidden children show does not run; one no node shows
does (it feeds an embed, the badge or a later loader), except a list
only the page's own `options_from: '#x'`, a form's `choices` or a field's
`from` names (`on_demand_loaders`): no node shows it, no loader reads it, it
isn't the badge's, a `see_also` rows loader's, the alert's watched loader
or a column an embed takes. Such a loader runs when the list is asked for
(`page_options`, a form's open: `run_loader_chain`), so a slow list never
re-runs with the page. A page whose
`needs` are unmet runs no loader (`inactive`). A command loader runs as
the viewer, `Actor::Page { page_id, on_behalf_of }` (the page's URI is the
source of what it records). One run keeps at most 5 MiB of results
(`result_budget.rs`): later rows are cut whole, never half a row, and the
loader says so. Do filtering and `LIMIT` in SQL.

**Open runs** (`pages/open_runs.rs`): a window's open page keeps its run
and the request that made it. A change to a dataset one of its loaders read
(a command loader's `reads`), or to a param it binds (`head_sha` reads
`v_stream`), re-runs the loaders that depend on it (`dirty_loaders`).
`OxplowEvent::PageRunChanged` names the loaders whose rows differ (an
embedded page's as `<embed id>/<loader>`), and the window fetches only
those. A catalog change (a deploy) re-runs every open run
(`page_runs.notify_everything()`).

### View nodes

`view:` is one node, usually a `stack`. Every node has an `id` (default:
its loader, embedded page, slot or component; else `stack`, `stack-2`, …),
unique in the page. `#node=<id>` in a link lands on it. `title`,
`replaceable` and `size` are on any node. An embedded page's nodes are
`<embed-id>/<node-id>`; `oxplow_run_page` text prints each node under
`[node <id>]` (`pages/text.rs`).

- `stack: [nodes]`; `grid: [nodes]` (a child's `size`: `small`, `wide`,
  `tall`, `full`).
- `switch: { param, children }` shows the one child whose id is the
  param's value (else the first).
- `embed: { page, params }` shows another page inline. Its params flow by
  name: the embed's `params`, else the host's param of that name, else
  that column of the host loaders' first rows, in order. Its page actions
  run on the embedded run (`pages/actions.rs`, `locate`).
- `slot` and `component`: below. `markdown: "text"`: static text.
- `viz` (+ `from: <loader>`, a role block, `columns`, row `actions`,
  `empty`): a `VizConfig` with no query of its own. A viz that reads no
  rows (`form`, `approvals`) has no `from`. The YAML only binds; logic is
  SQL or a read command. `hide_header: true` leaves a table's header row
  out, for columns that explain themselves (`DataTable`'s `header`).
  There is no way to leave a section out for
  being empty (`hide_empty` is gone): it stays, over its `empty:` line,
  as a `form` over a loader with no rows does too ("A page keeps one
  shape"); a failed loader shows its error.
  `fold: true`: the rows start closed under a row count (a `<details>` the
  desktop draws around the viz); a click opens them. For a long list that is
  rarely read. A list row leaves out a cell with no value, not a dash.
  `lead: <column>`: the first row's text of that column is a plain paragraph
  under the node's heading and above its viz (what a thing is, above its
  facts); null or blank draws nothing. It is a column of the loader, checked
  in a dry run like a role block's; the desktop draws it in `VizNode`
  (`leadOf`) and `text.rs` writes it under the heading.

**Vizes** (`Viz`): `table`, `list`, `number`, `markdown` (the first row's
`markdown: { text: <column> }`, else its first column, drawn by the one
Markdown component, `.context/markdown.md`; `base: <column>` names the URI
the text came from, a file whose folder its relative links and images
resolve against; with an `edit` action it is
that component's editor, saving through the action), `bar`, `line`,
`treemap`, `tree`, `timeline`, `detail`, `sections` (a subsection per
row: **`sections`**, below), `steps`, `hunks`, `board` (the
groups as columns of cards, each a list row; needs `group`, whose
`values: [..]` fix the columns' order and keep empty ones as drop
targets), `terminal` (a terminal per row, `terminal: { of, title?,
stopped?, conversation? }`; `.context/terminal.md` "The terminal viz and
its host"), `editor` (the working tree's file, edited: `editor: { path,
lines? }`; saved through `oxplow.file.save`; `useTabEditing`;
`.context/editor-and-monaco.md` "The editor viz"), `embed` (a web page:
`embed: { url }`, the first row's `http(s)` URL, in oxplow's sandboxed
window; `.context/external-url-tabs.md`), `code` and `diff` (read-only:
a file at a revision, or one file's diff) and `media` (each row's file as
an image or a PDF, from its bytes: `media: { path, filesystem?, revision?
}`; `.context/editor-and-monaco.md` "Read-only code, diff and media
vizes"), `approvals` (core's consent component:
it reads `v_command_proposal` itself and decides through the approve and
decline commands; params `thread` and `setting` narrow the list), `form`
and `custom` (below). `terminal`, `editor`, `embed`, `form`, `approvals`
and `custom` are refused in an inline page. A viz that fills its page
(`code`, `diff`, `editor`, `terminal`, `embed`, `media`) draws no in-body title.

**`sections`** (`viz: sections`; `VizSections`, `SectionsViz.tsx`,
`sectionRows` in `vizModel.ts`) — a thing gets a section, not a row
(`.context/usability.md` "Pages are informational"). One subsection per
row, in row order: `sections: { title, id?, link?, body? }` (`title`
required; `link` a column holding a URI the heading's arrow opens; `body` a
column of markdown, drawn by the one Markdown component) and the viz's
`columns` as a short `Facts` list (the role columns are never facts; with
no `columns`, every other column is). `empty` and `when:` per row work as
for any viz. A subsection is a real section of the page, drawn by the viz
rather than made a `RunNode` per row: `kit/Section.tsx` chrome whose
heading level is one below the enclosing section (`SectionLevelContext`:
`h2` at the top, `h3` inside a section), kept when it has only a heading
(`keepEmpty`), and a `data-node` of `<viz node>/<row key>`. The key is the
`sections.id` column, else a column named `id`, else the title (a repeat
gets `-2`, `-3`) — so give a `sections` loader a stable `id`. Row `actions`
are in each subsection's right-click menu (and a `primary` one on click),
as for a `list` row. Contents lists subsections under their parent up to
`CONTENTS_SUBSECTIONS_MAX` (10; `viz/Contents.tsx`, the one constant): the
viz's root carries `data-sections=<rows>`; past ten, one "… N more" entry
scrolls to the parent (`railHas` counts every row, with `CONTENTS_FROM`).
The heading is never a link: a row's `link` is the `OpenPageLink` arrow
after it. `sections.group` / `group_link` group rows under a group section
one level above (each group container carries its own `data-sections`; Contents
caps per container, and skips what is under a group it left out). It adds no width:
subsections stay in the reading column. Agent text: a `###` heading (the
link in parentheses), the facts as `**label**: value` lines, then the body.
The Definition page's View outline reads it as "Sections of `loader`".

Each component names the columns it draws from in its own block: `chart`
(`x`, `y`, `series`, `label`, `size`, `group`, and a `line`'s `targets`: a
target line from that column's last non-null value), `tree` (`id`,
`parent`, `label`; lazy: `lazy`, `branch`), `timeline` (`at`, `label`,
`uri`), `steps` (`label`, `status`), `sections` (`title`; optional `id`,
`link`, `body`), `hunks` (`path`, `from`, `to` — two
revisions, `working` / `snap:<id>` / `git:<rev>`), `code` (`path`,
`revision`; optional `lines`, a column holding `12-40` or `12`, marked and
revealed, and `sync: true`), `diff` (`path`, `from`, `to`, optional
`lines` and `base`; the first row only). `role_columns` lists every
result column a block names; `extension check` and `validate` fail a page
whose loader does not return one (`viz_column_problems`). An agent reads
`code` as each file's lines in a fence and `diff` as its unified diff
(`viz_text`).

- **A lazy tree: `tree.lazy: <param>`, `tree.branch: <column>`.** A tree
  whose loader returns one level (a folder of `v_file`, `WHERE
  parent = :folder`): `lazy` names the page's own param the level is read
  with (it needs a `default`, the top level, and cannot be one the context
  supplies), `branch` the column that is truthy on rows that open. A
  branch opens by running the page again with the param set to the row's
  `id`, its rows spliced under it (`useLazyBranches`). A column whose
  `select` names the lazy param is refused (`lazy_tree_problem`).
- **A `form` node** is one command's input: `form: { command, defaults?,
  fixed?, fields?, mode?, choices?, then?, again? }`. `defaults` takes
  `{{param.x}}`; a `query`-less form prefills from its loader's first row.
  `fixed` fields show their `defaults` value, imposed again on submit.
  `fields` lists which to show, the rest keep their starting values; an
  entry is a name or `{ name, visible_if?, secret?, from? }`:
  `visible_if: { kind: [http, https] }` shows the field only while each
  named field holds one of the values (hidden, it is neither required nor
  submitted); `secret: true` masks it, keeps it out of any draft, and
  leaves it out of the input when blank. A command input field whose
  schema says `writeOnly: true` is masked without the flag
  ([commands.md](./commands.md) "Secrets"). The command must keep a secret
  out of its record (`unrecorded`), or the form refuses to open
  (`form_start`). `mode: edit` edits what is saved: it starts from the
  saved values, Save is enabled only when the form is changed, Discard (or
  Escape) puts them back, and it lists its `fields`. `create`, the
  default, gathers something new. `choices` maps a field to a page of the
  extension, run with the form's params, whose rows (`value`, `label`,
  optional `group`) are the field's choices. A field `{ name: native,
  from: <page> }` is an object whose children that page names, one row per
  child — `name`, `title`, `kind` (`enum`, `text`, `number`), `values`,
  `read_only` — and its value is the object of those children; the form
  reads again when what the page read changes (`viz_actions::subfields_from`).
  `again: { label, keep: [...] }` adds a second submit ("Save and
  Another"). `form_fields_problem` and `page_ref_problem` are the load
  checks.
- **`custom` and `component`.** `viz: custom` (`custom: { component,
  props? }`) and the `component: { id, props }` node draw one of the
  extension's `custom_components` in a sandboxed frame, fed the page's
  loaders. extensions-components.md "Custom components" below. A name the extension does not
  declare is a load error (`component_problem`).
- **A `slot` node** is a place other extensions mount pages at. See
  extensions-slots.md "Slots".
- **A section's `callout`, `collapsible` and `link`** (`Node`; `PageTree`,
  `kit/Section`). `callout: true` draws a panel, its title (with `icon:`, which only a
  callout takes) on the first line and its facts under it, a button that says
  `fact: <column>` on that fact's line (any `detail`'s)
  (`.ox-section[data-callout]`); one fact's label is left to the title, and
  with `hide_title` it is one line. `title_from: <column>` names it from its first row (`title` stands with
  no row; agents' text reads `title`). `collapsible: true` folds at the heading,
  remembered in the tab's history state (`PageTree` is the outermost
  `CollapsibleSections`). `link: <page id>` is the arrow beside the heading
  (`RefSite::SectionLink`: it must name a page).
- **A `rail:` tree** draws first in the page's rail, under its root's
  `title` as a group (bare without one): what the page is about, such as
  the branch a history is of. A row it picks sets the page's view param.

**Columns** (`PageColumn`): `key`, `label`, `link`, `unit`, `icon`,
`tone`, `width`, `format`, `highlight`, `select`, `expand`, `when`.

- `when: <column>` on a `detail`'s column draws the fact only while that
  column of the row is truthy (the same rule as an action's `when`): a
  merge's second parent on a merge alone. A null value with no `when`
  reads "(none)".

- `link: { kind, from, line?, base?, head? }` makes a cell a link
  (kinds: `task`, `file`, `wiki`, `effort-diff`, `commit`, `metric`,
  `page`, `diff-at`, `compare`). `page` opens any oxplow URI. The pages
  a column's links open are the page's **origin**
  (`viz/linkOrigin.ts`, `.context/pages-and-tabs.md` "Origins"): Up and
  Down step through the rows that have one.
- `select: <param>`: a click sets that param to the column's value, so
  whatever reads it re-runs (master and detail). It names a declared
  param (`highlight_problem`).
- `highlight: <param>` marks every case-insensitive occurrence of the
  param's value in the cell's text. `expand: <page>` puts a ▸ in the cell
  that opens that page under the row, with the row's same-named columns
  as its params (`page_ref_problem`).
- `unit` names another column of the row holding the unit, so a number
  shows as a metric value (`16.4 s`, `42.5%`).
- `icon` and `tone` each name another column of the row. `icon`'s value
  is an icon drawn before the cell from a fixed vocabulary
  (`viz/vizIcons.tsx`): the work glyphs (`ready` / `todo`,
  `in_progress`, `blocked`, `done`, `canceled`, `archived`), a file's
  status (`added`, `modified`, `deleted`, `renamed`, `untracked`,
  `conflicted`), the kinds (`epic`, `task`, `wiki`, `file`, `folder`,
  `commit`, `diff`, `metric`, `dashboard`, `comment`), the resource icon
  names (`extension_resources::ICONS`), and any page kind's icon. `tone`'s
  value is `accent`, `success`, `warning`, `danger` or `muted`. Values
  come from the query, so the loader cannot check them: an unknown name
  draws nothing, an unknown tone is plain.
- `width` is pixels (a table lays out fixed; cells are cut to one line).
  `format` (`ColumnFormat`): `date`, `datetime`, `relative`, `bytes`,
  `duration` (ms), `integer`, `percent` (a 0–1 fraction), `sparkline` (a
  JSON array of numbers), `badge` (a pill in the column's `tone`), `items` (a JSON array of
  strings or `{text, hint?, uri?}`, drawn inline separated by " · ", the
  hint on hover, the uri a link; an agent reads `text (hint) · text`). It
  changes only what a person sees; an agent reads the raw value (but for `items`).
- A timestamp cell shows in local time, its full time on hover.
- A `table` sizes its columns and stripes its rows with no page settings.
  The renderer picks each column's wrap from its `format` and the texts
  it shows (`columnWrap` in `vizModel.ts`): `ox-nowrap` for a date,
  number, `unit` or short (<= 20 chars, no space) values; `ox-words`
  (break only between words) when the longest word is <= 24 chars;
  `ox-breakable` (break anywhere) for long tokens such as URIs and paths,
  the column that gives way. Every other row of a table or group is
  `ox-stripe` (`--surface-row-stripe`); hover, selected, cursor and
  emphasis backgrounds win over it.

**Row styling** (`row_style_problem`), each naming a result column:
`group: { by, link?, values?, labels? }` (`list` / `table` / `board`) puts rows
under a heading per distinct `by` value, in the order each first appears,
folding (▾ / ▸) and counting its rows; `labels: { value: heading }` names a
heading in plain words while the value stays what a drop binds
(`in_progress: In Progress`); `emphasis` (`list` / `table` /
`tree`) highlights rows with a truthy value; `depth` (`list` / `table`) is
a whole number 0–8 that indents. They are never shown as cells.

Every `list`, `table` or `tree` row's menu has Ask the Assistant and Add
to Context (the first page it links to, else the row as text); a row drags
only where a drag action runs. A table or list is keyboard-driven (`useListNav`: ↑/↓,
Enter opens the cursor row's first link or runs its first row action;
usability.md "Keyboard").

### Actions

`actions:` on a page or a viz node: `{ id, label, command | open, input, row?,
group?, all?, form?, fixed?, fields?, about?, choices?, then?, when?, enabled?, fact?, primary?,
prominent?, submenu?, icon?, drag?, bulk?, key?, edit?, confirm? }` (`PageAction`).
`input` binds `{{param.x}}` and, on a row action, `{{row.x}}`
(`{{row.x|json}}`: the JSON a text cell holds, typed; the one filter,
checked at load). A page action with `when` or `{{row.*}}` and no `row`
reads the page's first loader's one row. A drag action also binds
`{{before.x}}`, `{{after.x}}`, `{{onto.x}}`, `{{group.value}}`, and
`{{order.x}}` (a list of column `x` in the new order).

- **`all: true` is a whole-viz action over every row the viz shows** (a
  Clear, an Enable All, a Retry all): the app offers its button only while
  the viz has rows (above). It is
  not a row, group, drag or edit action (`parse_actions` refuses the
  mix); the server runs it as any whole-viz action, rows or none, so the
  command must take an empty list.
- **`open: <column>` is an action that is not a command**: exactly one of
  `command` or `open` (`parse_actions` refuses both and neither). The column
  of the row the action is offered on (a whole-page action's first-loader
  row, a holds-one-row viz's, a row action's) holds the URI to open. Nothing
  runs: no bus run, no audit, no `command.executed`. The press still goes
  to the server (`viz_actions::prepare_resolved` → `Prepared::Open`), which
  checks the row and `when` as for a command, and answers `{ "opened": <uri> }`;
  the desktop then opens it (`performVizAction`, as `open_after` does) and
  an agent's `oxplow_run_page_action` reads it. An `open` action takes no
  `input`, `form`, `fixed`, `choices`, `drag`, `bulk`, `edit` or `group`.
  The extension check names a column the loader lacks (`when` and `open`
  alike, for page and viz actions). `PageAction.command` is nullable in the
  bindings; `actionDoes` (vizActions.ts) is what an action's title says
  (`opens <column>`). The Text view's "Open current version" is one:
  `open: current_uri` over its loader's column.
- An action runs as the page for whoever pressed it
  (`Actor::Page { page_id, on_behalf_of }`; `pages/actions.rs`,
  `viz_actions::prepare_resolved`), so it grants no power. A press names
  the open run (`run_id`), the node and the action; a row action names a
  row, and the server checks it is one of the node's rows. An agent
  presses one with `oxplow_run_page_action`; a command it may not run, or
  that needs a person's confirmation, is refused.
- `form: true` opens the command's form instead of running (the tab's
  modal, `TabModal`, or, when the form less its `fixed` fields is one text
  field, a prompt strip above the rows). What the person fills in runs,
  its `fixed` fields put back to the bound values. A refusal naming a
  field (`INVALID` with `field`) shows under that field. A run that came
  back undoable toasts with Undo. The form is for a person:
  - the heading and the submit are the action's `label` less a trailing
    `…`; Cancel and Escape close it; the submit is the primary button,
    off only while a required field is empty (its hover says which);
  - `fixed` fields are never drawn (no raw ids). What the form is about
    is its subtitle: a row action's `about: <column>` (the row's words),
    else, for a page action with `fixed` fields, the page's title;
    `about` is for a row action's form and the column must be in the
    result (checked at load);
  - `fields: [name | { name, label?, help? }]` (the same `FormField` as a
    `form` node's `fields`, minus `secret` and `from`) lists the only
    fields shown, in order; the others keep `input`'s value. Lookup order
    for a field's text: `label` here, else the schema `title`, else the
    humanized name; `help` here, else nothing — the command's schema
    `description` is for agents and is never shown. Every name must be an
    input of the command (`check_action_fields`, at load, when the
    registry can be asked);
  - non-`fixed` `input` values are the fields' starting values; a
    `choices` field shows each row's `label`, sends its `value`, and
    starts on the one `input` names.
- `primary: true` (a row action) is what a click on the row, or Enter,
  runs: the first one the row is offered (`when`). `prominent: true`
  keeps a page action a button in the nav bar; the rest go in the ☰ menu
  (`submenu` groups them). `bulk` runs once per marked row; `key` runs on
  the cursor row; `group: <value>` puts the button in that group's
  heading (it needs `group` on the viz and binds only `{{param.x}}`).
- An action's command may declare where a person goes after it
  (`ui.open_after`, [commands.md](./commands.md) "Offering a command to a
  person"): the page opens it once the action ran, bound from the run's
  result. What becomes of the acting page is `then`: `open` (default; Back
  returns), `replace` (it leaves history), `stay` (nothing opens) or
  `close`. Outside a page (a panel) a page opens in a new tab.
- `edit: true` makes an action the Save of a `markdown` viz's editor. Its
  one input field bound to `{{row.<text column>}}` and not `fixed` takes
  the text (`markdown_problem`).

### Checks

- **Load** (`parse_page`, then the extension's load): a page file's own
  errors at `file:line`; **every reference to another page file is typed**
  (`pages/refs.rs`: `RefSite` is the one table of places — a param's
  `options_from`, a form's `choices` and field `from`, a column's
  `expand`, an `embed`, a `{ page, loader }` loader, `see_also`, a panel's
  body / collapsed / count / badge, a slot mount, a replacement — each with
  the kind it takes: a query for options and fields, a part or a page for
  what is drawn inside a host, a page for a link, any for a borrowed
  loader; `for_each_ref` is the one traversal, so a new place is one
  variant). At load the target of each must exist in this extension and be
  the right kind (`page_ref_problem`, `parse_panels`, the mount and
  replacement checks all call `refs::kind_problem`; only a form's `choices`
  / `from` may name the page itself); once every extension is loaded, a
  target in another extension is checked the same way (`check_cross_refs`:
  the wrong kind is an error and the page is dropped; one no enabled
  extension has is a warning, `page.ref_not_loaded`, and shows nowhere). A
  `component` node or `viz: custom` names a declared `custom_components`
  entry (`component_problem`); a `replaceable:` node names a target in the
  table, placed once per page (`check_page`); a renderer's `for`,
  `accepts`, `default`, `pinned` and `label` fit its resources
  (`renderer_problem`). A page that fails is dropped, with its error on the
  extension.
- **Layout warnings** (`pages::checks::layout_warnings`): load, validate,
  `v_extension.warnings` and a deploy's result carry them, each naming
  its page file and the fix after the dash. A warning never stops a page
  loading.

  | Rule | Limit |
  |---|---|
  | `filter` params on a page (a `switch`'s do not count) | 6 — fold related ones into one `choice`, or split the page |
  | options of a `choice` | 12 — use a `search`, or a page that lists them |
  | a param no loader of the page reads (`:x` whole token, a date_range also `:x_from` / `:x_to`, an action's `{{param.x}}`, a column that `select`s or `highlight`s it, a form's defaults, the `label`); not checked on a page with an `embed`, `slot`, `component`, `replaceable` node or `viz: custom`, which pass the params on | none — remove it, or read it |
  | description of a listed page (one with a `category`) | 200 characters |
  | declared `columns` of a `table` / `list` (not on a page with `tool:`, whose rows are for an agent) | 8 — put the rest in a node a row `select`s |

- **`extension check` dry-runs pages** (`pages::checks::dry_run`): each
  `sql` loader runs once with the params' defaults (an earlier loader
  reads as a table), so a query that fails, and a viz node naming a column
  its loader does not return, are found before deploy. `command` and
  `page` loaders do not run (a check acts as no one), so a node they feed
  is not column-checked. `pages::checks::text` renders the dry run as
  text; the impact review compares it between two versions ("Reviewing by
  impact").
- A slot mount whose host page is not loaded, or has no slot by that
  name, warns (`"… shows nowhere"`).

### Custom components

A page's `component` node and `viz: custom` run a web bundle from the
extension's `custom_components`, sandboxed, fed the page's loaders. The
reference is extensions-components.md "Custom components" below and `docs/guide/pages.md`. A page
whose view is one `component` node fills the page under oxplow's bar
(a custom page; bundled's Timeline is the example).

### Agents

`oxplow_find_page { question }` (answers, then titles and descriptions),
`oxplow_run_page` (the page's text and rows; up to 200 rows a loader as
JSON, 50 as text, with `truncated`), `oxplow_run_page_action`,
`oxplow_show_page` (an inline `spec`, or an existing page with params,
beside the conversation), `oxplow_get_open_page` (the page's tree, the
viewer's filters and picks and the focused node). `oxplow_validate_extension`
checks a page before deploy, and `$OXPLOW_BIN extension test <name>` runs
its fixtures.
`v_page`, `v_page_answer` and `v_page_param` list the loaded pages
for SQL.
