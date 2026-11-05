# Extensions: How pages run today

What works today for pages: files, ids, reading from the live set, params, checks, surfaces (IPC and MCP), panels, actions and the desktop UI. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today

Code: `crates/oxplow-app/src/extension/` (the extension machinery: the
loader `extensions.rs`, activation, catalog, deploy, jobs, resources),
`crates/oxplow-app/src/pages/` (the page schema and spec types — `Viz`,
`VizConfig`, `ViewContext` in `spec.rs` —, runner, open runs, actions,
checks and text renderings), `crates/oxplow-rpc/src/commands/extensions.rs`
and `pages.rs` (IPC), and the page tools in `crates/oxplow-mcp/src/lib.rs`.
The authoring format, as agents learn it, is in the `oxplow-extension`
skill's `references/` (`page-format.md`, `loaders.md`, `view-nodes.md`);
the user guide is `docs/guide/pages.md`.

- **Files.**
  - `.oxplow/extensions/<name>/extension.yaml` is a **manifest v2**
    (`manifest: 2`, `name` = the folder, `intent`, the
    contribution kinds; see extensions-manifest.md "`extension.yaml`" below). It has no `pages:`
    key: pages are files.
  - `pages/<slug>.yaml` is one page: metadata (`title`, `description`,
    `label`, `icon`, `category`, `keywords`, `params`, `answers`, `actions`,
    `alert`, `badge`, `needs`, `for` / `accepts` / `default` / `pinned`,
    `comments`, `refresh`, `tool`), `loaders:` (named loaders: `sql`,
    `command`, `page`, `rows`) and `view:` (a tree of nodes, with an optional
    `rail:`). `parts/<slug>.yaml` is a part (drawn only inside a host) and
    `queries/<slug>.yaml` a query (rows only); each takes a subset of the
    keys (extensions-pages.md "Three kinds, by folder"). Details:
    [pages-and-tabs.md](./pages-and-tabs.md) "Page files".
  - Unknown keys are errors, so typos surface instead of being ignored.
- **Ids.** A page id is `<extension>/<slug>`.
- **Reading.** What the app shows is the **live set**: each extension's
  active deployment (extensions-deploy.md "Sources and deployments"), whichever stream is open:
  listings, pages, slots and panels, custom components, prompts,
  advisories, `oxplow_show_page` and `oxplow_get_open_page`. A stream's
  `stream_id` only scopes the data a page reads (`:stream_id` /
  `:thread_id`). Reads go through `Services.extension_catalog`
  (`crates/oxplow-app/src/extension/extension_catalog.rs`). `find_page`, the page
  tools and the advisory/metric/source readers take the catalog; the
  authoring checks (`oxplow_validate_extension`, preview) load the source
  instead. `ExtensionCatalog::snapshot` returns immutable declarations with
  their generation identity; `runtime_snapshot` is the last committed
  catalog plus immediate disable/removal revocation. The raw `snapshot` is
  the authoring/candidate view, with a staged change in it.
- **Params.** Values are bound as `:name`. Supplied values override
  defaults; for a param the viewer's context supplies (`stream_id`,
  `thread_id`) the context comes second. An unknown param name is an
  `Invalid` error, so a typo can't silently fall back to a default. A param
  with no default and no value binds NULL. `head_sha`, when a page declares
  it, is the viewed stream's HEAD: the run resolves it, and a change to
  `v_stream` (a commit, a branch switch) runs again the loaders that bind it.
- **Errors never cascade.** A missing or bad `extension.yaml`, or a folder
  name mismatch, marks that extension's `errors` and skips its pages. A
  bad page file is listed in `errors` and skipped; the other pages still
  load. A loader that fails shows its error and the rest of the page
  still shows.
