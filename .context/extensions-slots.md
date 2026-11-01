# Extensions: Slots, replacements and shell UI

Slots, replacements, commands in core menus and decorators: how an extension extends the shell. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Slots on today's pages

- **Slots (see "Slots" below).**
  - `extension.yaml` declares `ui: { slots: [{slot, page, at?, title?}] }`
    (v1: top-level `slots`). A slot is a `slot` node in a page, named
    `<extension>/<page>.<name>` (`oxplow-foundation/diff-view.sections`).
    A mount whose page is not in `pages/`, or whose name is not that shape,
    is a load error. A mount whose host page is not loaded, or has no slot
    of that name, is a warning ("`x` shows nowhere"). Loaded as
    `Extension.ui.page_slots`.
  - The mounted page runs as an embed: its params come by name from the
    host's params, else from the first row of the host's loaders. So the
    changes page (`diff-view`) offers `change_id` and `effort_id` from its
    loader row; the work item page offers `uri`; the diff page offers
    `path` in its `file-header` slot (bundled's co-change page mounts
    there). The host draws each mounted page as a section titled by its
    page.
  - A `stack` folds titled sections that have nothing to show into one
    closing line (see "A page of sections leads with what matters").

## Slots

A **slot** is the only way an extension reaches another extension's page.
A `slot` node in a page's `stack` or `grid` (`- { slot: sections }`) is a
slot named `<extension>/<page>.<name>`; with nothing mounted it draws
nothing. A mount is an entry in the mounting extension's manifest:

```yaml
ui:
  mounts:
    - { page: item-tokens, at: oxplow-bundled/work-item.sections, position: { before: links }, title: Tokens }
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
  page is not in `pages/`; the slot is not `<extension>/<page>.<slot>`.
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
| `oxplow-foundation/diff-view.sections` | the Changes page (an effort's or a range's diff; nodes `effort`, `header`, `description`, `verdict`, `concurrent`, `files`, `extensions`) | `uri`, `start`, `end`, `stream_id`, and the page's row (`effort_id`, `change_id`, …) |
| `oxplow-bundled/work-item.sections` | the Work Item page (nodes `state`, `fields`, `body`, `info`, `links`, `comments`, `activity`) | `uri`, `thread_id`, `stream_id` |
| `oxplow-bundled/tasks.sections` | the Tasks page (nodes `list`, `summary`, `backlog`; `at: { before: list }` puts a strip above the list) | `show`, `priority`, `thread_id`, `stream_id` |
| `oxplow-foundation/commit.sections` | a commit's page (nodes `detail`, `files`) | `uri`, `sha`, `stream_id`, and `change_id` (the page's row) |
| `oxplow-foundation/uncommitted-changes.sections` | Uncommitted Changes (nodes `commit`, `files`; `at: { before: commit }` puts a strip above everything) | `stream_id`, and `change_id` (the page's row) |
| `oxplow-foundation/new-session.sections` | the New Session page (nodes `start`, `earlier`) | `stream_id`, `thread_id`, `q` |
| `oxplow-foundation/diff.file-header` | the Diff page (one file between two revisions; the slot sits above the diff). oxplow-bundled mounts its `file-co-change` page here | `path`, `left`, `right`, `label`, `base`, `lines` |

`change_id` points at the page's change in `v_change`, with its analysis
in `v_change_file`, `v_change_function`, `v_change_import`,
`v_change_co_change` and `v_change_duplicate`. To learn a page's slots
and params, read it with `oxplow_find_page` or `oxplow.page.describe`: its
slot nodes and loaders are listed.

The launcher is not a slot: a page lists itself with `category`.

## Replacements

A `ui.mounts` entry whose `at` names a core component (no `/`; one of
`v_replaceable`) puts a **page in place of that named core
sub-component** — never a whole page. The same list mounts a page at
another page's slot (`at: <extension>/<page>.<slot>`, with `position`,
`title`, `size`); `version` goes only with a core component, the slot
keys only with a slot. The old `ui.slots` / `ui.replacements` keys are
refused, the refusal naming the `ui.mounts` spelling. Panels stay their
own `panels:` declaration: a panel is a thing with an id, title, icon and
scope made of part-pages, not a placement of one page.

```yaml
ui:
  mounts:
    - { page: swimlanes, at: work_item.board, version: 1 }
