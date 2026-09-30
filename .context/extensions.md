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
>   - lens alerts (tsk316; the `rail` slot they mounted in became panels
>     in P6.G1), and extension-declared
>     measures, metrics and gauges (tsk311; see "Contributing metrics").
> - **Current:** extension-declared dimensions (tsk328).
> - **Current:** lens action buttons (tsk329) and the `settings` slot
>   (tsk330).
> - **Current (P1, 2026-09-28/29):** manifest v2 with `intent`,
>   `sharing` and the stable/experimental split (tsk413), the textual
>   v1→v2 migrator (tsk414), the per-root catalog cache (tsk415, tsk390) and
>   the SDK: `oxplow plugin new|check|migrate` (tsk416; "The SDK").
>
> When a piece ships, move it from "target" to "current" here, in the
> same commit.

## What works today

Code: `crates/oxplow-app/src/extensions.rs` (the loader, lens runs and
validation), `crates/oxplow-rpc/src/commands/extensions.rs` (IPC), and the
lens tools in `crates/oxplow-mcp/src/lib.rs`.

- **Files.**
  - `oxplow/extensions/<name>/extension.yaml` is a **manifest v2**
    (`manifest: 2`, `name` = the folder, `sharing`, `intent`, the
    contribution kinds; see "`extension.yaml`" below).
  - `lenses/<slug>.yaml` contains `title`, `description`, `query`, `viz`
    (`table` | `list` | `number` | `markdown`), `params` (`name`, `label`,
    `default`), `columns` (`key`, `label`, `link: {kind: task | file |
    wiki | effort-diff, from}`) and `empty`.
  - Unknown keys are errors, so typos surface instead of being ignored.