- **Checks.** `oxplow_validate_extension` and `oxplow extension check` run
  the load checks and then dry-run every page (`pages::checks::dry_run`):
  each `sql` loader runs once with its params' defaults, earlier loaders
  as tables. They report SQL errors and viz nodes whose `columns` or role
  blocks (`link.from`, `chart.x`, `tree.parent`, …) name a column the
  loader doesn't return. `command` and `page` loaders do not run (a check
  acts as no one), so a node they feed is not column-checked.
  `pages::checks::layout_warnings` adds warnings, never errors: more than
  six filters, a `choice` with more than twelve options, a description
  over 200 characters, more than eight sections or eight table columns, a
  param nothing reads. A page in the launcher (one with a `category`)
  is also warned, at the param's line, for a param nothing
  fills in when it opens there (`launcher_unfilled_params`: no `default`,
  `options_from`, filter or selection role, and not the viewer's
  `stream_id`, `thread_id` or `head_sha`). A collector's `input` query is
  dry-run as a page's loaders and an advisory's query are, so one that
  reads a physical table (or doesn't compile) is an error at the
  collector's line. Load checks also cover an unknown `icon`, a page named by `options_from`,
  `expand` or a form field's `from` that is not in `pages/`, a `component`
  node or `viz: custom` naming a component the extension doesn't declare,
  a `replaceable` target that is not a replaceable component
  (`v_replaceable`), the renderer rules (below), and the params a panel gives
  its pages. A slot mount whose host page is not loaded, or has no such
  slot, is a warning on the mounting extension, and it shows nowhere.
- **Surfaces.**
  - IPC: `open_page` (starts a run a window keeps open), `run_page`,
    `get_page`, `page_text`, `page_options` (a choice's options, resolved
    for the page and param), `page_form`, `submit_page_form`,
    `run_page_action`. MCP: `oxplow_run_page`,
    `oxplow_find_page`, `oxplow_show_page`, `oxplow_run_page_action`,
    `oxplow_validate_extension`; an agent reads a
    page's definition with `oxplow_read`
    (`oxplow:oxplow/page?page=<ext>/<slug>`).
  - An agent lists pages, extensions and what each contributes with
    datasets, not tools: `v_page`, `v_page_answer`, `v_page_param`,
    `v_extension` and `v_contribution` (restated with each extension
    generation from the active catalog, the live set; an extension that
    fails to load holds its generation back, so it is not a row —
    `oxplow_validate_extension` says why). The UI's `list_extensions` stays
    IPC.
  - **Over MCP these are the authoring tools** and read the caller's
    **source**: an omitted `stream_id` is the caller's own stream
    (its header, else its thread's; the primary only for an anonymous
    caller), so an agent in a worktree can check the extension it just
    wrote before it deploys it — the same for
    `oxplow_run_page` with `draft: true` and `oxplow_ensure_change`.
  - **An agent tests an extension with the CLI, not a tool:**
    `$OXPLOW_BIN extension test <name> [--junit <file>]` from its own
    checkout (a worktree's draft as it is). `OXPLOW_BIN` is set for every
    agent session to the binary that launched it (the daemon's, which has
    the `extension` subcommands too, so it works on a remote host). The
    throwaway runs in that process, never in the app: a hang, a crash or
    its memory are the test's. It is a test run like any other — a
    `runPatterns` entry and a report collector for `--junit` put it in the
    Tests panel — and its programs see the agent's own environment, which
    the agent's shell already has.
    `oxplow_preview_collector` reads that stream's files and data but runs
    the deployed entity collector. `oxplow.extension.deploy` names no
    stream: it deploys the main stream's copy.
    `site_search` is the exception by design: omitted, it searches every
    stream; it indexes pages as well as the wiki.
  - **Over IPC (the UI)** they show the live set: `list_extensions`,
    `get_page`, `load_component` and `invoke_component_command` take no
    stream; `open_page`, `run_page` and `run_page_action` take one only for
    the page's data context. Checking an extension is the agent's
    `oxplow_validate_extension` (and the CLI's `check`), not an IPC read.
  - `oxplow_get_open_page` (an agent's) plus `report_open_page` (UI) are
    current; see extensions-commands.md "Agents: the MCP surface".
    Agents write page files with their normal Edit tool, under the filing
    guard, taught by the `oxplow-extension` skill.
- **UI.**
  - The desktop draws every page the same way: the page host
    (pages-and-tabs.md "The desktop host") runs the page, `PageTree` draws
    its node tree, and its viz nodes use the viz components
    (`apps/desktop/src/viz/`, entry `VizView.tsx`; the directory
    keeps the old name). A viz node draws a `table`, `list`, `number`
    (through `formatMetricValue`), `markdown` and the rest below. Linked cells go
    through `RouteLink`, so plain-click navigates in the tab.
  - The rail is the page's one place for controls (`PageRail`,
    usability.md "A page's controls are in its rail"): its filter params
    as `ParamRow`s (label beside value, click to edit in place or in a
    popover), its selection params as the picked row, and About this
    page (`origin`). A row shows what it applied at once, before the run
    echoes it. A row click on a column that `select`s a param sets it
    (`onSetParam`). Copy, Add to Context and the whole-page actions are
    handed to the nav bar: in its ☰, except a whole-page action the
    manifest marks `prominent: true`, which stays a button there (several
    may; a row, group, drag or edit action can't be — `parse_actions`). In
    the ☰ the page's actions sort by label; `submenu: <name>` puts a page
    action in that submenu (a prominent one can't be in one); the commands
    about the page's resource are not in a submenu, they sit among the
    actions at the top level, by label. Any action may name an `icon`
    from `extension_resources::ICONS` (the desktop's `src/icons.ts`): alone
    on its button in the bar, beside its label in a menu.
    **A page is a function of its tab entry** (`tabs/session.ts`): its
    params are its URI's (`oxplow:oxplow/page?page=<ext>/<slug>&k=v#f=v`): the
    `?` holds its `resource` params, the `#` its filters and picks. A change
    of a filter or pick goes back through `PageNavigation.setViewParams`
    with the values that differ from the defaults (`changedParams`, by
    type) — a filter replaces the entry, a selection pushes one — and a
    change of a `resource` param navigates to another resource
    (`withResourceParams`). So a reopened tab, a restart, a bookmark and
    Back show the same filters and the same pick. Outside a tab it keeps its
    params itself.
  - A run failure shows the error inline, with a nudge to ask an agent. A
    page whose `needs` are not met (a capability that is not active) runs no
    loader: its `PageRun.inactive` says what it needs, and the page shows
    that instead of failing.
  - Refresh is opt-in: a page offers it only when it says `refresh: true`.
    A page re-runs on its own when the datasets it read change, so Refresh is
    for one that reads outside oxplow (an integration's list, a file on
    disk).
  - The page re-runs when a dataset it read changed (`datasetsChanged`), when
    facts landed for a measure its `metric_grid()` read
    (`metricSamplesChanged`), when the live set changes, and after a
    reconnect (`onRemoteReconnect`): events sent while the daemon was
    unreachable never arrive, so anything may have changed. The daemon
    keeps each open run (`pages/open_runs.rs`) and tells the window which
    loaders changed (`PageRunChanged`); `useRerunOnChange` in
    `src/viz/rerun.ts` is the window's side.
  - A run whose tab is not on screen is paused. `CenterTabs` keeps every
    tab mounted and hides the others, so `PageHost` reads
    `useTabVisible()` and tells the daemon (`set_page_visible`) when none
    of a run's hosts is on screen. A change then marks the run stale and
    re-runs nothing; shown again, a stale run re-runs once and the
    window fetches it. A host that opens onto a paused run (a page drawn
    again by a new host while its run stays open) tells the daemon too,
    as a host shown again does. Rail panels and the selected tab stay live.
  - A window that goes quiet for `FORGET_AFTER_MS` (a background window's
    timers can stop) has its runs forgotten (`reap_expired_clients`). Its next
    `report_tabs` is answered with the `missingRuns` it still names, and it
    opens those again (`pageRuns.reopen`), or they would show their last rows
    for good.
  - The Cmd+P launcher re-reads pages (the live set's) every time it
    opens and lists them under their `category` (a page with none, a part
    and a query are not listed), merged into the static directory by
    category order (`mergeDirectory`).
  - `alert:` marks when a page needs attention: `{ min_rows: N }` (the
    watched loader returned at least N rows) or `{ column: c, above: X }` /
    `below: X` (the first row's value), with an optional `label`. The loader
    watches the `badge:` loader, else the first loader. Loading
    refuses an alert with both or neither condition. Every `PageRun` (IPC
    and MCP) carries `alert: {firing, count, value, message}`; the message
    is `label: count`, `N rows`, or `label: value`. `badge: { loader,
    column? }` is a count on the page's nav entry (the loader's first row's
    value, else its row count; Backlog's ready count).
  - **Pages** (`Extension.page_files`, `pages::schema::Page`; checked at
    load) — every page is a page file, oxplow's own included. Its tab id is
    the URI `oxplow:oxplow/page?page=<extension>/<slug>[&params]`, or
    `page=<slug>` for the extension whose `namespace:` is `oxplow`
    (oxplow-foundation: its pages are core-named as its commands are,
    `CORE_NAMESPACE`); `pageRef` carries it. `category` lists it in the
    launcher (none: a page only opened from elsewhere — a diff, a form);
    `keywords` are more words the launcher matches. `title` is its name (the
    launcher's, Open with's); `label` is what its tab says when that
    isn't the name, `{{param.<name>}}` filled from what it shows
    (`label: "{{param.path}}"`; `pages/renderers.ts` `tabLabel`; filled
    to nothing, the title) and `{{row.<column>}}` from the first row of the
    page's first loader. The tab says the page's title until that loader
    has run (`PageHost`: `tabLabel` over `labelRow` → `usePageTitle`), then
    the filled label; a loader that fails keeps the title. Either may take
    filters: `{{param.sha|short}}` (the first 7 characters), `|basename` (the
    last path segment) and `|url` (a web page's host and path, cut to 40),
    refused if unknown. A renderer's label names params of what it renders
    (else a load error), a route's its `params`. A page's `icon` is one of
    the resource icons (`ICONS`; an unknown one is a load error); the tab
    strip, launcher, Go To lists and backlinks draw it (`PageKindIcon` by
    the page's id). A **custom page** is a page whose `view:` is one
    `component` node: the extension's own `custom_components` entry fills the
    page body under core's nav bar, told the page's view params, history
    state, stream and thread in `init.page`, setting its title and actions as
    data core draws (extensions-components.md "Custom components"). How the desktop draws a page:
    pages-and-tabs.md "Declared pages".
    **A page with `for` is a renderer** of a resource; one without is a
    route (`oxplow:oxplow/page?page=…`). A URI opens as itself and the
    desktop picks among its renderers (pages-and-tabs.md "Renderers").
    The rules, checked at load (`renderer_problem`, after `resources:` is
    parsed):
    - `for: <resource> | [<resource>, …]` names a resource of the
      extension's own, one of oxplow-foundation's (`oxplow/*`), another
      extension's, or the scheme `https` / `http` (a web page).
    - `accepts: { <param>: present | absent | <pattern> }` keys are the
      resource's params (any key when the resource takes `additional`
      params); a value is `present`, `absent` or an anchored id pattern
      (the `param_pattern` subset). A renderer shows a URI only when every
      rule holds.
    - `accepts: { content: <loader> }` (the key `content`, not a param)
      names one of the page's own loaders (or the system's `content`); the
      page is offered for a URI only if that loader yields a row. It is a
      load error if the name isn't a loader of the page, if it (or what
      it reads) is a `page` loader, or on a page with no `for`. The daemon
      runs only that loader's dependency chain (`sql`, `rows`, a read-only
      `command`, `content`) with 2 s and 1 MiB; a failure is not offered.
      Which pages draw a URI is `pages/renderers.rs`, asked by
      `oxplow.page.renderers { uri }` (pages-and-tabs.md "Renderers").
    - `default: true` is only for a resource the extension declares (or,
      for oxplow's namespace, a scheme), and one page per resource. An
      extension that renders another's resource adds a renderer to pick
      ("Open with"), never the default.
    - `pinned: true` makes its tabs pinned.
    - `comments: true` makes the page's text take comments: its URI (its
      tab id) is their target, the nav bar gets the comment navigator, and
      a selection in its text starts one — plain text and the markdown
      viewer through the app's DOM layer (the body carries `data-ref`),
      the markdown editor and a `code` viz through their own layers
      (usability.md "Comments are a page's"). Comments name one URI, so
      `true` is the whole declaration.
    - `default`, `pinned` and `accepts` without `for` are a load error.
    - A page with `for` takes no `category`, `keywords` or `badge`,
      declares `params: [{ name: uri }]`, and is handed the `content` loader
      (above, Metadata and Loaders).
    - A renderer is run with the params of the URI it draws: a param it
      declares that the URI carries takes the URI's value (supplied → URI
      → context → default), so a `path` param of an `oxplow/dir` renderer
      is the URI's `path`. Its resource params besides `uri`, `stream_id`
      and `thread_id` must be params of each resource it renders (a load
      error otherwise: it would never get them); its filters ride in the
      `#` as any page's. Its rail doesn't draw `uri` (the page is the
      URI), and another value of a URI param opens that URI
      (`withRenderedParams`, pushed).
  - **Panels**: `panels: [{ id, title, icon?, scope: project | stream |
    thread, body, badge?, open?, collapsed?, count?, dock? }]` put a page in
    the left nav (with `dock: true` it goes in the bottom dock instead: an
    icon with the count as its badge that opens the body above the bar,
    and the rail does not draw it). `body`, `badge`, `collapsed` and
    `count` name **pages of the same extension, by slug**. `open`, a
    `page` URI, is the page its header opens — a core page it summarizes,
    as Comments opens the inbox; absent, the body page. `collapsed` is a
    page shown compact while the panel is collapsed, as its summary
    (absent, a collapsed panel is just its header). `count` is a page whose
    first cell (else row count) is the header's count, raising no alert; with
    a `badge` too, the count page's number shows and the badge still feeds
    Alerts (`panelCount`). (`Extension.panels`, `ExtensionPanel`; checked at
    load in `parse_panels`: a valid, unique id; pages that exist — body,
    badge, collapsed, count — a badge page with a `badge` or an `alert`,
    and a `stream` / `thread` scope's pages declaring `stream_id` /
    `thread_id`). The nav binds those itself (`panelParams`: the stream's and
    thread's row ids for the stream and thread it's shown for, re-run when
    they change) rather than leaving the backend to infer the thread from
    the selection, and runs every panel's pages from one owner
    (`useExtensionPanelRuns`), each distinct page once per refresh whatever
    roles it plays. The body renders compact; the badge's alert count shows
    on the panel, and the **Alerts** panel and page (the dock's bell) list
    every firing badge. oxplow-bundled's Waiting on You is a panel whose
    badge is its own page. A page dropped by `public_contracts` takes the
    panel parts that name it with it.
  - **Actions are commands**: `actions:` on a page declares
    `{ id, label, command, input?, … }` — a button in the nav bar or its ☰
    menu; on a viz node, with `row: true`, an item in each row's menu —
    every row of every row component (table, list, tree, timeline, detail,
    steps, hunks, board) is focusable and opens it from a right-click or the
    Menu key / Shift+F10. A command that asks shows `CommandConfirm` from
    the page itself, above the body, so it asks wherever the row is, not only
    where the toolbar is shown. An embed node's page actions run on the
    **embedded run**: pressing one on an embedded page acts on that page's
    loaders (`<embed id>/<node id>` names the node).
    - `input` is the command's input. A string that is exactly
      `{{param.<name>}}` or `{{row.<column>}}` becomes that value, typed
      (a number stays a number); a string containing them has them spliced
      in as text. `{{param.<name>.id}}` is the key of a URI param, for a
      field that takes a bare id. The values go into the command's input,
      never into SQL. One tokenizer reads them (`oxplow_domain::template` —
      every template's, a command's `ui.input` and `open_after` too), for
      load-time validation and run-time binding alike, so the two can't
      disagree. At load, a `{{param.x}}` must name a declared param and
      `{{row.x}}` needs `row: true` — or a node that holds one row (detail,
      form, markdown), where an action may name `{{row.x}}` and `when:` too
      and means that row; a whole-page action's `{{row.x}}` means the first
      loader's one row. `oxplow_validate_extension` checks `{{row.x}}` and
      `when` against the loader's columns; the command name must be
      well-formed. Any other shape is a field error.
    - **An action over a one-row result** (`PageAction::one_row`) is
      derived at load, not declared: an action without `row: true` that
      names `when:` or `{{row.*}}`, on a page or node that holds one row. The
      server reads the row itself (`pages::actions::page_row`, and the node's
      row for a viz node): it refuses unless the loader has exactly one row
      ("its first loader has N rows") and refuses a `row` the caller passes,
      then binds and checks `when` as for a row action. A person's press, a
      form's prefill (a Rename… starting from the name) and
      `oxplow_run_page_action` are the one path. The window shows such a
      button only while the result has exactly one row and `when` holds
      (`offeredWholePage`, the one predicate the toolbar and the page's
      hand-up share).
    - **Drag, bulk and key actions.** `drag: true` makes a row action
      what a drop runs (`rowInteract.ts`, `RowsBody`): a row the action is
      offered on (`when`) drags (`VIZ_ROW_DRAG_MIME`, taken only by the view
      it came from; a marked row carries every marked row), and where it
      lands binds `{{before.*}}` (the row below the drop), `{{after.*}}`
      (the row above), `{{onto.*}}` (the row dropped on: its middle third,
      offered only when some drag action names it), `{{group.value}}`
      (the group a drop lands in: among its rows, on its heading, on a board
      column) and `{{order.*}}`: that column of **every row of the result**
      (not only the drawn ones) in its order after the drop, the dragged
      rows moved before the row below, else after the row above, else to
      the end (`withOrder`; a drop onto a row moves nothing), so a command
      that takes a whole new order gets one. `{{order.*}}` binds a list, so
      it is only a field's whole value (spliced into text or put in a list
      it is an error), and a drop always has one (`Drop.order`; without it
      the field is an error, never left out). A field that is exactly one of
      the others with nothing there is **left out** of the input (a drop at a
      list's end has no row below), so a command's optional field stays
      absent; one spliced into text, or in a list, with nothing there is an
      error. Every drop scope needs `drag: true`, and its column must be in
      the result. Several drag actions may be declared: a drop runs the first
      the row is offered whose places are all there (one naming the group
      needs a group; one naming `onto` needs a row dropped on; one naming the
      order needs it). Rows dropped together run it once each, in drawing
      order, each after the one before it. The press carries the `drop`
      (`viz_actions::Drop`); a drop on another action, or a drag action
      pressed without one, is `Invalid` at `/drop`. `bulk: true` puts a row
      action in the **marks bar** (which stands in for the page's buttons
      while rows are marked, the page's nav-bar actions too): it runs once
      per marked row it is offered, in order, asking once and toasting once.
      `key: <k>` (one character, or `Delete` / `Backspace`; never one the
      row list keeps — Enter, Escape, Space, Tab, the arrows, Home, End,
      Page Up/Down, the menu key — nor one taken twice in a node) runs a row
      action on the cursor row, or on the marked rows when it is `bulk`. The
      marks themselves are the kit's (usability.md "Rows: marks, drags and
      keys"), kept per node in the entry's history state.
    - **Edit actions.** `edit: true` makes an action the Save of a
      `markdown` viz's editor: a form over the viz's first row (`row` and
      `form` implied). Its **edited field** is the one input field bound
      exactly to a `{{row.<column>}}` and not `fixed`
      (`PageAction.edit_field`, `extensions::edit_column`); Save runs it
      with `{ <field>: <the text> }` as the form's input, laid over the
      bound input as any form's is. The viz must name that column as its
      text (`markdown: { text: <column> }`). Refused at load: no such
      field or several ("fix all but one"), `edit` with `drag`, `bulk`,
      `key`, `group` or `primary`, two edit actions on a node, an edit
      action off a `markdown` viz, a `markdown` text other than the one
      it saves.
    - **`when: <column>`** (row actions only): the action is offered only on
      rows where that column is truthy (not null, 0, `""` or false) — the
      row menu leaves it out elsewhere (`actionOffered`), and a run on such
      a row is refused. `oxplow_validate_extension` checks the column is in
      the result. The query computes the condition (`can_enable`).
    - **A page grants no power.** `pages::actions::run_page_action` runs the
      command through the bus as `Actor::Page { page_id, on_behalf_of }`
      (`page_id` is `<extension>/<page>`, with `#<node>` for a node's
      action) — `Human` from the UI (`run_page_action` RPC), the calling
      agent from MCP `oxplow_run_page_action` (which never confirms). The
      bus applies the agent policy through `on_behalf_of`. A command that
      asks (`confirm`) asks only a person; a page action an agent pressed
      runs without asking. The audit records the page's URI as its source
      (V114 added the `page` actor kind).
    - **The command runs after the read.** An action, a form submit and
      a component's invoke are resolved under one pinned catalog
      generation (`viz_actions::Prepared`: the page, its `when`, the
      bound input, the bundle's commands), and the command runs once
      that pin is released. Activation waits for every reader of the
      generation it replaces, so a command that activates (the
      Extensions page's Deploy, Disable, Remove) would wait on its own
      pin and is refused with "activation cannot wait on its own
      reader".
    - A command that asks (`NEEDS_CONFIRMATION`, now kept on the thrown
      `IpcCallError`'s `code`) shows a confirm strip in the toolbar: Run
      (focused, Enter) or Cancel (Escape).
    - **Every page has Copy** (its text rendering, the `page_text` RPC, from
      `viz_text.rs` and `pages::text`) **and Add to Context**
      (`[oxplow page <id> …]` into the agent's input, never sent) —
      kit affordances, not declared. A compact strip (a panel body, an
      embed) shows no toolbar.
    - Running a collector is the command **`oxplow.collector.sync { collector }`**
      (External; `collector_runner::CollectorRunner::sync`,
      which the scheduler uses too). It **never grants**: what a collector
      may reach is its extension's `permissions:`, approved at deploy
      (extensions-permissions.md "Permissions"). An agent's `run_command oxplow.collector.sync` runs it as the
      agent.
    - UI: the row menu and buttons in `VizView.tsx`, pressed
      on the page run (`run_page_action`); logic in `viz/vizActions.ts`
      (`performVizAction`, `rowRecord`). Copy is the page host's
      (`page_text`), Add to Context the ☰'s.
    - The GitHub example's PR page has a Sync PRs action
      (`command: oxplow.collector.sync`).
  - **Viz** (`VizView.tsx`; the `Viz` enum and `VizConfig` in
    `pages/spec.rs`): `table`, `list`, `number`, `markdown`, `bar`
    (`DailyBarChart`), `line` (`components/charts/TrendChart`, one chart per
    `chart.series`), `treemap` (two-level `components/charts/squarify`),
    `board`, and the structure components: `tree` (collapsible nesting by
    `tree.parent`; orphans are roots, a cycle is cut; `lazy` and `branch`
    read one level per param value), `timeline` (oldest first, linked
    through `timeline.uri`), `detail` (the first row as label/value pairs),
    `sections` (a subsection per row — see **`sections`** below),
    `steps` (a checklist by `steps.status`: done, active, failed, else
    pending) and `hunks` (each row's file diffed between `hunks.from` and
    `hunks.to` in the diff viewer, `DiffPane`, one expanded at a time).
    The full-height ones (`Viz::fills`) are `code`, `diff`, `editor`,
    `terminal`, `embed` and `media` (each row's file as an image or a PDF,
    from its bytes); `approvals` is core's consent list; `custom` is
    an extension's sandboxed component. **`form`**: `form: { command,
    defaults? }` renders the command's `input_schema` as a `SchemaForm`, its
    fields starting from `defaults` (placeholders bound like an action's)
    under the loader's first row — a form needs no `from`
    (RPC `page_form` returns the spec and those values). Submitting (RPC
    `submit_page_form`, `pages::actions::submit_page_form`) runs the command
    as the page acting for the person, the defaults under what they
    entered; a command that asks shows `CommandConfirm`. A form's text
    rendering names its command (an agent runs the command itself). The
    loader drops a page whose viz node lacks the column roles its viz
    needs, naming the block (`chart`, `tree`, `timeline`, `steps`,
    `sections`, `hunks`), and `oxplow_validate_extension` checks every named column
    exists in the loader's result (`VizConfig::role_columns`). Pure data
    shaping lives in `vizModel.ts` (`barRows`, `lineSeries`, `treemapItems`,
    `treeNodes`, `timelineEntries`, `stepItems`, `hunkRows`).
