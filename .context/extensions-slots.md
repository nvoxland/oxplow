# Extensions: Slots, replacements and shell UI

Slots, replacements, commands in core menus and decorators: how an extension extends the shell. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Slots on today's pages

- **Slots (see "Slots" below).**
  - `extension.yaml` declares `ui: { slots: [{slot, page, at?, title?}] }`
    (v1: top-level `slots`). A slot is a `slot` node in a page, named
    `<namespace>/<page>.<name>` (`oxplow/diff-view.sections`).
    A mount whose page is not in `pages/`, or whose name is not that shape,
    is a load error. A mount whose host page is not loaded, or has no slot
    of that name, is a warning ("`x` shows nowhere"). Loaded as
    `Extension.ui.page_slots`.
  - The mounted page runs as an embed: its params come by name from the
    host's params, else from the first row of the host's loaders. So the
    changes page (`diff-view`) offers `change_id` from its
    loader row; the task page offers `uri`; the diff page offers
    `path` in its `file-header` slot (bundled's co-change page mounts
    there). The host draws each mounted page as a section titled by its
    page.
  - A mounted page with nothing to show keeps its section, its own
    `empty:` line under its title, as the host's sections do (see "A page
    keeps one shape whatever its data holds").

## Slots

A **slot** is the only way an extension reaches another extension's page.
A `slot` node in a page's `stack` or `grid` (`- { slot: sections }`) is a
slot named `<namespace>/<page>.<name>`; with nothing mounted it draws
nothing. A mount is an entry in the mounting extension's manifest:

```yaml
ui:
  mounts:
    - { page: item-tokens, at: oxplow/task-detail.sections, position: { before: links }, title: Tokens }
```

`slot` is the host's slot name; `page` is a slug of the mounting
extension; `at`, `title` and `size` are optional. The mounted page runs as
an embed (`PageRunner::run_mount`): its params by name from the host's
params and its loaders' first rows, so it declares the ones it needs. Its
node id is the mounted page's id with `/` written `.` (`beta.extra`), and
the desktop marks it with the mounting extension's name. A disabled
extension mounts nothing.

- `at` (`SlotAt`, untagged because serde_yaml reads tagged enums only as
  YAML tags): `start`, `end` (the default, at the slot node) or `{ before:
  <sibling id> }` / `{ after: <sibling id> }` beside a sibling of the slot
  in its parent stack. The id is the contract, so a mount survives the
  host's other code changing. An `at` beside an id the slot's parent does
  not hold shows nowhere: the runner drops that mount. `size` (`small`, `wide`, `tall`, `full`) is for a
  grid host.
- Load errors at the mount's line (`extensions.rs`, `slot_files`): the
  page is not in `pages/`; the slot is not `<namespace>/<page>.<slot>`.
  Load warnings: the host page is not loaded, or has no slot by that name
  (the mount "shows nowhere").
- A mount's contribution id is `<host>.<slot>-<n>` with the host's `/`
  written `.` (`PageSlot::local_id`). Slot mounts are `ExtensionUi.page_slots`.
- A top-level `slot_mounts:` / `decorators:` / `replacements:` is an
  unknown field: they live under `ui:`.