```

The targets are one table of data, `crates/oxplow-domain/data/replaceable.json`
(read by `oxplow_domain::replaceable::all()`; `fixtures/replaceable.json` pins
its targets so it only shrinks):
each a `target`, the `capability` it belongs to, whether it is **bound to
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

| Target | Core component | Capability | Bound | Props | View |
|---|---|---|---|---|---|
| `work_item.board` | the Board's columns of cards (the `board` page's node) | `work_items` | no | `scope` (`thread` / `backlog` / `all`), `thread_id` (applies when `scope` is `thread`) | — |
| `work_item.list` | the Tasks page's list: the scope picker, detail pane and backlog drawer stay oxplow's | `work_items` | no | `scope` (`thread` / `stream` / `all`), `thread_id` (the thread when `scope` is `thread`, else null) | — |
| `work_item.detail` | a work item's description editor (the `body` node of the `work-item` page): the state, fields, About, links, comments, activity and the mounts stay oxplow's | `work_items` | no | `uri` (the item) | — |
| `work_item.detail.state` | a work item's State (the `state` node) — never its Move To (the About node's menu), so a replacement cannot strand an item | `work_items` | yes | `uri` (the item) | — |
| `effort.verdict` | an effort's verdict, the first line of its page (the `verdict` node of the `diff-view` page). Core has **none of its own**: with no replacement, or `oxplow` chosen, the page shows nothing. oxplow-bundled's `effort-verdict` is the default | `effort_policy` | no | `effort_id` (the effort's row id) | — |
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

**Bound or not.** A bound target shows one provider's own thing — the
state control lists that tracker's states — so only the extension of the
provider whose item it shows replaces it. Every other target is a **view**:
the Board, the list and the editor show the capability as a whole, reading
`v_work_item` and changing items through the interface commands
(`oxplow.work_item.update`, `transition`, `reorder`, `move`), so they work
with whichever list is active and any extension may bring one — a view
needs no provider. Choosing a presentation is separate from installing a
work list.

**Whose shows** is the person's: `replacements: { <target>: <extension> |
oxplow }`, one project key validated against the table. An extension
chosen shows (an unbound target only); `oxplow` keeps oxplow's own (any
target); unlisted is the default — the only extension replacing it,
several and no choice keeping oxplow's own, and for a bound target its
provider's extension's. Settings → Integrations lists each replaced
target with that choice (a `<select>`: Default, oxplow's own, each
extension). Every target has a `label` a person reads, in the same table
("board", "state control", "history graph"), carried on each
`UiReplacement`. A state control replacement lists the tracker's own
states, each with **Move Here** (`oxplow.work_item.transition` with the
`native_state`); oxplow's Move To stays beside it.

**At load** (`replacements::parse_replacements`, errors at the entry's
line): the target is in the table; the page is one of the extension's and
declares **every** prop (a replacement takes the whole contract — a Board
that ignores `scope` is not the Board); a pinned `version` is the
contract's; for a bound target, the extension brings a provider of the
target's capability (`providers:`); a target is replaced once per
extension. The viewer's stream is not a prop: a page that declares
`stream_id` gets it, as everywhere. Valid ones are
`ExtensionUi.replacements` (`UiReplacement { id: <ext>/<target>,
extension, target, capability, bound_to_provider, page, version, view,
label }`).

**In the run** (`PageRunner::replacement` → `replacements::chosen`), from
the enabled extensions' replacements of the target, only when one exists:
- **bound**: the **item's own provider** decides, whichever is active, so
  another provider's item never gets a page that sends this one's states.
  The provider is the `provider` param of the node's `uri` param (a work
  item's URI carries it); which extension brings it is the capability
  registry's answer at run time, not at load, so installing or switching
  one needs no reload;
- **unbound**: the person's choice (`replacements`), or the only
  candidate; no provider is read;
- no candidate, a provider no extension brings (oxplow's own), several
  unbound candidates and no choice, or `oxplow` chosen → the node's own
  content, `replaced_by` unset;
- chosen → its page runs as the node's embed with the props it declares;
  a run that fails sets the node's `error` and the node's own content
  runs instead.

**Drawn** (`components/Replaced.tsx`, chosen by `PageTree` when the node
has `replaced_by`): the embed (kit vizzes, or `viz: custom` with its
sandbox and "custom" badge, sharing the view its target's contract lists)
under a **"replaced by <extension>"** badge (`replacement-<target>`,
`replacement-badge`), kept fresh as any page run is. The page's own chrome
(the Board's scope picker) stays oxplow's. A run with an `error` draws
the core component under a line saying whose it was and why ("beads's
board couldn't load: …. Showing oxplow's.", `replacement-fallback`),
outside the replacement's frame: no badge. Nothing is decided in the
desktop, so the wrong component never flashes.

Showing one records `usage { kind: "replacement", key: <ext>/<target> }`
(the evidence a kind needs to be promoted), once per replacement drawn
without an error. Agents read work items from `v_work_item` either way.
The rules are tested in Rust (`replacements::chosen`'s tests, the runner's
`a_replaceable_node_runs_the_replacement_that_stands_in_for_it`); which
replacement a page shows is a fixture's (`loaders: { <node>/<loader> }`,
a `ui` step on the embedded component); the badge, the fallback line and
choosing one on Settings are the e2e specs'. The pages with every
enhancement off are `tests-e2e/specs/pages/plain.spec.ts`'s
([work-items.md](./work-items.md), "Conformance").

**The first real one: the history graph.** oxplow-bundled's
`history-graph` component (`components/history-graph/`, classic scripts on
the client library and the kit) draws the stream's commits as `git log
--graph` does — a lane per line of descent, a dot per commit — from its
`history-graph` part (a recursive walk of `v_commit` from
`:head_sha`, with local branch and tag labels from `v_branch` / `v_tag`).
Its text, author and branch filters only dim or hide rows, so typing never
re-runs the page. On Git History they are the page's `q`, `author` and
`branch` (the contract's `view`): they start from the page's, each change
writes them (`setViewParams`; an echo of its own write is ignored, so
typing is not undone by a late one) and Back sets them again
(`onViewParams`). With no view (a tile) they are its own. The host re-runs
the page when a commit lands or a ref moves (its reads). A row opens the
commit's URI (`oxplow:oxplow/commit?sha=<sha>`); a Cmd/Ctrl-click opens it
in a new tab (`navigate(uri, { newTab: true })`). It declares no commands,
so it is not a program to approve. Core keeps no lane code: `CommitList`
(the fallback, and the Git dashboard's recent commits with their stats) is
a plain list. `viz/historyGraph.test.ts` runs the bundle's own script
against its page — lanes, merge lanes, child-before-parent order,
filters, reveal, Cmd-click.

**The example that proves views and forms: `examples/extensions/task-views`.**
An extension with no provider of its own, replacing `work_item.board`
(the `swimlanes` component: a column per canonical state, drag a card to
another column to `oxplow.work_item.transition` it, drag within a column to
`reorder` it only when the active provider's `features` include `ordering`,
click a title to rename it through an inline `<form>` with `ox-input` and a
field refusal shown in `ox-error`, a click on a card `navigate`s, a
Cmd/Ctrl-click opens a new tab, and a `DENIED` refusal is one line above the
board), `work_item.list` (`compact-list`, a `viz: table` with Done and an
`Edit…` form action) and `work_item.detail` (`edit-item`, a `viz: form` over
`oxplow.work_item.update` with `uri` fixed). It reads `v_work_item` and
changes items only through the interface commands, so it works with any
list. `viz/swimlanes.test.ts` runs the bundle's script against its page;
`every_example_scaffolds_checks_and_tests_clean` checks it clean and runs its fixtures.

A form (`FormViz`) re-reads its start values when it resets, so after a
successful Save it shows what was saved, not the title it first opened
with. Activation takes a cloned snapshot of the config (`read_config`)
rather than holding its read guard across the transaction: a nested read
behind a waiting config reload would otherwise deadlock the daemon when
the active work list switched
(`tests-e2e/specs/components/task-views.spec.ts` is its regression test).

## Commands in core menus (a command's `ui.about`)

A command an extension declares meets a person through its own `ui`
([commands.md](./commands.md) "Offering a command to a person"). One
**about a resource** (`ui: { label, group, about: oxplow/work_item, input: {
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
    - { dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: label, color: color }
```

The dataset (`v_<extension>_<dataset>`) must declare a `uri` column and the
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

**Promoted to stable** on its first-party use: oxplow-bundled shows each
effort's latest verdict on the effort — a `ref-chip` on its page and a
`row-badge` where a row links to it. Its dataset `verdict` (`Accepted`,
`Accepted (forced)` or `Changes requested`; green, orange, red) reads
`verdicts`, which keeps every verdict event (`materialize: {
incremental: seq }`, appended as each lands; a rewrite of the log —
retention's payload expiry, a restart's refill — refills it whole). It
reads only the events' **envelopes**: the type says the verdict
(`oxplow_bundled.accepted` / `.changes_requested`) and the subject what
it was about — the effort first, then its work item, then each claim and
decision an acceptance took unchecked (any makes it forced). Retention
keeps envelopes and an extension's payloads go after 30 days at most, so
a verdict a decorator shows must not live in its payload. It shows; it
never acts.
