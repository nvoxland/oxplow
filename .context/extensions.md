# Extensions

This doc covers how anything that **measures or visualizes** is added to
oxplow: the `extension.yaml` format, lenses, slots, actions and alerts, and
the bundled `oxplow-analytics` example extension.

> **Status: built (epic tsk275; host finished in tsk278).**
> - **Current:**
>   - loading project extensions and their lenses from
>     `oxplow/extensions/` (see "What works today" below);
>   - running and validating lenses over IPC and MCP;
>   - the `oxplow-extension` agent skill;
>   - the lens page and the launcher's "Lenses" section;
>   - sharing: team via the repo, world via `install_extension` /
>     `update_extension` and Settings → Extensions;
>   - the core explorer: the Explore Data page (with Save as Lens) and
>     lens tiles on dashboards;
>   - **bundled extensions** (compiled in, read-only, reserved names) and
>     the `effort-review` **slot**; the bundled `oxplow-review` extension
>     is the effort review packet;
>   - `exec` **sources** that bring external records in as entities. The
>     mechanics and decisions are in
>     [semantic-layer.md](./semantic-layer.md) → "User and extension
>     sources", and a tested example is in `examples/extensions/github/`.
>   - **slots** (`effort-review`, `commit`, `uncommitted`, `task-detail`,
>     `thread`), **advisories**, per-project **disabling**, and the
>     **`oxplow-analytics` extraction** (tsk280): every analytics page and
>     widget is now a lens in that bundled extension, and core works with
>     it disabled (checked headless, 2026-09-27).
>   - lens alerts and the `rail` slot (tsk316), and extension-declared
>     measures, metrics and gauges (tsk311; see "Contributing metrics").
> - **Current:** extension-declared dimensions (tsk328).
> - **Current:** lens action buttons (tsk329) and the `settings` slot
>   (tsk330).
>
> When a piece ships, move it from "target" to "current" here, in the
> same commit.

## What works today

Code: `crates/oxplow-app/src/extensions.rs` (the loader, lens runs and
validation), `crates/oxplow-rpc/src/commands/extensions.rs` (IPC), and the
lens tools in `crates/oxplow-mcp/src/lib.rs`.

- **Files.**
  - `oxplow/extensions/<name>/extension.yaml` contains `name` (must equal
    the folder) and `description`.
  - `lenses/<slug>.yaml` contains `title`, `description`, `query`, `viz`
    (`table` | `list` | `number` | `markdown`), `params` (`name`, `label`,
    `default`), `columns` (`key`, `label`, `link: {kind: task | file |
    wiki | effort-diff, from}`) and `empty`.
  - Unknown keys are errors, so typos surface instead of being ignored.
- **Ids.** A lens id is `<extension>/<slug>`.
- **Reading.** Everything is read from the **stream's worktree** on every
  call. There's no cache yet, so an edit shows up on the next call.
- **Params.** Values are bound as `:name` (`SemanticLayer::query_sql_named`).
  Supplied values override defaults. An unknown param name is an `Invalid`
  error, so a typo can't silently fall back to a default. A param with no
  default and no value binds NULL.
- **Errors never cascade.** A missing or bad `extension.yaml`, or a folder
  name mismatch, marks that extension's `errors` and skips its lenses. A
  bad lens file is listed in `errors` and skipped; the other lenses still
  load.
- **`validate_extension`** also dry-runs every lens with its defaults. It
  reports SQL errors, and `columns` / `link.from` keys that the query
  doesn't return.
- **Surfaces.**
  - IPC and MCP: `list_extensions`, `get_lens`, `run_lens`,
    `validate_extension`.
  - MCP only: `list_lenses`.
  - All take an optional `stream_id`; the default is the primary stream.
  - `get_open_page` (MCP) plus `report_open_page` (UI) are current; see
    "Agents: the MCP surface".
    Agents write lens files with their normal Edit tool, under the filing
    guard, taught by the `oxplow-extension` skill.