The slots foundation's and bundled's pages declare (each `slot` node is one; its page's
loaders' first rows and params are what a mount can take):

| Slot | Host page | Params a mount may take |
|---|---|---|
| `oxplow/diff-view.sections` | the Changes page (a request's, a turn's or a range's diff; nodes `header`, `description`, `concurrent` (the other requests, on any thread, whose turns changed the same files in the same window), `files`, `extensions`) | `uri`, `start`, `end`, `stream_id`, and the page's row (`change_id`, …) |
| `oxplow/task-detail.sections` | the Task page (nodes `state`, `fields`, `body`, `info`, `links`, `comments`, `activity`) | `uri`, `thread_id`, `stream_id` |
| `oxplow/knowledge-page.sections` | a knowledge page (the end of its stack; oxwiki mounts its Edit part) | `uri`, `store`, `id` |
| `oxplow/commit.sections` | a commit's page (nodes `detail`, `files`) | `uri`, `sha`, `stream_id`, and `change_id` (the page's row) |
| `oxplow/uncommitted-changes.sections` | Uncommitted Changes (nodes `commit`, `files`; `at: { before: commit }` puts a strip above everything) | `stream_id`, and `change_id` (the page's row) |
| `oxplow/change-review.sections` | oxplow-bundled's Review page (one analysed change; nodes `asked`, `look-here`, `by-area`). A signal belongs on `v_oxplow_review_signal` (`adds_to: oxplow/review_signal`); mount here only a whole section of your own | `change_id` |
| `oxplow/settings.integrations` | Settings → Integrations, under Capabilities: an extension's own settings for the project. oxtask mounts its Task workflow (`task-workflow-settings`) | none (the page's params: `area`) |
| `oxplow/new-session.sections` | the New Session page (nodes `start`, `earlier`) | `stream_id`, `thread_id`, `q` |
| `oxplow/diff.file-header` | the Diff page (one file between two revisions; the slot sits above the diff). Nothing shipped mounts here | `path`, `left`, `right`, `label`, `base`, `lines` |

`change_id` points at the page's change in `v_change`, with its analysis
in `v_change_file`, `v_change_function`, `v_change_import`, `v_change_hunk`,
`v_oxplow_change_co_change`, `v_change_duplicate` and the review's
`v_oxplow_review_signal`. To learn a page's slots
and params, read it with `oxplow_find_page` or `oxplow.page.describe`: its
slot nodes and loaders are listed.

The launcher is not a slot: a page lists itself with `category`.

## Replacements

A `ui.mounts` entry whose `at` names a core component (no `/`; one of
`v_replaceable`) puts a **page in place of that named core
sub-component** — never a whole page. The same list mounts a page at
another page's slot (`at: <namespace>/<page>.<slot>`, with `position`,
`title`, `size`); `version` goes only with a core component, the slot
keys only with a slot. The old `ui.slots` / `ui.replacements` keys are
refused, the refusal naming the `ui.mounts` spelling. Panels stay their
own `panels:` declaration: a panel is a thing with an id, title, icon and
scope made of part-pages, not a placement of one page.

```yaml
ui:
  mounts:
    - { page: swimlanes, at: task.board, version: 2 }
```

The targets are one table of data, `crates/oxplow-domain/data/replaceable.json`
(read by `oxplow_domain::replaceable::all()`; `fixtures/replaceable.json` pins
its targets so it only shrinks):
each a `target`, the `capability` (or shared `model`) it belongs to, whether it is **bound to
a provider**, its contract's **version**, its **props contract** — the
params the replacement page is given instead of any host state — and its
**view**: the host page's view params the replacement shares, read
(`init.view`), written (`setViewParams`) and heard (`viewParams`) as the
core component reads and writes them. Only those keys: any other is
refused, and a target with no view gives its replacement none. The table
is published as `v_replaceable` and generated into the reference
(`the_reference_lists_every_replaceable_view`). A declaration's optional
`version:` must be the contract's: a changed contract refuses it rather
than handing it props it does not know.

| Target | Core component | Belongs to | Bound | Props | View |
|---|---|---|---|---|---|
| `task.board` | the Work board (the `board` component node of bundled's `work` page; v2) | model `oxplow/task_record` | no | `scope` (`thread` / `stream` / `me` / `pool` / `all`: whom the items are assigned to), `thread_id` (applies when `scope` is `thread`); a page's implicit `stream_id` applies when `scope` is `stream` | — |
| `task.list` | the Task List's list (the `list` node of bundled's `task-list` page; v2): its filters and New Task… stay oxplow's | model `oxplow/task_record` | no | `scope` (`thread` / `stream` / `me` / `pool` / `all`: whom the items are assigned to), `thread_id` (applies when `scope` is `thread`) | — |
| `task.detail` | a task's description editor (the `body` node of the `task` page): the state, fields, About, links, comments, activity and the mounts stay oxplow's | model `oxplow/task_record` | no | `uri` (the item) | — |
| `task.detail.state` | a task's State (the `state` node) — never its Move To (the About node's menu), so a replacement cannot strand an item | model `oxplow/task_record` | yes | `uri` (the item) | — |
| `knowledge.detail.body` | a knowledge page's body (the `body` node of bundled's `knowledge-page`): oxplow's own is the read-only markdown; About, referenced files, backlinks, comments and mounts stay oxplow's. oxwiki's replacement is an editor with Save | model `oxplow/knowledge_record` | yes, by the URI's `store` (a task's is `provider`; the table's `provider_param`) | `uri` (the page) | — |
| `vcs.history.graph` | Git History's commits (a node of the `git-history` page): oxplow's own is a plain list | `vcs` | no | `head_sha` (the stream's head), `reveal_sha` (a commit to scroll to and mark, or null), `limit` | `q`, `author`, `branch` (Git History's filters; v2) |