- **Ids.** A lens id is `<extension>/<slug>`.
- **Reading.** Everything is read from the **stream's worktree**, through
  `Services.extension_catalog` (`crates/oxplow-app/src/extension_catalog.rs`):
  a per-root cache behind a stat-only fingerprint of `oxplow/extensions/**`
  and `.oxplow/project.yaml`, so a hit costs ~60 µs instead of the ~3 ms
  parse and an edit still shows up on the very next call (see
  [performance.md](./performance.md)). `find_lens`, `run_lens`,
  `validate_extension`, `list_data_entities` and the advisory/metric/source
  readers all take the catalog; `load_extensions` itself is the cache's
  loader and the write paths' direct read.
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
  - All take an optional `stream_id`. Over MCP an omitted one is the
    caller's own stream (its header, else its thread's; the primary only
    for an anonymous caller), so an agent in a worktree sees the
    extension it just wrote — the same for `preview_source`,
    `review_extension`, `install_extension`, `update_extension`,
    `run_lens_action` and `ensure_change` (tsk574). `site_search` is the
    exception by design: omitted, it searches every stream. The UI
    (IPC) names its stream.
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
  - The page re-runs when a model it read changed (`modelsChanged`), when
    facts landed for a measure its `metric_grid()` read
    (`metricSamplesChanged`), and on edits under `oxplow/extensions/` —
    the run's `result.reads` is what it subscribes to (P4.6). Every lens
    host uses `useRerunOnChange` in `src/lens/lensRerun.ts`.
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
  - **Panels** (P6.G1, target §11.3): `panels: [{ id, title, icon?,
    scope: project | stream | thread, body, badge? }]` put a lens in the
    left nav (`Extension.panels`, `ExtensionPanel`; checked at load in
    `parse_panels`: a kebab-case id, lenses that exist, a badge with an
    `alert`, and a `stream` / `thread` scope's lenses declaring
    `stream_id` / `thread_id`). The body renders compact; the badge's
    alert count shows on the panel, and the core **Alerts** panel lists
    every firing badge. The `rail` slot is gone: a rail mount is a load
    error naming `panels:`. oxplow-review's Waiting on You is a panel
    whose badge is its own lens.
  - **Actions are commands** (P6.B1, target §11.4): `actions:` declares
    `{ id, label, command, input?, row? }` — a button above the result, or,
    with `row: true`, an item in each row's right-click menu.
    - `input` is the command's input. A string that is exactly
      `{{param.<name>}}` or `{{row.<column>}}` becomes that value, typed
      (a number stays a number); a string containing them has them spliced
      in as text. The values go into the command's input, never into SQL.
      At load, a `{{param.x}}` must name a declared param and `{{row.x}}`
      needs `row: true`; `validate_extension` checks `{{row.x}}` against
      the result's columns; the command name must be well-formed. The old
      fixed registry (`copy`, `add-to-context`, `run-source`) is a load
      error saying where each went.
    - **A lens grants no power.** `lens_actions::run_lens_action` runs the
      command through the bus as `Actor::Lens { lens_id, on_behalf_of }`
      — `Human` from the UI (`run_lens_action` RPC), the calling agent from
      MCP `run_lens_action` (which never confirms). The bus applies the
      command's `invokers.lens`, and the agent policy through
      `on_behalf_of`, so an agent can't reach a human-only command or
      confirm one through a lens. The audit records `lens:<id>`.
    - A command that asks (`NEEDS_CONFIRMATION`, now kept on the thrown
      `IpcCallError`'s `code`) shows a confirm strip in the toolbar: Run
      (focused, Enter) or Cancel (Escape).
    - **Every lens has Copy** (its text rendering, the `lens_text` RPC)
      **and Add to Agent Context** (`[oxplow lens <id> params…]` into the
      agent's input, never sent) — kit affordances, not declared. A
      compact strip and a grid's children (`toolbar={false}`) show no
      toolbar.
    - Syncing a source is the command **`source.sync { extension, source }`**
      (External, `Invokers::ALL`; `source_runner::SourceRunner::sync`,
      which the scheduler uses too). It **never approves**: approving is
      `source_runner::approve_reviewed` behind the UI-only
      `approve_source` RPC (Settings → Data's Approve & Run approves, then
      runs the command). MCP `run_source` runs the command as the agent.
    - UI: `LensToolbar` and the row menu in `LensResultView.tsx`; logic in
      `lens/lensActions.ts` (`performLensAction`, `copyLens`,
      `addLensToContext`, `rowRecord`).
    - The GitHub example's PR lens has a Sync PRs action
      (`command: source.sync`).
  - **Viz** (`LensResultView.tsx`): `table`, `list`, `number`, `markdown`,
    plus `bar` (`DailyBarChart`), `line` (`components/charts/TrendChart`,
    one chart per `chart.series`), `treemap` (two-level
    `components/charts/squarify`) and `grid` (children run with the
    params they declare), and the structure components (P6.A2): `tree`
    (collapsible nesting by `tree.parent`; orphans are roots, a cycle is
    cut), `timeline` (oldest first, linked through `timeline.ref`),
    `detail` (the first row as label/value pairs), `steps` (a checklist
    by `steps.status`: done, active, failed, else pending) and `hunks`
    (each row's file diffed between `hunks.from` and `hunks.to` in the
    diff viewer, `DiffPane`, one expanded at a time), and **`form`**
    (P6.B2): `form: { command, defaults? }` renders the command's
    `input_schema` as a `SchemaForm`, its fields starting from `defaults`
    (placeholders bound like an action's) under the query's first row —
    a form needs no `query` (RPC `lens_form` returns the spec and those
    values). Submitting (RPC `submit_lens_form`,
    `lens_actions::submit_form`) runs the command as the lens acting for
    the person, the defaults under what they entered; a command that asks
    shows `CommandConfirm`. A form's text rendering names its command
    (an agent runs the command itself). Only a form may omit `query`. The loader drops a
    lens missing the column roles its viz needs, naming the block
    (`chart`, `tree`, `timeline`, `steps`, `hunks`), and
    `validate_extension` checks every named column exists in the result
    (`Lens::role_columns`). Pure data shaping lives in `lensModel.ts`
    (`barRows`, `lineSeries`, `treemapItems`, `treeNodes`,
    `timelineEntries`, `stepItems`, `hunkRows`, `childParams`).
- **Sharing.** There are three levels:
  - **Yourself.** An extension works in your worktree as soon as it's
    written.
  - **Your team.** Extensions are committed files under
    `oxplow/extensions/`.
  - **The world.** Publish an extension as a git repo with
    `extension.yaml` at its root. Others run `review_extension(git_url,
    git_ref?)` then `install_extension(git_url, git_ref?, reviewed_sha)`
    (IPC and MCP, or the install box in Settings → Extensions).
    - **Review first (tsk378).** `review_extension` (or `(name)` for an
      update) clones into `.oxplow/tmp/` and returns `ExtensionReview`:
      the extension as it would load (`errors` = load errors), its commit
      `sha`, and `problems` from a dry run of its lenses/advisories
      (reported, not blocking: a lens over an unsynced source can't run
      yet). Settings shows it as an inline panel spelling out each exec
      source's program, hosts and credentials, derived sources,
      advisories, gauges and slots (`reviewModel`); Install/Update
      confirms. `install_extension` / `update_extension` take the
      `reviewed_sha` and refuse a clone at any other commit, or one with
      load errors.
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
    - Installing runs nothing. An exec source (and a shared extension's
      advisories) runs only after a person approves it in Settings →
      Data. Approvals are stored per machine outside the repo, MACed under
      a keychain key, and bound to a hash of that version of the program,
      so a changed script needs approving again (`exec_consent.rs`,
      `source_runner.rs`; see [architecture.md](./architecture.md) → "A
      repo's config never runs a program without consent").
- **Core explorer (stays in core, deliberately simple).**
  - **Explore Data** (`explore-data` page, `ExploreDataPage.tsx`):
    - Lists every model from `v_model`, with its columns from
      `v_model_column` (SQL, like everything else; live as extensions
      compile).
    - A picked model shows its **lineage** (`ModelLineage.tsx`, from
      `v_model_lineage`): the models it reads (links that open them), the
      tables it reads (plain text), and the models that read it.
    - Picking one runs `SELECT * … LIMIT 50`; the SQL box then accepts
      any read-only query (Cmd/Ctrl+Enter runs it), metrics included —
      `SELECT bucket, MEASURE('<key>') FROM metric_grid('week')` — and
      re-runs when what it read changes.
    - "Show as" switches the viz: `table`, `list`, `number`, `markdown`,
      and the charts `bar`, `line`, `treemap` (P6.F1), whose columns are
      picked from the result (`chartDefaults`, then a select per role;
      a chart keeps its columns while a re-run still returns them).
    - **Chart a metric** seeds the explorer's own metric query
      (`metricTemplate`: `metricSeriesSql` as a line). While the SQL is
      still that template, **Slice By** picks a dimension (`v_dimension`)
      and regenerates it with `metric_grid`'s second argument
      (`sliceTemplate`); edited SQL is free SQL and is never rewritten
      (the line's series picker slices it instead).
    - Pin to Dashboard and Save as Lens keep the chart (the tile's
      `chart` option, the lens's `chart:`).
    - **Raw tables** (a checkbox, in warning color) reads physical tables
      too — the person's debugging switch, IPC `query_sql { raw }` only,
      never an agent's. A raw result carries a banner, and **Save as
      Lens** and **Pin to Dashboard** are disabled with the reason
      (`keepBlockedReason`): a lens or a tile reads only models.
    - **Pin to Dashboard** adds the query as a `query` tile shown with
      the chosen viz (`PinToDashboard.tsx`, shared with the lens page).
    - **Save as Lens** writes the file through the UI-only `save_lens`
      IPC, then opens the new lens. It creates the extension if missing
      (with the same v2 manifest `oxplow plugin new` writes —
      `extensions::scaffold_manifest`, so a saved lens starts as a
      checkable extension with an `intent` to fill in), refuses to
      overwrite a lens, and refuses git-installed extensions.
  - **Thread answers** (P6.C1, `commands/lens.rs`,
    `oxplow-db/src/thread_answer_store.rs`). An agent answers with a lens
    rather than pasting rows: MCP **`show_lens { lens | spec, params? }`**
    runs `lens.show`, which stores a `thread_answer` (V124, model
    `v_thread_answer`, ref `answer:<id>`) and returns the text rendering.
    - **One lens shape, `LensSpec`**: what a lens file holds, what an
      answer stores and what `save_lens` writes (`Lens::from_spec`,
      `Lens::spec`). `extensions::save_lens(root, ext, slug, &spec,
      &LensOrigin)` refuses a spec with a `spec_problem`, writes the YAML
      pruned of nulls and empties, and — when the file doesn't then load —
      removes it and reports the loader's errors. Serialize a spec's JSON
      values with `plain_json` (serde_json's `arbitrary_precision` makes
      numbers maps under `serde_yaml`).
    - An answer's query is agent SQL: `lens.show` checks it with the same
      read-only authorizer as `query_sql` before storing anything.
      `run_answer` (RPC and MCP) re-runs it.
    - **Keep This** is `lens.keep`: the answer becomes a private lens in
      `my-lenses` (params → defaults, `intent.origin` the thread).
      **`lens.share`** (a person's only) moves a lens into a shared
      extension, created with `sharing: shared` and `engine`
      (`ManifestScaffold.shared`), and refuses one that doesn't load or
      reads beyond models.
    - **UI** (P6.C2, `components/Answers/`): a terminal thread's
      `AgentPage` shows the **Answers strip** above the terminal — the
      thread's answers from `v_thread_answer` (`threadAnswers.ts`,
      `useThreadAnswers`), newest first, hidden while there are none,
      collapsible (Escape inside it collapses; remembered per thread in
      `oxplow.answers.collapsed`). An ACP thread renders each answer
      inline under the `show_lens` call that made it (`answerOfTool`
      finds `answer:<id>` in the call's result). Each `ThreadAnswer`
      re-runs through `run_answer` with `useRerunOnChange`, and offers
      **Keep This** (an inline name — empty takes it from the title —
      Enter keeps via `lens.keep`, Escape cancels) or, once it is a lens
      (kept, or an existing lens shown), a link to it. The agent tab has
      no route context, so `AgentPage` takes `onOpenPage`.
    - Explore Data's **Save as Lens** (`save_lens` RPC) uses the lens's
      title as a new extension's `intent.purpose`.
  - **Lens tiles.** A lens can be pinned to a dashboard: "Pin to
    Dashboard" on a lens page, or MCP `add_dashboard_item(kind: "lens",
    lens_id)`.
    - The tile is kind `lens`, with `lensId` stored in `options_json`.
    - `LensTile` renders it with the shared `LensResultView`, capped at 8
      rows, against the primary stream, since dashboards are
      project-global.
    - The launcher's new **Data** category holds Explore Data, Metrics
      and Dashboards.
- **Sources (collectors).** Declared under `collectors:` in `extension.yaml` (v1: `sources:`) and parsed
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
  - `extension.yaml` declares `slot_mounts: [{slot, lens}]` (v1:
    `slots`). `SLOTS` in
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
    changed, its claims and recorded decisions, and what to report; an
    effort on another provider's work item has no task row, so the lens
    LEFT JOINs `v_task` and names the `work_item` ref instead, tsk458.
    What Deviated stays silent there: with no task text, no area is
    stated)
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

Manifest **version 2** (P1.11, tsk413; `crates/oxplow-app/src/extensions/manifest_v2.rs`).
Every contribution is declared as data, unknown keys are errors, and the
loader resolves cross-references with `file:line` messages.

```yaml
manifest: 2
name: oxplow-analytics   # must equal the folder name
description: …
sharing: shared          # private (default) | shared — see below
engine: ">=0.7"          # the oxplow it targets; required when shared
intent:                  # required
  purpose: What question it answers, or what job it does
  origin: effort:eff42   # the thread/effort ref that created it, or null
  examples:              # inputs → expected outputs; fixtures for `plugin test`
    - { name: …, input: {…}, expect: … }
# stable kinds (permanent API)
measures:    [...]   # same schema as .oxplow/project.yaml
metrics:     [...]   # `key:` definitions; sourceMeasure must be declared here or oxplow.*
gauges:      [...]   # starlark / jaq only; emits must be declared here or oxplow.*
dimensions:  [...]
collectors:  [...]   # v1 `sources`: exec / starlark / jaq programs → entities (see semantic-layer.md)
slot_mounts:         # v1 `slots`: mount lenses into core pages
  - { slot: effort-review, lens: change-review }
advisories:  [...]   # see "Advisories"
launcher:            # entries for non-lens targets (a page, a command); a lens uses its own launcher: block
  - { label: …, category: Data, target: page:… }
models:     [...]   # SQL models: ModelDecl entries + models/<name>.sql → v_<ext>_<name> (semantic-layer.md "Extension models")
commands: …  pages: …  panels: …  config: …   # parsed as data; runtimes land in later phases
# experimental kinds — a PRIVATE extension only
providers: [...]    # external providers over the provider protocol (providers.md); the others below are parsed as data only
effects: … event_types: … ref_kinds: … custom_components: … decorators: … replacements: …
```

Lenses aren't listed here: every `lenses/*.yaml` file in the folder is
loaded. A lens gets into the launcher through its own `launcher.category`
(or stays out with `hidden: true`).

**Private vs shared.** `sharing: private` is the usual case (this
project, this person, agent-built): experimental kinds allowed, fast
evolution. `sharing: shared` (committed for a team, installed from git,
bundled — bundled *must* be shared) is held to the strict lifecycle:
stable kinds only, `engine` declared and satisfied. Using an
experimental kind in a shared extension is an error naming the key and
its line.

**What the loader checks** (`Extension.errors` / `Extension.warnings`):
- shape: unknown keys, `manifest` version, `intent` present and
  `intent.origin` a canonical ref; an empty `intent.examples` is a
  warning;
- lifecycle: sharing rules above; `engine` is `>=MAJOR.MINOR[.PATCH]`;
- cross-references: a `slot_mounts` lens exists (and declares a param
  the slot binds); a grid's `children` exist — in this extension or,
  once everything is loaded, in another; a `launcher` target is a
  canonical ref. A metric's `sourceMeasure` or a gauge's `emits` that
  is neither declared in the extension nor an `oxplow.*` built-in is a
  **warning** (it may come from the project's or another extension's
  `measures:`, which resolves when the catalog is assembled).

**Decision (2026-09-28): `advisories` is a stable kind.** The plan sketch
had it experimental, but the bundled `oxplow-analytics` — shared by
definition — ships on it, and a first-party extension depending on a
kind is exactly the evidence promotion requires (target §10.1, §12).

**v1 still loads, through the migrator.** A manifest with no `manifest:`
key is v1 (`sources`, `slots`, no intent). The loader runs
`extensions::migrate_v1::migrate_v1_to_v2` on its text in memory, reads
the result as v2, and carries a warning to run `oxplow plugin migrate`
(which writes the same text to the file; see "The SDK"). The migration is **textual**
so a person's consent survives it: it prepends `manifest: 2`, inserts
`sharing: private` and an `intent` skeleton (`purpose` from
`description`, `origin: null`, `examples: []` — the agent fills those
in) after the header, and renames the top-level `sources:` →
`collectors:` and `slots:` → `slot_mounts:`; every other byte,
comments included, is unchanged, and it is idempotent.
`Extension.manifest_version` says which path a loaded extension took.

**Consent is unaffected by the migration.** `approval_hash` never
included `extension.yaml`, and the advisory program hash is over the
advisories' content, so rewriting a manifest from v1 to v2 asks for no
re-approval (tested).

## The SDK

`crates/oxplow-sdk` (P1.14, tsk416; target §10.5) is the one
implementation behind three doors: the `oxplow plugin new|check|migrate|test`
CLI, the RPC/MCP `validate_extension`, and `save_lens`'s manifest. Every
door gives an author the same report, so an agent editing from a terminal
and one calling MCP read identical `file:line: what — fix` lines.

- **`scaffold(root, Kind::Lens|Extension, name, origin)`** writes
  `oxplow/extensions/<name>/extension.yaml` (v2, `sharing: private`, an
  `intent` whose `origin` is the ref passed with `--origin`, one
  example), `fixtures/basic.yaml` (the example as a fixture for the
  future `plugin test`), and for a lens `lenses/<name>.yaml` (open tasks
  in the viewer's stream). It refuses an existing folder, a bad name and
  a non-ref origin; what it writes passes `check` with no warnings.
- **`check(root, name, catalog, layer: Option<&SemanticLayer>)`** is
  `catalog.named` (manifest shape, lifecycle, cross-refs, lens shape)
  plus, with a layer, `extensions::validate_extension`'s dry run of the
  extension's models (P4.9: compiled as temp views beside the other
  enabled extensions', on the read-only database — resolution, lineage,
  and the contract, so a changed contract at a published version fails
  here before it publishes; `models::check_extensions`) and of every
  lens and advisory. It returns a `CheckReport { ok, errors, warnings,
  sql_checked, extension }`; `render_findings` prints it as text
  (`error: <file:line …>` lines then a one-line summary) or JSON.
- **`migrate(root, name)`** writes `migrate_v1::migrate_v1_to_v2` to the
  file; `changed: false` when it was already v2.
- **`scaffold(…, Kind::Provider, …)`** (`plugin new provider <name>`,
  P5.D5) adds a `providers:` entry (id = the name with `_` for `-`,
  capability `work_items`), `provider.json` (create / update /
  transition, the core `work_item.recorded@1`, an object config
  schema), a stub `bin/provider` that exits 1, `fixtures/basic.yaml`
  invoking `create` and expecting `{ ref: $any }`, and
  `fixtures/provider-<id>.yaml` (`config: {}`).
- **`plugin_test::test_extension(root, name, layer, bless)`** (`plugin
  test <name> [--bless] [--json]`, P5.D5) runs `check` and then, per
  declared provider, the conformance kit ([providers.md](./providers.md)
  "The conformance kit"): its handshake against its declarations, its
  `check` of `fixtures/provider-<id>.yaml`, every intent example that has
  a fixture (`input: { command, input }`, `expect`; `$any` matches
  anything), every message against the protocol's schema goldens, the
  golden transcript `fixtures/transcripts/<id>.jsonl` (`--bless` writes
  it), and the capability's conformance suite through a throwaway host.
  Without providers, intent examples aren't run (a warning: nothing
  declares a runtime). `TestReport { ok, errors, warnings, blessed, ran
  }`; `render` prints it like `check`'s. Exit 0 / 1 / 2 as `check`.
- **The CLI** is `apps/desktop/src-tauri/src/plugin_cli.rs`, dispatched
  by `main.rs` before Tauri boots exactly like `oxplow hook`. It takes a
  bare name (under `--root` or the cwd) or the extension folder's path
  (the project is read off `…/oxplow/extensions/<name>`). `check` opens
  `<root>/.oxplow/local.sqlite` **read-only** (`Database::open_read_only`:
  no migrations, no writes, refused unless its schema version matches
  this build — the CLI may not be the app's version) so lens SQL is
  dry-run against the real project; without a usable one it warns on
  stderr with the reason and checks everything else. Exit 0 clean, 1 with errors (or an SDK error such as an unknown
  extension), 2 usage.
- **`validate_extension`** (RPC and MCP) returns the `CheckReport`, not
  the bare `Extension`; the extension is inside it.

The `oxplow-extension` skill requires `check` after every edit.

## Answerability

`crates/oxplow-sdk/src/answerability.rs` (P5.F1) checks that an agent
reading the right skill can answer the questions a capability exists
for. A questions file is a list of `{ question, skill, reaches: { sql } |
{ command, input }, shape: { columns } }`:

- **Core capabilities**: `crates/oxplow-plugin/assets/questions/<capability>.yaml`
  (`vcs`, `work_items`, `knowledge`, `code_intel`;
  `oxplow_plugin::CAPABILITY_QUESTIONS`), each naming a shipped skill
  (`oxplow-codebase` — history and code, added for this —
  `oxplow-runtime`, `oxplow-wiki-capture`). The in-tree test
  (`every_capability_question_reaches_what_its_skill_names`) runs them
  against a throwaway `Services::in_memory`.
- **Per question**: the skill's text names every model the SQL reads
  and the command it runs — as a whole word, in any case
  (`answerability::names`: `v_commit_file` doesn't name `v_commit`); a
  model is a table the query reads (`models_in`: after `FROM`, `JOIN` or
  a comma in a `FROM` list), never a `v_*` word in a string, a comment
  or an alias (tsk570); the SQL runs through the gateway and
  returns exactly `shape.columns`; a command's input validates against
  its schema. A question whose skill doesn't lead there is a finding,
  which is the point: when a capability grows, its questions say which
  skill must learn about it.
- **Live** (`OXPLOW_LIVE_ANSWERABILITY=1`): `answerability::live` asks
  this machine's `decide` model (`AiService::for_this_machine`: the
  global `ai.yaml`, keychain read only), given only the skill text and
  the catalog (every `v_model` view and every command an agent may run),
  what it would reach for first; it must pick the same model or command.
- **An extension's own** `questions.yaml` runs in `oxplow plugin test`:
  `skill` is a markdown file in the extension, SQL runs against the
  project's database when there is one (else a warning), and a command
  is one of its providers' `<id>.<name>` (schema from its declarations).

## Lenses

A **lens** is a user- or agent-built way of looking at your work: a
query over the semantic layer plus how to show it. (Not "view", which is
taken by the SQL `v_*` views; not "data app".)

A lens file (`LensFile`, `deny_unknown_fields`) takes `title`,
`description`, `query`, `viz`, `params`, `columns`, `empty`, `chart`,
`children`, `launcher`, `hidden`, `actions` and `alert`.

- `params`: `name`, `label`, `default`. Untyped: a value is bound as-is.
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
- `viz`: `table`, `list`, `number`, `markdown`, `bar`, `line`, `treemap`,
  `grid` (child lenses, `children`), `tree`, `timeline`, `detail`,
  `steps`, `hunks` or `form` (`form: { command, defaults? }`; no `query`
  needed). Each component names the columns it draws from in
  its own block: `chart` (`x`, `y`, `series`, `label`, `size`, `group`),
  `tree` (`id`, `parent`, `label`), `timeline` (`at`, `label`, `ref`),
  `steps` (`label`, `status`), `hunks` (`path`, `from`, `to` — two
  revisions, `working` / `snap:<id>` / `git:<rev>`).
- `columns`: `key`, `label`, and `link: {kind, from, line?, base?, head?}`
  to a core page, so rows are page-graph links. Kinds: `task`, `file`,
  `wiki`, `effort-diff`, `commit`, `metric`, `page`, `diff-at`,
  `compare`.
- `actions`: commands the lens offers, `{ id, label, command, input?,
  row? }`, with `{{param.x}}` / `{{row.x}}` placeholders; run as the lens
  for whoever pressed, so they grant no power.
- `alert`: a row-count or threshold condition that shows a rail badge
  (nudging the agent is what advisories are for).

> **Target (not built yet).** From the original design: `hunks` (an
> ordered file/range list that opens the diff at the range) and `steps`
> (a guided walkthrough) viz; a column `format`; a `decision` link kind;
> `followup-comment` and `open-diff` actions. (`copy-review-prompt` became
> `copy` on the Review Prompt lens.)

Lenses render through one core `LensPage` / `LensSlots` in oxplow's design
system, as a `lens:<ext>/<slug>` page kind. Bookmarks, backlinks, the
launcher and sibling navigation work unchanged. Every lens has an "Improve
with Agent" action that pastes `[oxplow lens <ext>/<slug>]` and its params
into the agent's context through the existing add-to-context path; oxplow
never types into the agent.

## Slots

Slots are the **only** way an extension reaches a core page. Core pages
declare them (`SLOTS` in `extensions.rs`) and render whatever is mounted,
in declaration order (there is no `order` field). With nothing mounted,
the page is plain.

| Slot | Core page |
|---|---|
| `effort-review` | diff-view for an effort |
| `task-detail` | TaskPage |
| `thread` | PlanPane (compact strip) |
| `commit` | GitCommitPage |
| `uncommitted` | UncommittedChangesPage |
| `rail` | rail HUD Alerts section |
| `settings` | Settings: a section per mounting extension, titled with its name, before AI (tsk330; `SettingsSlotSections`, `slotRuns(…, extension)`). No params |

The launcher isn't a slot: a lens lists itself with `launcher.category`.

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

- `query_sql`: read-only SQL over the published models. `v_model` /
  `v_model_column` / `v_model_lineage` describe every view (core `v_*` and
  extension `v_<ext>_<entity>`) with its column docs and inputs;
  `v_dimension`, `v_metric_spec` and `v_measure` list the dimensions,
  metrics and measures, extension-declared ones included, each with its
  scope.
- A metric's numbers are SQL too: `SELECT bucket, MEASURE('<key>') FROM
  metric_grid('day'[, '<dim>'])` — sliced by a dimension, in any bucket.
  Changing metrics is the `metric.*` commands (`run_command`).

**Working with lenses**

- `list_lenses`: every lens (`<ext>/<slug>` id, title, description, viz,
  params).
- `get_lens(id)`: the lens definition: query, params with defaults, viz,
  columns/links and the file it lives in.
- `run_lens(id, params?, format?)`: **what the UI shows** for those
  params, as its text rendering by default — `LensText { lens, title,
  params, columns, rowCount, truncated, reads, alert, text }`, no rows
  (the text carries them) — or, with `format: "json"`, the full
  `LensRun` with its rows.
- **Text rendering** (design rule 17, `crates/oxplow-app/src/lens_text.rs`,
  P6.A1): one renderer per kit component — a table or list is a markdown
  table of the displayed columns (the UI's `displayColumns` rule); a
  number or markdown lens is its first cell; a bar chart is its labels
  and values with the total; a line chart pivots to one column per
  series with each series' total; a treemap lists labels by size,
  largest first; a grid renders its children in order under `###`
  titles; a tree is an indented list; a timeline is `at — label (ref)`
  lines oldest first; a detail is `**label**: value` lines; steps are a
  numbered `[x]`/`[>]`/`[!]`/`[ ]` checklist; hunks are each file's
  unified diff between its two revisions (up to 20 files, 8 KB each).
  What a component needs beyond its rows — a grid's child runs, the
  hunks' diffs — is `lens_text::resolve`d first (`text_of`). Tables
  stop at 50 rows and say how many more; an empty result says the
  lens's `empty` text. It's what `run_lens`, `get_open_page` and `copy`
  return, so an agent reads a lens the way the person sees it.
- `get_open_page(thread_id)` **(current)**: what the human has open in
  that thread, for any page kind (`task:42`, `file:…`, `lens:…`). For a
  lens it adds `lens`, the lens re-run with the human's *current*
  params as its text rendering, so "look at what I'm looking at" works.
  - The UI reports the active page (`report_open_page`, UI-only) together
    with whatever that page published to `tabs/openPageDetail.ts`. The
    lens page publishes `{lensId, params}`.
  - The state is held in memory on the thread runtime
    (`ThreadRuntimeRegistry::open_page`).
  - A hidden-but-mounted page can't overwrite the report, because only
    the active page's detail is sent.
- **Row actions (current):** right-click a lens row → "Ask About This"
  (P6.D1): `[oxplow ref <ref>]` for the first ref the row links to, else
  `[oxplow lens <id> row: col=value, …]` (`rowAsk`).
- **The prompt catalog (current, P6.D2):** what a person can ask.
  - Core's are the capability questions
    (`crates/oxplow-plugin/assets/questions/*.yaml`,
    `oxplow_plugin::capability_prompts`); a question with `about: <ref
    kind>` is phrased with "this" and is offered on pages for that kind
    (its `reaches` keeps concrete fixture values for the answerability
    check, which also checks `about` names a registered kind).
  - An extension's are `intent.prompts: [{ prompt, about? }]` in its
    manifest (checked: non-empty, `about` a registered ref kind).
  - `prompt_catalog::prompt_catalog(extensions)` merges them (core first,
    then enabled extensions by name); RPC `prompt_catalog { stream_id }`
    (UI only) serves it.
  - The UI: the **Catalog** page (`page:catalog`, `pages/CatalogPage.tsx`:
    prompts by source, `v_model` by owner, `config.list_keys`), the nav
    bar's Ask menu (suggested prompts for the page's ref kind), and
    `EmptyState`'s prompts (usability.md → "Empty states").
- **Launcher entries (current, P6.D1):** the manifest's `launcher:` lists
  what isn't a lens, each `{ label, category, target }`, where `target` is
  exactly one of `{ ref }` (a canonical ref, opened as a page),
  `{ command, input? }` (run as the person, asking first when the command
  asks — `personCommands.ts` + `PersonCommandConfirm`, mounted once in
  `App`) or `{ prompt }` (put in the agent's input, never sent).
  `manifest_v2::launcher_entries` types them at load (a bad one is an
  error at its line and is dropped); the dry run (`validate_extension`,
  `review_extension`, with `CommandBus::input_schema` as
  `CommandSchemas`) checks that a command is registered and its input
  fits. `oxplow plugin check` has no running app to ask, so it says the
  commands weren't checked. The launcher (`components/extensionLauncher.ts`)
  merges ref entries into the page directory and lists the others as
  actions under their category.
- **Lens actions (current, P6.B1):** `actions:` are commands, run by an
  agent with `run_lens_action` as the lens acting for it (see "Actions
  are commands").

**Building**

- Agents author extensions and lenses **by editing files** under
  `oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  filing guard. The loader hot-reloads.
- **No scaffold tools (decided).** A scaffold tool would write project
  files outside the filing guard (and the write guard, and the caller's
  worktree). The skill carries the templates instead, and humans have
  Save as Lens in Explore Data. The same rule holds for metrics:
  the `metric.scaffold` command returns a template the agent writes itself
  (tsk391).
- `validate_extension(name)` returns load errors, schema errors and a dry
  run of every lens query, so the agent can check its work without the UI.
- `list_extensions`, `list_sources`, `run_source(extension, source_id)`
  (never approves).
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
teaches the format, the `v_*` contract and the loop "read `v_model` →
query_sql → write files → validate_extension → run_lens". "Improve with Agent" on a lens pastes
`[oxplow lens <ext>/<slug>]` plus its params into the agent's context.

**Getting newcomers there (tsk373).** "New Lens with Your Agent…"
(`lens.newWithAgent`, Tasks menu, so also in the launcher; needs a
thread) puts `NEW_LENS_PROMPT` (`lens/lensModel.ts`) into the agent's
input via `insertIntoAgent`. It never sends; the person finishes the
sentence. The Extensions empty state and Explore Data's intro offer the
same button, and getting-started's "Your first stream" has a "Build your
own view" step.

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
- Lens `actions:` as commands (P6.B1; the tsk329 registry retired).
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see "Advisories").
- Lens tabs carry params: `lens:<ext>/<slug>?k=v` (`lensRef(id, params)`),
  so a slot lens's heading opens its page with the slot's values, and
  history and bookmarks keep them.