- **UI.**
  - `lens:<extension>/<slug>` pages (`LensPage.tsx`) render `table`,
    `list`, `number` (through `formatMetricValue`) and `markdown`.
    Linked cells go through `RouteLink`, so plain-click navigates in the
    tab.
  - Params sit in the right rail: Enter or blur applies, Escape reverts.
    A tab id may carry starting values (`lens:<ext>/<slug>?effort_id=12`).
  - A run failure shows the error inline, with a nudge to use "Improve
    with Agent".
  - The page re-runs, debounced, on data events and on edits under
    `oxplow/extensions/`; `shouldRerunLens` in `src/lens/lensModel.ts`
    decides which events count.
  - "Improve with Agent" inserts `[oxplow lens <id> k=v…]`, with only the
    changed params, through the standard add-to-context path.
  - The Cmd+P launcher re-reads lenses from the stream's worktree every
    time it opens and lists them under their `launcher.category`
    (default **Lenses**; `hidden` ones not at all), merged into the
    static directory by category order (`mergeDirectory`).
  - `alert:` marks when a lens needs attention: `{ min_rows: N }` (the
    run returned at least N rows) or `{ column: c, above: X }` /
    `below: X` (the first row's value), with an optional `label`. The
    loader refuses one with both or neither condition. Every `LensRun`
    (IPC and MCP `run_lens`) carries `alert: {firing, count, value,
    message}`; the message is `label: count`, `N rows`, or `label: value`.
  - The `rail` slot (no params) takes only lenses with an `alert`; the
    rail's **Alerts** section (`AlertsSection` in `RailHud.tsx`, first by
    default) runs them and lists each firing one, opening the lens.
    oxplow-review mounts Waiting on Me there.
  - **Actions** (tsk329): `actions:` adds buttons above the result, from a
    fixed registry only (`LensActionKind`). An extension can offer a
    button but never run code through one.
    - The forms: `copy`, `add-to-context`, or
      `{action: run-source, source: <ext>/<id>, label?, id?}`. The `id`
      defaults to the kind and must be unique within the lens.
    - `copy` returns `extensions::lens_text`: a markdown/number lens's
      value, else a markdown table of exactly the displayed columns (the
      same rule as the UI's `displayColumns`).
    - `run-source` goes through `source_runner::sync_source`, the one
      entry point the IPC and MCP source runs use too. It **never
      approves**: an exec source nobody approved fails with a pointer to
      Settings → Data, where what runs and its hosts are shown. So the
      lens click isn't a weaker consent path.
    - `add-to-context` is UI-only; it pastes `[oxplow lens <id> params…]`.
    - Core: `lens_actions::run_lens_action`, behind IPC and MCP
      `run_lens_action` (MCP refuses add-to-context). UI: `LensActions` in
      `LensResultView.tsx`, logic in `lens/lensActions.ts`, hidden in
      compact strips.
    - This replaces the old `copy: true` flag. The review packet's Review
      Prompt uses `actions: [copy]`, and the GitHub example's PR lens has
      Sync PRs + Copy.
  - **Viz** (`LensResultView.tsx`): `table`, `list`, `number`, `markdown`,
    plus `bar` (`DailyBarChart`), `line` (`components/charts/TrendChart`,
    one chart per `chart.series`), `treemap` (two-level
    `components/charts/squarify`) and `grid` (children run with the
    params they declare). The loader drops a chart lens missing the
    `chart` columns its viz needs, and `validate_extension` checks those
    columns exist in the result. Pure data shaping lives in `lensModel.ts`
    (`barRows`, `lineSeries`, `treemapItems`, `childParams`).
- **Sharing.** There are three levels:
  - **Yourself.** An extension works in your worktree as soon as it's
    written.
  - **Your team.** Extensions are committed files under
    `oxplow/extensions/`.
  - **The world.** Publish an extension as a git repo with
    `extension.yaml` at its root. Others run `install_extension(git_url,
    git_ref?)` (IPC and MCP, or the install box in Settings → Extensions).
    - The repo is cloned inside `.oxplow/tmp/`, because workspace
      isolation forbids writing outside the project.
    - It's validated, then copied without `.git` into
      `oxplow/extensions/<name>/`. Symlinks are skipped, since they could
      point outside the extension.
    - The origin is recorded in `source.yaml` (`git`, `gitRef`, `sha`).
      The loader surfaces it as `Extension.source`.
    - Installing never overwrites an existing folder, and a name must be
      lowercase letters, digits and single dashes.
    - `update_extension(name)` re-clones from the recorded source. The old
      folder is replaced only after the new clone validates, and only for
      git-installed extensions.
    - Installing is a write tool on MCP. The skill says to do it only when
      the user asks, and to offer to commit the result.
  - Installed extensions are declarative (SQL lenses), so nothing
    executable runs. Exec sources will need explicit consent when they
    land.
- **Core explorer (stays in core, deliberately simple).**
  - **Explore Data** (`explore-data` page, `ExploreDataPage.tsx`):
    - Lists every entity from `describe_schema`, with column docs.
    - Picking one runs `SELECT * … LIMIT 50`; the SQL box then accepts
      any read-only query (Cmd/Ctrl+Enter runs it).
    - "Show as" switches the viz.
    - **Save as Lens** writes the file through the UI-only `save_lens`
      IPC, then opens the new lens. It creates the extension if missing,
      refuses to overwrite a lens, and refuses git-installed extensions.
  - **Lens tiles.** A lens can be pinned to a dashboard: "Pin to
    Dashboard" on a lens page, or MCP `add_dashboard_item(kind: "lens",
    lens_id)`.
    - The tile is kind `lens`, with `lensId` stored in `options_json`.
    - `LensTile` renders it with the shared `LensResultView`, capped at 8
      rows, against the primary stream, since dashboards are
      project-global.
    - The launcher's new **Data** category holds Explore Data, Metrics
      and Dashboards.
- **Sources.** Declared under `sources:` in `extension.yaml` and parsed
  into `Extension.sources`. A bad source is reported in `errors` without
  hiding the extension's lenses. Settings → Data shows each source
  (runtime, schedule, row counts or failure, Approve & Run / Sync Now);
  Settings → Extensions keeps each extension's source credentials.
- **Bundled extensions.**
  - Their sources live in the repo at `extensions/<name>/`. They're
    compiled into the binary by `crates/oxplow-app/src/bundled_extensions.rs`
    (`include_str!`). A test fails if a file in the folder isn't listed.
  - The loader reads them through the same `load_one` as project
    extensions, over an `ExtensionFiles` abstraction (`Disk` /
    `Embedded`). Nothing is written to disk.
  - `origin` is `bundled` and their path is `bundled:<name>`.
  - Their names are reserved:
    - a project folder with that name is listed with an error and never
      shadows the bundled one;
    - `install_extension` refuses the name;
    - `save_lens` refuses to write into a bundled extension.
- **Slots (current: `effort-review`, `task-detail`, `thread`, `commit`,
  `uncommitted`).**
  - `extension.yaml` declares `slots: [{slot, lens}]`. `SLOTS` in
    `extensions.rs` names each slot and the params it **offers**; a
    mounted lens gets the ones it declares and must declare at least one,
    or the mount is an error. Loaded as `Extension.slots`.
  - `src/lens/LensSlots.tsx` renders a slot: every mounted lens, run with
    the slot params it declares (`slotRuns`), re-run on data events.
    DiffViewPage offers `effort_id` and `change_id`, TaskPage `task_id`,
    PlanPane `thread_id` (the compact `strip` variant, which hides lenses
    with no rows), GitCommitPage and UncommittedChangesPage `change_id`.
    Numeric ids come from `numericRowId` (`tsk42` → 42).
  - **Latest request wins** (`src/request-guard.ts`, tsk370). LensSlots,
    LensPage and ExploreDataPage `begin()` each fetch and apply its result
    only while it's still the newest; new params (another thread, lens or
    stream) clear the old results first, so thread A's rows never show
    under thread B.
  - `change_id` comes from `src/lens/useChange.ts`: it calls
    `ensure_change` on mount, again on `ChangeStale` for its stream
    (working tree / effort targets) and on `ChangeAnalyzed` for its
    change (`shouldReensure`).
- **Change links.** `diff-at` (`from` path, optional `line`, `base` and
  `head` columns holding the change's labels; join `v_change`) opens that
  file's diff between the two sides (commit shas / `HEAD` → git refs,
  `working tree` → disk, `snapshot N` → that snapshot, so an effort's
  change links too). `compare` takes
  a `path:start-end|peer:start-end` value (build it in SQL) plus an
  optional `head` version column and opens the side-by-side
  `duplicate-block` page, which is now the general core compare page.
  Both refs carry their whole spec, so App's `handleOpenPage` and
  in-tab navigation register them directly.
- **Disabling.** `extensions: { disabled: [name] }` in
  `.oxplow/project.yaml` (bundled extensions included). `load_extensions`
  reads the list with `oxplow_config::disabled_extensions` (just that key,
  from the worktree being read) and returns a disabled extension with
  `enabled: false` and no lenses, slots or sources, so every consumer
  ignores it; `find_lens` and `run_lens` say it's disabled. Settings →
  Extensions toggles it through the UI-only `set_extension_enabled`.
- **`oxplow-review` (the review packet).** Its lenses, mounted in
  `effort-review`:
  - Decisions Made (`v_decision`, `provenance = 'recorded'`)
  - Decisions Oxplow Noticed (`v_decision`, `provenance = 'inferred'`)
  - Unverified Claims (`v_claim` where `verified = 0`)
  - What Deviated (`v_effort_file` vs the task's title/description: a
    file is in the task's area when the text names it or one of its
    directories at least two levels deep; silent when the task names no
    area)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `commit` and `uncommitted` slots)
  - Struggled Here (`v_struggle`)
  - Review Prompt (a copyable markdown prompt for reviewing the effort
    with a second harness: the task, the agent's summary, the files it
    changed, its claims and recorded decisions, and what to report)
  - Context Read (`v_context_read`)

  It also has a Waiting on Me lens, reachable from the launcher:
  questions agents are waiting on you to answer (the latest `await_user`
  call per thread, with the question in `v_tool_call.detail`, when no
  turn started after it), blocked tasks, and open notes. Agent status
  itself is in memory only (V2 dropped `agent_status`), so this reads
  the durable tool-call log instead. Each empty state says what's *good*, e.g.
  "Every claim for this effort is backed by evidence."

  The slot lenses above need `effort_id` / `change_id`, so they're
  `hidden` from the launcher (opened there they'd show NULL-param empty
  states). The launcher gets stream-level starters instead, under Work:
  **Recent Decisions** and **Unbacked Claims**, scoped by the implicit
  `stream_id`. `launcher_lenses_need_no_slot_params` enforces the rule
  for every bundled lens (tsk374).
- **One skill list.** Every agent runtime writes its skills from the single
  `OXPLOW_SKILLS` list in `crates/oxplow-plugin/src/lib.rs`, so adding a
  skill takes one row. ACP agents get the same list as an index in their
  system prompt plus the MCP `get_skill` tool (see agent-model.md → ACP).

## The rules

- **Controls in core, instruments in extensions.** Core ships what you
  steer with: threads, tasks and efforts, agent surfaces, comments, wiki,
  diff, the page graph. Everything that measures or visualizes is an
  extension over the [semantic layer](./semantic-layer.md). That includes
  what oxplow itself ships.
- **One format for first- and third-party.** The bundled
  `oxplow-analytics` extension uses exactly the format a user or their
  agent writes. If extracting it needs a capability the format lacks, that
  capability is added to core for everyone and logged under "Added for
  extraction" below. No private backdoors.
- **Declarative and scripted only.** An extension is YAML, SQL,
  Starlark/jq/exec sources and declarative lenses. No user-authored code
  runs in the app window. Reason: code in the window holds the daemon's UI
  token, and `/ipc` includes `forward_terminal_input`, so agent-written JS
  in the window could drive the agent. Revisit only after a scoped, read-only IPC
  capability exists ([remote-daemon.md](./remote-daemon.md) names the
  extension point).
- **Exceptions over trends.** Shipped lenses are live exception lists at the
  effort boundary (rows disappear when addressed), not dashboards of
  totals.

## Where extensions live

- `extensions/<name>/`: bundled with the app, read-only.
- `oxplow/extensions/<name>/`: project extensions, authored by the user or
  their agent and **committed with the repo**. This is how a team shares
  them.

**Why `oxplow/` and not `.oxplow/`:** `.oxplow/` is oxplow's *local
state*. Its `.gitignore` ignores everything except `project.yaml`, and the
fs watcher prunes its subdirectories wholesale, so files there are neither
committed nor snapshotted nor attributed to efforts. Project-authored
oxplow content already lives in `oxplow/` at the repo root (for example
`oxplow/gauges/*.star`). Extensions follow that convention, so they are
ordinary project files: visible in the file tree, diffed, reviewed and
merged like code.

**Per stream.** Extensions are read from the **stream's worktree**. An
agent building a lens in a worktree stream sees it there immediately.
Other streams see it once it's merged, just like any other code change.

Each extension is enabled or disabled per project in
`.oxplow/project.yaml`. Loading respects the workspace isolation rule
([architecture.md](./architecture.md)). A malformed extension shows an
error for that extension only; it never breaks the others or core.

## `extension.yaml`

```yaml
name: oxplow-analytics
description: …
sources:     [...]   # new data: entities and/or facts (see semantic-layer.md)
measures:    [...]   # the existing measures: schema
dimensions:  [...]   # over any entity or fact, including core-owned ones
metrics:     [...]   # aggregations over entities or facts
derived:             # named read-only SQL over v_*
  - {name: hotspot, sql: sql/hotspot.sql}
ai:                  # AI-function usages by role, cached as facts
  - {id: effort-risk, role: decide, over: v_effort, questions: …, measure: ext.effort_risk}
lenses: [lenses/*.yaml]
slots:               # mount lenses into core pages
  - {slot: effort-review, lens: read-this-first, order: 20}
  - {slot: task-detail,  lens: token-usage}
  - {slot: rail,         lens: waiting-on-me, as: badge}
  - {slot: launcher,     lens: metrics-explorer, category: Analytics}
```

## Lenses

A **lens** is a user- or agent-built way of looking at your work: a
query over the semantic layer plus how to show it. (Not "view", which is
taken by the SQL `v_*` views; not "data app".)

- `title`, `description`, `params` (typed, with defaults such as `stream`,
  `effort`, `range`).
- **Implicit params (tsk375).** A param named `stream_id` or `thread_id`
  is bound to the viewer's context (`extensions::LensContext`, numeric ids)
  unless the caller supplies it; precedence is supplied → context →
  `default`. `lens_context(svc, stream?, thread?)` resolves it: the given
  stream (else the thread's, else the primary) and the given thread (else
  the stream's selected-or-active one). IPC `run_lens` / `run_lens_action`
  resolve it from their `stream_id`, so the UI needs no plumbing; MCP
  `run_lens` / `run_lens_action` also take the agent's `thread_id`;
  `get_open_page` uses the viewer's thread. Validation dry-runs use no
  context (defaults only).
- `query`: SQL over `v_*` and the extension's entities, with `:param`
  binding.
- `viz`: `table`, `list`, `number`, `line`, `bar`, `markdown`, `treemap`,
  `hunks` (an ordered file/range list with badge columns that opens the
  diff at the range), `steps` (a guided walkthrough) or `grid` (child
  lenses).
- `columns`: label, format, and `link:` to a core ref (task, file, effort
  diff, wiki page, decision), so rows are page-graph links.
- `actions`: from a fixed registry only: add-to-context, followup-comment,
  open-diff, copy-review-prompt, run-source. Never arbitrary code.
- `alert`: a row-count or threshold condition that shows a rail badge
  (current; nudging the agent is what advisories are for).

Lenses render through one core `LensPage` / `LensSlot` in oxplow's design
system, as a `lens:<slug>` page kind. Bookmarks, backlinks, the launcher
and sibling navigation work unchanged. Every lens has an "Improve with
agent" action that pastes `[oxplow lens <slug>]` and its params into the
agent's context through the existing add-to-context path; oxplow never
types into the agent.

## Slots

Slots are the **only** way an extension reaches a core page. Core pages
declare them and render whatever is mounted, in `order`. With nothing
mounted, the page is plain.

| Slot | Core page |
|---|---|
| `effort-review` | diff-view for an effort |
| `task-detail` | TaskPage |
| `commit` | GitCommitPage |
| `uncommitted` | UncommittedChangesPage |
| `rail` | rail HUD Alerts section (current) |
| `launcher` | Cmd+P launcher entries (as a lens's `launcher.category`, not a slot) |
| `settings` | Settings: a section per mounting extension, titled with its name, before AI (current, tsk330; `SettingsSlotSections`, `slotRuns(…, extension)`). No params. Slots render in declaration order; there is no `order` field |

## Contributing metrics (current)

`extension.yaml` takes `measures:`, `metrics:`, `gauges:` and `dimensions:` in the
`.oxplow/project.yaml` schema, checked with the same
`oxplow_config::validate_*` functions, plus:

- Metrics must be `key:` definitions; `use:` belongs to the project.
- Entity metrics (`entity:` + `where` / `time` / `value`, tsk322) work
  here too, usually over the extension's own source views. Their fragments
  are checked at seed time, so one over a view whose source hasn't synced
  yet stays out of the catalog until the next reseed.
- Dimensions (tsk328), fact or entity (`entity` / `expr` / `join`), layer
  like metrics (`resolve_dimensions` takes an `extensions` layer) and slice
  an extension's entity metrics over the same view. They are stored as
  `scope = 'global'` plus the `extension` column (V89).
  - `promote` is refused: toggling the extension would rebuild the
    metric cube each time, so promoting stays a project decision.
  - A disabled or removed extension's dimensions are deleted
    (`delete_extension_dimensions_not_in`, never a promoted one). Facts
    don't reference dimension rows, so nothing else is lost.
- Gauges run `starlark` / `jaq` only. `exec` is refused: nothing from an
  extension runs a program without the user's approval, which is what
  (approved) sources are for. `entryFile` must exist in the extension.
- Precedence is built-in < global < each extension < project
  (`oxplow_config::resolve_{metrics,gauges,measures}` take an
  `extensions` layer; scope `extension:<name>`). An enabled extension's
  metrics are **on** unless the project mentions the key (a `use:`
  override or disable marker, or its own definition).
- `MetricsService::extension_catalog` loads enabled extensions from the
  primary worktree on each resolve; gauge scripts are read through
  `extensions::read_extension_file` (bundled or disk), and a gauge's
  `report` still resolves against the project.
- Storage: the scope CHECK only allows built-in / global / project, and
  rebuilding `measure` / `metric_spec` would cascade-delete facts (V54),
  so the store writes `extension:<name>` as `scope = 'global'` plus the
  `extension` column (V84) and reads it back. `v_metric_spec` shows
  `scope = 'extension'` and the `extension` name.
- `seed_catalog` prunes extension specs no longer declared (a disabled or
  removed extension) with `delete_extension_specs_not_in`; its measures
  stay, since deleting one would take its facts. It reseeds when an
  `oxplow/extensions/*/extension.yaml` changes (`WorkspaceChanged`).

## Advisories

An extension gives the coding agent guidance with **advisories** in
`extension.yaml` (current). Core owns only the mechanism; what to say, and
when, is SQL in the extension.

```yaml
advisories:
  - id: coverage-target
    on: post-tool-use        # or prompt
    once_per: effort         # effort (default) | row (needs a `key` column) | turn
    heading: "# Optional heading line"
    query: |                 # :effort_id = the thread's open effort
      SELECT '...' AS message FROM v_effort_observation WHERE effort_id = :effort_id ...