**Who places a target.** A page node marked `replaceable: <target>`
(`Node.replaceable`; `check_page` refuses an unknown target and a target
placed twice on a page). The node's own content is oxplow's own and the
fallback. **The page run decides** (`PageRunner::replacement`, the rules in
`replacements::chosen`): when a replacement stands in, the node runs it as
an embed (`run_embedded`, as a slot's mount runs) — its props by name from
the run's `params`, a prop the params leave out from the first row of the
page's loaders (the Tasks page's `row` loader derives `scope` from its
`show` param), only the props the replacement page declares — and
`RunNode.replaced_by` names the extension; else the node's own content
runs. So a fixture of the host page sees the replacement
(`loaders: { <node>/<loader> }`). The desktop draws what the run decided
(`PageTree` → `components/Replaced.tsx`: the embed under a badge, sharing
the view its contract lists; a replacement that didn't run shows oxplow's
own and why). The page's params must cover the contract so
the props exist; a page with a `replaceable` node is not warned about
params it passes on.

**Bound or not.** A bound target shows one store's own thing — the
state control lists that store's stages — so only the extension that fills
the model as the item's own store replaces it. Every other target is a
**view**: the Board, the list and the editor show the model as a whole,
reading `v_oxplow_task` and changing items through the task action points
(`oxplow/task.move`, `task.edit`, … and bundled's `oxplow.tasks.assign`
and `reorder`), so they work with whichever store holds the task and any
extension may bring one — a view needs no store. Choosing a presentation
is separate from installing a store.

