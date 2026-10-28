# Slots, panels, renderers, decorators and replacing a core view

- **Slots** mount a page into another page's `slot` node (`ui: { slots: [{
  slot, page, at?, title?, size? }] }` in `extension.yaml`). `slot:
  <extension>/<page>.<name>`; the mounted page runs as an embed, getting the
  host's params by name (declare the ones you need). `at` is `start`, `end`
  (default) or `{ before | after: <sibling id> }`; an `at` naming a node the
  host doesn't have is a load warning and the mount goes last. `size` is for
  a grid host. The mounted section shows marked with your extension. Foundation
  pages with a `sections` slot:
  - `oxplow-foundation/commit.sections` and
    `oxplow-foundation/uncommitted-changes.sections` → `change_id` (the
    `v_change*` analysis; the page's `row` loader) — `at: { before: commit }`
    puts a strip above the uncommitted page;
  - `oxplow-foundation/diff-view.sections` (an effort's or a range's diff) →
    `effort_id`, `change_id`, …;
  - `oxplow-foundation/work-item.sections` (a work item's page, whichever
    list) → `uri`;
  - `oxplow-foundation/tasks.sections` (the Tasks page) → `thread_id`,
    `stream_id`;
  - `oxplow-foundation/new-session.sections` (sections `start`, `earlier`) →
    `thread_id`.
  `oxplow_find_page`/`oxplow.page.definition` on a page shows its slot
  nodes; read a page's loaders to learn the params it binds.

- **`ui.decorators`** add a label to the URIs of one resource: `{ dataset,
  resource: oxplow/work_item, placement: ref-chip | row-badge, label, color?
  }` — the dataset (this extension's) needs a `uri` column plus the `label`
  (and `color`) columns; a chip shows on that URI's page, a badge after a
  linked cell in a row.

- **Renderers**: a page with `for: <resource>` draws that resource's URIs
  (`page-format.md` "Renderers"). A page has no launcher entry unless it has
  a `category`; its id carries its params, `label` names its tab.

- **Panels** put a page in the left nav (`panels: [{ id, title, icon?, scope,
  body, badge?, open?, collapsed?, count?, dock? }]`): `body` is a page drawn
  compact; `badge` is a page with an `alert` (or `badge:`), giving the
  panel's count and listing in the Alerts panel while it fires; `collapsed`
  is a page shown compact while the panel is collapsed; `count` is a page
  whose value (first cell) or row count is the header count; `open` is the
  URI the header opens. `scope` is `project`, `stream` or `thread` — a
  `stream`/`thread` panel's pages must declare `stream_id` / `thread_id`,
  which the nav binds. With `dock: true` the panel is an icon in the dock
  at the bottom left, its count the badge, the body opening above the bar;
  use it for a short list the person checks and dismisses.

## Replacing a core view

When a person wants their own Board, Tasks list or work item editor, an
extension replaces that core view with a page (or a component) — no work
list provider needed. The views and their props are `v_replaceable`; the
page declares each prop as a param:

```yaml
ui:
  mounts:
    - { page: swimlanes, at: work_item.board, version: 1 }   # props: scope, thread_id
    - { page: edit-item, at: work_item.detail, version: 1 }  # props: uri
```

- Read items from `v_work_item`; change them only through the interface
  commands (`oxplow.work_item.update`, `transition`, `reorder` — when the
  active list's `features` in `v_capability_provider` include `ordering` —
  and `move`), so the view works with whichever list is active. A component
  declares the commands it runs (`commands:`); one outside the extension's
  namespace also needs `commands:<pattern>`, approved at deploy.
- **An effort's verdict** (`effort.verdict`, props `effort_id`) is the first
  line of an effort's review page, and core has none of its own: to change
  review rules ("block on a dropped table", "coverage of 90"), replace it
  with a page that judges the effort from `v_test_run`,
  `v_effort_observation`, `v_change_function` and your own facts. A `tone`
  column (`success`, `warning`, `danger`) colors a line.
- The person picks whose view shows: Settings → Integrations, or
  `replacements: { <target>: <extension> | oxplow }` in the project config
  (a person-only key: tell them, don't set it). One extension replacing a
  view shows by itself.
- The page around the view stays oxplow's (the Board's scope picker, a work
  item's state pill and Move To), so an item can always move.
- A target whose `view` (in `v_replaceable`) lists keys shares them with its
  page: a replacement component reads `component.viewParams`, writes
  `setViewParams` and follows `onViewParams` for exactly those keys (any
  other is refused). Pin the contract's `version:`.