```

- `crates/oxplow-app/src/advisories.rs`: `AdvisoryRunner` runs the enabled
  extensions' advisories for one hook point and applies `once_per` with an
  in-memory, bounded fired-set (like the nudges it replaced, a restart may
  repeat one). Marks are recorded only after every query ran. A failing
  query is logged and skipped.
- **Consent (tsk352).** A shared extension's advisories (committed by a
  teammate, or installed from git) speak into the agent's context, so they
  run only once a person approved them. `exec_consent::advisory_program`
  turns an extension's advisories into a program (kind `advisories`), with
  one arg per advisory: its id, trigger, repeat rule, heading and query.
  Settings → Data → Programs lists them, and any change needs approving
  again. `advisories::consented` filters before running; bundled
  extensions aren't gated.
- `for_thread(svc, thread, on)` runs them for the thread's **single** open
  effort (none under parallel efforts), reading extensions from the
  thread's stream worktree. Post-tool-use hits are persisted as nudges
  (kind `<extension>/<id>`, `v_agent_nudge`).
- The control plane appends post-tool-use hits to the collection nudge in
  PostToolUse `additionalContext`, and prompt hits to the UserPromptSubmit
  context (with the session-context and decisions blocks).
- `validate_extension` dry-runs each advisory with `:effort_id` NULL and
  checks it returns `message` (and `key` for `once_per: row`).
- oxplow-analytics ships three: `coverage-target`, `metric-deltas`,
  `threshold-crossed` (see [metrics.md](./metrics.md)). Advisories read
  stored views (`v_effort_metric_delta`, `v_effort_observation`), never the
  engine directly.
- The report-less-run nudge stays in core: it's about collection hygiene,
  not an instrument.

## Agents: the MCP surface

Everything a human can see or build, an agent can see or build too, over
MCP. That includes whatever **extensions** add: an extension's sources,
entities, dimensions, metrics and lenses show up in the same generic tools
as core's. Extensions never add their own MCP tools. That keeps the agent's
tool list stable no matter how many extensions are installed.

**Understanding the semantic layer**

- `describe_schema`: every entity (core `v_*` and extension
  `v_<ext>_<entity>`) with column docs, declared relations (joins), and the
  owning source/extension.
- `list_dimensions`, `list_metrics`: including extension-declared ones,
  each marked with its owner.
- `query_sql`: read-only SQL across everything above.
- `get_metric(key, dims?, range?)`: a metric's value or series, sliced by
  any applicable dimension.

**Working with lenses**

- `list_lenses`: every lens with its extension, params, slots and alert
  state.
- `get_lens(slug)`: the lens definition (YAML) and its resolved query.
- `run_lens(slug, params?)`: the **same rows, columns and alert state the
  UI shows** for those params, so the agent sees exactly what the human
  sees.
- `get_open_page(thread_id)` **(current)**: what the human has open in
  that thread, for any page kind (`task:42`, `file:…`, `lens:…`). For a
  lens it adds `lensRun`, the lens re-run with the human's *current*
  params, so "look at what I'm looking at" works.
  - The UI reports the active page (`report_open_page`, UI-only) together
    with whatever that page published to `tabs/openPageDetail.ts`. The
    lens page publishes `{lensId, params}`.
  - The state is held in memory on the thread runtime
    (`ThreadRuntimeRegistry::open_page`).
  - A hidden-but-mounted page can't overwrite the report, because only
    the active page's detail is sent.
- **Row actions (current):** right-click a lens row → "Add Row to Agent
  Context", which inserts `[oxplow lens <id> row: col=value, …]`.
- **Lens buttons (current, tsk329):** `actions:` from the fixed registry,
  run by an agent with `run_lens_action` (see "What works today").

**Building**

- Agents author extensions and lenses **by editing files** under
  `oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  filing guard. The loader hot-reloads.