**Whose shows** is the person's: `replacements: { <target>: <extension> |
oxplow }`, one project key validated against the table. An extension
chosen shows (an unbound target only); `oxplow` keeps oxplow's own (any
target); unlisted is the default — the only extension replacing it,
several and no choice keeping oxplow's own, and for a bound target its
provider's extension's. Settings → Integrations lists each replaced
target with that choice (a `<select>`: Default, oxplow's own, each
extension). Every target has a `label` a person reads, in the same table
("board", "state control", "history graph"), carried on each
`UiReplacement`. A state control replacement lists the store's own
stages, each with **Move Here** (the `oxplow/task.move` point with the
stage id); oxplow's Move To stays beside it.

**At load** (`replacements::parse_replacements`, errors at the entry's
line): the target is in the table; the page is one of the extension's and
declares **every** prop (a replacement takes the whole contract — a Board
that ignores `scope` is not the Board); a pinned `version` is the
contract's; for a bound target, the extension fills the target's model
(`fills:`) or brings a provider of its capability (`providers:`); a target is replaced once per
extension. The viewer's stream is not a prop: a page that declares
`stream_id` gets it, as everywhere. Valid ones are
`ExtensionUi.replacements` (`UiReplacement { id: <namespace>/<target>,
extension, target, capability, bound_to_provider, page, version, view,
label }`).

**In the run** (`PageRunner::replacement` → `replacements::chosen`), from
the enabled extensions' replacements of the target, only when one exists:
- **bound**: the **item's own store** decides, so another store's item
  never gets a page that sends this one's states. The store is the
  node's `uri` param's `provider_param` (the table's: `provider` on a task's URI, `store` on a knowledge page's);
  which extension fills it is the model registry's answer at run time, not
  at load, so installing or switching one needs no reload;
- **unbound**: the person's choice (`replacements`), or the only
  candidate; no provider is read;
- no candidate, a store no extension fills, several
  unbound candidates and no choice, or `oxplow` chosen → the node's own
  content, `replaced_by` unset;
- chosen → its page runs as the node's embed with the props it declares;
  a run that fails sets the node's `error` and the node's own content
  runs instead.

**Drawn** (`components/Replaced.tsx`, chosen by `PageTree` when the node
has `replaced_by`): the embed (kit vizzes, or `viz: custom` with its
sandbox and, for an extension that isn't oxplow's own, a "by <extension>" mark,
sharing the view its target's contract lists) under a hover title naming
the extension (`replacement-<target>`, no chip), kept fresh as any page run is. The page's own chrome
(a Task List's filters) stays oxplow's; the Work board has none, so its stand-in shares its filters (`view`). A run with an `error` draws
the core component under a line saying whose it was and why ("beads's
board couldn't load: …. Showing oxplow's.", `replacement-fallback`),
outside the replacement's frame: no badge. Nothing is decided in the
desktop, so the wrong component never flashes.

Showing one records `usage { kind: "replacement", key: <namespace>/<target> }`
(the evidence a kind needs to be promoted), once per replacement drawn
without an error. Agents read tasks from `v_oxplow_task` either way.
The rules are tested in Rust (`replacements::chosen`'s tests, the runner's
`a_replaceable_node_runs_the_replacement_that_stands_in_for_it`); which
replacement a page shows is a fixture's (`loaders: { <node>/<loader> }`,
a `ui` step on the embedded component); the badge, the fallback line and
choosing one on Settings are the e2e specs'. The pages with every
enhancement off are `tests-e2e/specs/pages/plain.spec.ts`'s
([tasks.md](./tasks.md), "Conformance").

**Git History.** Bundled's `history-graph` component replaces the commit
list at `vcs.history.graph`. It starts in List view. Graph draws each commit
and its parents in lanes, with children before their parents. Both views
use the same rows from `:head_sha` through `v_commit` and label local branch
tips and tags from `v_branch` and `v_tag`. A `reveal_sha` link starts in Graph
and highlights the commit. The mode is kept in the tab entry's history state.
The List filters hide nonmatching rows. Graph dims text and author mismatches
to retain their connections; its branch filter keeps the reachable rows.
Both share Git History's `q`, `author` and `branch` view params, which the
replacement page also declares as params, so they flow into its run by
name. The List shows the `history_matches` loader: the search runs in SQL
over everything reachable from the head, before `limit`, as core's list
does, so an older commit is found too. The Graph draws the `history_graph`
loader, the newest `limit` commits unfiltered: lanes need every commit
between two matches, so a match beyond the loaded range is not drawn
there. Both also filter locally, so a change shows before the re-run
answers, and a tile (no view) filters its loaded rows only
(`git-history-search-finds-commits-beyond-the-loaded-limit`). The host refreshes the rows when a commit or ref moves. Each row
links to the commit, with Cmd/Ctrl-click or middle-click opening a new tab.
Setting `oxplow.replacements.vcs.history.graph` to `oxplow` restores the
standard list. Bundled's fixtures exercise the replacement and its controls.

**The example that proves views and forms: `examples/extensions/task-views`.**
An extension with no provider of its own, replacing `task.board`
(the `swimlanes` component: a column per canonical state, drag a card to
another column to move it (`oxplow/task.move`), drag within a column to
`reorder` it in its assignee's queue,
click a title to rename it through an inline `<form>` with `ox-input` and a
field refusal shown in `ox-error`, a click on a card `navigate`s, a
Cmd/Ctrl-click opens a new tab, and a `DENIED` refusal is one line above the
board), `task.list` (`compact-list`, a `viz: table` with Done and an
`Edit…` form action) and `task.detail` (`edit-item`, a `viz: form` over
the `oxplow/task.edit` point with `uri` fixed). It reads `v_oxplow_task`
and changes items only through the task action points, so it works with any
store. `viz/swimlanes.test.ts` runs the bundle's script against its page;
`every_example_scaffolds_checks_and_tests_clean` checks it clean and runs its fixtures.

A form (`FormViz`) re-reads its start values when it resets, so after a
successful Save it shows what was saved, not the title it first opened
with. Activation takes a cloned snapshot of the config (`read_config`)
rather than holding its read guard across the transaction: a nested read
behind a waiting config reload would otherwise deadlock the daemon when
a capability's choice switched
(`tests-e2e/specs/components/task-views.spec.ts` is its regression test).

## Commands in core menus (a command's `ui.about`)

A command an extension declares meets a person through its own `ui`
([commands.md](./commands.md) "Offering a command to a person"). One
**about a resource** (`ui: { label, group, about: oxplow/task, input: {
uri: "{{uri}}" } }`) is offered on a URI of that resource. It shows in
its page's nav-bar ☰ (at the top level, sorted by label with the page's
own actions) and in the right-click menu of a row for the row's URI (a
page's table or list rows, by the first URI the row links to — `rowRef`
— and Board cards), grouped by `ui.group`. It is offered only on a ref
whose state it accepts: `ui.about_when: <column>` names a column of the
dataset its resource resolves through (`v_agent_session.reopenable`)
that must be truthy on the ref's row, the rule of a page action's
`when:`; the daemon checks it (`ref_commands`) as a menu opens, and a
command that fails is left out of the menu (`oxplow extension check`
refuses an unknown column). Give it to a command whose operation
refuses some states (Reopen Session: a closed session only). The input's strings may be exactly
`{{uri}}` / `{{uri.id}}` (or `{{stream}}` / `{{thread}}`, which expand to
bare ids; `{{uri.id}}` is the URI's key). Without an `input`, a command
gets `{ uri: "{{uri}}" }`. The desktop builds the offers from the
person's command listing and the ids the daemon says apply
(`refOffers`, `components/refCommands.ts`) and
runs each as search does (App publishes its offer host,
`setRefOfferHost`). To put another command on a URI's menu — oxplow's
transition to done, say — an extension declares its own command over the
same operation (`scope:` / `provider:` + `op:`, with its `ui.input`) or
a script that composes it. The old `ui.commands` block is gone.

**The desktop reads the extensions from one store**
(`extensionsStore.ts`, `useExtensions()`): one `listExtensions` (the
live set, the same in every stream) and one event subscription
however many readers are mounted (slots, decorators, rail panels,
extension pages, the launcher). It reloads on `extensionsChanged`. The
listing lives while something reads it, so a later mount loads afresh.
Only the Extensions page lists them itself — it reloads after its own
deploys and toggles.

## Decorators

`ui.decorators` (**stable**; `extensions/decorators.rs`) put labels from
one of the extension's **datasets** on URIs of a resource:

```yaml
ui:
  decorators:
    - { dataset: flags, resource: oxplow/task, placement: ref-chip, label: label, color: color }
