# Slots, pages, panels, decorators and replacing a core view

- **Slots** mount a lens into a core page (`ui: { slots: [{ slot, lens }] }`
  in `extension.yaml`). The lens must declare at least one param the slot
  binds, and gets only the ones it declares:
  - `effort.review.details` (an effort's diff) → `effort_id`, `effort` (the effort's ref, for an action's `input`), `change_id`;
  - `oxplow-foundation/commit.sections` (a commit page) and
    `oxplow-foundation/uncommitted.sections` (the working tree), sections
    of those documents → `change_id` (the `v_change*` analysis; the page's
    row, declared in its `columns:`). `at: { before: commit }` puts a
    strip above the uncommitted page;
  - `vcs.history.sidebar` (beside Git History) → `stream_id`;
  - `oxplow-foundation/work-item.sections` (a work item's page, whichever
    list) → `uri`;
  - `thread.plan.header` (the Work panel, compact) → `thread_id`;
  - `settings.<area>.sections` → no params (`overview` passes `q`, its search); a section on that Settings
    area's page (`overview`, `agents`, `lsp`, `extensions`, `data`,
    `integrations`, `ai`), placed `at` the start, the end or beside one of
    the area's sections (`{ after: integrations }`): its status or setup
    views where a person looks for them.

  The old names (`task-detail`, `commit`, …) are errors naming the new
  one.

- **`ui.decorators`** add a label to the URIs of one resource:
  `{ model, resource: oxplow/work_item, placement: ref-chip | row-badge, label, color? }` — the
  model (this extension's) needs a `uri` column plus the `label` (and
  `color`) columns; a chip shows on that URI's page, a badge after a
  linked cell in lens rows.

- **Pages** give a lens a place of its own: `pages: [{ id, title, label?,
  icon?, category?, keywords?, params?, count?, lens }]` opens it full-page at
  `oxplow:oxplow/page?page=<extension>/<id>` (plus its params) and lists it in the launcher under
  `category` (Work, Code, Git, Activity, Knowledge, Data, Lenses,
  System; leave it out for a page only opened from elsewhere).
  `params` are what its id carries, handed to the lens; `label` is what
  its tab says, `{{param.<name>}}` filled from them (`title` stays its
  name in the launcher); `count` names a
  `number` lens whose value is the launcher badge; `keywords` are more
  words the launcher matches. A page with `for: <resource>` (or a list;
  your own `<extension>/<name>`, oxplow's `oxplow/*`, another
  extension's, or `https` / `http`) is a **renderer**: it draws the URIs
  of that resource, and has no launcher entry unless it has a
  `category`. `accepts: { <param>: present | absent | <anchored
  pattern> }` limits it to URIs whose params fit; `default: true` (only
  for a resource your extension declares, one page per resource) makes
  it the page a plain open uses; `pinned: true` pins its tabs;
  `comments: true` lets the person comment on the page's text (on its
  URI). Its lens
  must declare a `uri` param and runs with the URI; a param named like
  one of the URI's params (`id`, `path`) gets its value, and any other
  param must be a `role: filter`. A resource has no
  `page:` field; a URI nothing draws shows on a fallback page, not an
  error.
- **Panels** put a lens in the left nav (`panels: [{ id, title, icon?,
  scope, body, badge?, open?, collapsed?, count? }]`): `body` renders
  compact; `badge` (a lens with an `alert`) gives the panel's count and
  lists in the Alerts panel while it fires; `collapsed` is a lens shown
  compact while the panel is collapsed (its summary); `count` is a lens
  whose row count (a `number` lens: its value) is the header count
  without an alert — it wins over the badge's. `scope` is `project`, `stream` or `thread` — a `stream` /
  `thread` panel's lenses must declare `stream_id` / `thread_id`, which the
  nav binds. The person arranges, hides and collapses panels.

## Replacing a core view

When a person wants their own Board, Tasks list or work item editor, an
extension replaces that core view with a lens or component — no work list
provider needed. The views and their props are `v_replaceable` (query
it); the lens declares each prop as a `param`:

```yaml
ui:
  replacements:
    - { target: work_item.board, lens: swimlanes, version: 1 }   # props: scope, thread_id
    - { target: work_item.detail, lens: edit-item, version: 1 }  # props: uri
```

- Read items from `v_work_item`; change them only through the interface
  commands (`oxplow.work_item.update`, `transition`, `reorder` — when
  the active list's `features` in `v_capability_provider` include
  `ordering` — and `move`), so the view works with whichever list is
  active. A component declares the commands it runs (`commands:`); one outside
  the extension's namespace also needs `permissions.commands`, approved at deploy.
- **An effort's verdict** (`effort.verdict`, props `effort_id`) is the
  first line of an effort's review page, and core has none of its own:
  when a person wants their own review rules ("block on a dropped table",
  "coverage of 90"), replace it with a lens that judges the effort from
  `v_test_run`, `v_effort_observation`, `v_change_function` and your own
  facts. A `tone` column (`success`, `warning`, `danger`) colors a line.
- The person picks whose view shows: Settings → Integrations, or
  `replacements: { <target>: <extension> | oxplow }` in the project
  config (a person-only key: tell them, don't set it). One extension
  replacing a view shows by itself.
- The page around the view stays oxplow's (the Board's scope picker, a
  work item's state pill and Move To), so an item can always move.
- A target whose `view` (in `v_replaceable`) lists keys shares them with
  its page: a `viz: custom` replacement reads `component.viewParams`,
  writes `setViewParams` and follows `onViewParams` for exactly those
  keys (any other is refused), so it leaves the same address oxplow's own
  view does. Pin the contract's `version:`.