- **No scaffold tools (decided).** A scaffold tool would write project
  files outside the filing guard. The skill carries the templates
  instead, and humans have Save as Lens in Explore Data.
- `validate_extension(name)` returns load errors, schema errors and a dry
  run of every lens query, so the agent can check its work without the UI.
- `list_extensions`, `run_source(id)`.
- **Worktree streams: preview, don't run (tsk377).** Source data is
  project-wide (one `ext__<ext>__<entity>` table), so `run_source` /
  `sync_source` always run the **primary** worktree's copy. An agent
  writing a source in a worktree stream checks it with MCP
  `preview_source(extension, source_id, stream_id)`: `source_runner::
  preview_source` runs that worktree's version through the same
  `produce` step (same consent: an exec source needs a person's approval
  of that exact hash; derived sources don't) and returns the coerced
  rows per entity (first 50, plus totals), storing nothing and recording
  no run.

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-plugin/assets`)
teaches the format, the `v_*` contract and the loop "describe_schema →
query_sql → write files → validate_extension → run_lens". "Improve with agent" on a lens pastes
`[oxplow lens <slug>]` plus its params into the agent's context.

## The `oxplow-analytics` example extension

What moves out of core, and what it becomes:

| Today (core) | Becomes |
|---|---|
| Planning / Review / Quality dashboards | `grid` lenses (**done**: `planning`, `review`, `quality`) |
| Code-quality runner, dup scan, FindingPage, DuplicateBlockPage | **done:** the `findings` / `duplicate-blocks` lenses; the dup scan runs in core's change analysis; `DuplicateBlockPage` stays core as the compare page |
| Change-analysis cards (treemap, look-here-first, functions, co-change, zones) | **done:** the `change-review` grid in the `effort-review` / `commit` / `uncommitted` slots; core keeps a changed-files tree (`ChangedFilesTree`, `useChangedFiles`) |
| Gauges (`oxplow/gauges/*.star`, idiom `.star`) | extensions can declare gauges now (tsk311); the built-in catalog stays core as the opt-in standard library |
| Gauge-threshold nudges | **done:** the `threshold-crossed` advisory (with `coverage-target` and `metric-deltas`) |
| Usage / page analytics / token pages, `ThreadTokenTotal`, `EffortTokenUsage` | **done:** the `usage` grid; `task-tokens` (`task-detail` slot) and `thread-tokens` (`thread` slot) |
| Local history dashboard | stays core (snapshots are substrate); the `recent-snapshots` lens in `review` covers the at-a-glance view |
| Effort metrics block, effort coverage page, tests-run and nudge blocks | **done:** `effort-review` slot lenses `effort-tests` (grid: coverage, untested files, test runs, failed tests, analysis findings), `effort-metric-deltas`, `effort-nudges` |

**Stays in core, deliberately simple:** a basic **metrics explorer** and
**simple dashboards** (Nathan, 2026-09-27). The base version must let
people see what metrics exist and pin a few to a dashboard as a starting
point, a bit like Metabase. It doesn't need to be fancy. The explorer
browses the semantic-layer catalog (entities, measures, metrics), shows a
table or simple chart sliced by a dimension, and can save the result as a
lens or pin it to a dashboard. Dashboards are a plain grid of pinned
metrics and lenses. Tracked in the "basic metrics explorer" task.
**Done (tsk309):** Metrics is a catalog (search, enabled/all, range, branch,
sparkline + latest value); Metric Detail is trend + recordings + stats +
enable + Add to dashboard; dashboard tiles are line / number metrics, text
and lenses under a range/branch filter ([dashboards.md](./dashboards.md)).

**Also stays in core** (it is substrate other features need): snapshots,
collection ingest, attribution, token ingest, page visits (the rail and
launcher use them), the fact store and engine, Go To / bookmarks, the Git
dashboard. diff-view shrinks to title + file list + diff + the
`effort-review` slot; TaskPage keeps the `task-detail` slot.

New in the extension: the **effort review packet**, all live exception
lenses:

- Waiting on me
- What deviated (files outside the task's stated area)
- Unverified claims
- Decisions made
- Tests weakened (deleted/skipped tests, removed assertions)
- Missing co-change (files that historically change with these but didn't)
- Read this first (risk-ranked hunks with evidence badges)
- Struggled here
- Copy review prompt (for reviewing with a second harness; the human
  pastes it)

**Done when** core boots and is usable with `oxplow-analytics` disabled,
and enabling it restores the old pages' behavior as lenses.

## Added for extraction

Capabilities added to core because the extraction needed them (tsk280),
available to every extension:

- `bar`, `line`, `treemap` and `grid` viz, with `chart` and `children`.
- `commit` and `metric` link kinds; `file` links with `line`; `task`
  links accept a bare `v_task.id`.
- `task-detail` and `thread` slots, with slot params checked at load.
- Lens `launcher.category` and `hidden`.
- Lens `actions:` (copy, add-to-context, run-source; tsk329).
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see "Advisories").
- `LEGACY_PAGE_REDIRECTS` (`tabs/legacyRedirects.ts`): saved tabs,
  bookmarks and history for a page kind that moved to a lens open the
  lens. A `{ lens, param }` entry carries the old id's row id into a lens
  param (`effort-coverage:eff12` → `effort_id=12`).
- Lens tabs carry params: `lens:<ext>/<slug>?k=v` (`lensRef(id, params)`),
  so a slot lens's heading opens its page with the slot's values, and
  history and bookmarks keep them.