```

The dataset (`v_<namespace>_<dataset>`) must declare a `uri` column and the
`label` (and optional `color`) columns named. This is checked at load
against the dataset's declared columns, which are its contract. Column
names must be plain identifiers, since the desktop names them in its
query. The desktop (`components/decorators.ts`, `useDecorations`) runs
each decorator's queries over the URIs it shows (`SELECT uri, "<label>"
AS label[, "<color>" AS color] FROM <view> WHERE uri IN (…)`), re-run
when the dataset changes. **Bounded**: the URIs go 200 to a query
(`REFS_PER_QUERY`), each query with a row limit that has room for all of
them, so a large table loses no badge. One extension adds at most 3
decorations to one URI (`MAX_DECORATIONS_PER_REF`), each label at most 40
characters (`MAX_LABEL`, then `…`). `ref-chip`: a chip in the header of a
page whose URI the dataset lists, after the page's own chips (`Page`, by
the page's URI from its navigation context). `row-badge`: a `RefBadge`
(tone `label`) after a table or list cell that links to a listed URI. A
color is used only when it is a plain one (`#rgb…` or a CSS color name,
`safeColor`). Decorations are additive: a decorator whose query fails
shows nothing.

A decorator shows; it never acts.

## Action points and the store filter

Part of the shared-models work. Built: named action points, the object
form of `ui.about` with its store filter, and the SDK fixture kinds that
test them. **Not built:** the per-point default setting (Settings, then a
split button) and "a store's item commands default to their own stores"
(`fills:` exists now; the default is still to build). The store filter
reads the store from the item URI's param the item's shared model names
(`store_param`: tasks keep `provider`), falling back to `store`.

**A named action point** is a place where several extensions offer the
same kind of action ("New Task") and none of them owns the page.

- **Declared by its owner**, in the manifest (`extensions/action_points.rs`):
  `action_points: [{ name: task.new, label: New Task, doc?, context: {
  parent?: uri, thread_id?: int } }]`. Its id is `<namespace>/<name>`
  (`oxplow/task.new`). A context key is typed `string`, `uri`, `int` or
  `bool`; `name?` marks it optional.
- **Placed by a page:** a view node `{ action_point: oxplow/task.new,
  context: { parent: { param: uri } } }`. Its `context` values are bound
  the way an embed's `params` are (`{ param: x }`, `{ from: loader.col }`
  or a literal): the run resolves them (`RunNode.action_point`). The node
  is refused in an inline page.
- **Joined by a command:** `ui: { at: oxplow/task.new, input: { parent:
  "{{context.parent}}" } }`. `{{context.<key>}}` is a whole-string binding
  of the point's context; a key the page left out stays unset. Without an
  `input` the command gets the whole context. `at` excludes `about`. A
  command that needs more than the context sets `ui.form`: the form page
  opens with the context's keys as its params (the path item commands use
  with `?uri=`).
- **Drawn by the shell** (`components/ActionPoint.tsx`; the rule
  `drawsFor`, stated in Rust by `Draws::for_count`): nothing with no
  contributor, a button with one, a menu (the context menu under a button
  labelled with the point's label) with several. Menu order: by command id,
  which starts with the owning extension's namespace.
- **One list, two kinds of key.** Item commands (`ui.about`) and point
  commands (`ui.at`) are the same list in the command registry
  (`commands/ref_commands.rs`: `OfferKey::Item(kind)` or
  `OfferKey::Point(id)`, `offered`). Search lists neither.
- **Asked through** IPC `action_point_commands { point, context }`
  (`commands/action_point_offers.rs`): `{ point, label, draws, commands: [{
  id, label, input }] }`, each `input` already bound. A context key the
  point does not declare or of the wrong type is an error; a missing
  required key offers nothing. Item commands are asked through
  `ref_commands { uri }` as before. Neither is an MCP tool: an agent lists
  what it may run with `list_commands`.

**An item point.** A point may name its **item**: `item: uri`, a required
`uri` context key (`action_points: [{ name: task.move, label: Move, item:
uri, context: { uri: uri, stage: string } }]`). A command that joins it may
carry `ui.accepts: { store: <id> | [ids] }` (with `at` only; an item
command's filter stays in `about`): the point offers it only when the
item's store passes, by the same rule as below (the store param is the
item's model's `store_param`). A command with `accepts` at a point with no
item is never offered; one without `accepts` always is. This is how a
shared page (bundled's task pages, the Work board) offers "move this
task" without naming any store: each store joins the point for its own
items.

A command at an item point binds the item in its input: `{{uri}}` is the
item's URI and `{{uri.<param>}}` one of its params (`{{uri.id}}`), as an item
command's do (`CommandUi::point_input`); a key the item lacks leaves the
entry out.

**One command, one action: `about` and `at` together.** A command may have
both — it is offered on its item (`about`) and at a point (`at`), the same
action. The store filter is then `about.accepts`, which serves the point
too; `ui.accepts` is only for a point command with no `about` filter, and
declaring both is refused ("one store filter"). A `form` command at a point
opens its form with the context's keys (and the `{{uri}}` bindings) as
params; the form's submit is the input.

**The store filter.** `ui.about` is either the resource kind (a string) or
`{ resource, accepts?: { store: <id> | [ids] } }`. An item URI's `store`
param is matched like a renderer's `accepts` rule: an id equal to the
entry, or `present`, `absent` or an anchored id pattern
(`pages::renderers::takes`); a list is any-of. The command shows only on
items whose store passes; an item with no `store` param passes only a
filter that says `absent`.

**Tests.** A fixture `when: { action_point: <id | name>, context? }` has
`expect: { draws: none | button | menu, commands?: [ids] }`; `when: {
item_commands: <uri> }` has `expect: { commands: [ids] }`
(`oxplow-agent-text/.../references/testing.md`).
