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
>   - sharing: team via the repo, world via the `extension.install` /
>     `extension.update` commands and Settings → Extensions;
>   - the core explorer: the Explore Data page (with Save as Lens) and
>     lens tiles on dashboards;
>   - **bundled extensions** (compiled in, read-only, reserved names) and
>     the `effort.review.details` **slot**; the bundled `oxplow-review` extension
>     is the effort review packet;
>   - `exec` **sources** that bring external records in as entities. The
>     mechanics and decisions are in
>     [semantic-layer.md](./semantic-layer.md) → "User and extension
>     sources", and a tested example is in `examples/extensions/github/`.
>   - **slots** (now `effort.review.details`, `vcs.commit.details`,
>     `vcs.status.details`, `work_item.detail.body`, `thread.plan.header`),
>     **advisories**, per-project **disabling**, and the
>     **`oxplow-analytics` extraction** (tsk280): every analytics page and
>     widget is now a lens in that bundled extension, and core works with
>     it disabled (checked headless, 2026-09-27).
>   - lens alerts (tsk316; the `rail` slot they mounted in became panels
>     in P6.G1), and extension-declared
>     measures, metrics and fact collectors (tsk311, gauges until P7.B3;
>     see "Contributing metrics").
> - **Current:** extension-declared dimensions (tsk328).
> - **Current:** lens action buttons (tsk329) and the `settings.section` slot (then `settings`)
>   (tsk330).
> - **Current (P1, 2026-09-28/29):** manifest v2 with `intent`,
>   `sharing` and the stable/experimental split (tsk413), the textual
>   v1→v2 migrator (tsk414, removed with the v1 reader in tsk865), the
>   per-root catalog cache (tsk415, tsk390) and the SDK: `oxplow plugin
>   new|check|test` (tsk416; "The SDK").
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
  (every file in the folder — a custom component's bundle too, so its
  file cap keeps that walk small) and `.oxplow/project.yaml`, so a hit costs ~60 µs instead of the ~3 ms
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
    extension it just wrote — the same for `preview_collector`,
    `review_extension`, `extension.install`, `extension.update`,
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
  - **Pages** (P6.G2, target §11.3): `pages: [{ id, title, icon?,
    category, lens }]` — a lens shown full-page at
    `page:ext.<extension>.<id>` (`Extension.pages`, `ExtensionPage`;
    checked at load in `parse_pages`: a kebab-case, unique id, a launcher
    category, a lens that exists), listed in the launcher under its
    category (`components/extensionLauncher.ts`).
  - **Panels** (P6.G1, target §11.3): `panels: [{ id, title, icon?,
    scope: project | stream | thread, body, badge? }]` put a lens in the
    left nav (`Extension.panels`, `ExtensionPanel`; checked at load in
    `parse_panels`: a kebab-case id, lenses that exist, a badge with an
    `alert`, and a `stream` / `thread` scope's lenses declaring
    `stream_id` / `thread_id`). The nav binds those itself
    (`panelParams`: the stream's and thread's row ids for the stream and
    thread it's shown for, re-run when they change) rather than leaving
    the backend to infer the thread from the selection, and runs every
    panel's lenses from one owner (`useExtensionPanelRuns`), so a badge
    runs once per refresh. The body renders compact; the badge's
    alert count shows on the panel, and the core **Alerts** panel lists
    every firing badge. oxplow-review's Waiting on You is a panel
    whose badge is its own lens.
  - **Actions are commands** (P6.B1, target §11.4): `actions:` declares
    `{ id, label, command, input?, row? }` — a button above the result, or,
    with `row: true`, an item in each row's menu — every row of every row
    component (table, list, tree, timeline, detail, steps, hunks) is
    focusable and opens it from a right-click or the Menu key / Shift+F10. A command
    that asks shows `CommandConfirm` from `LensResultView` itself, above
    the body, so it asks wherever the row is (an answer in the strip, a
    grid child, a dashboard tile), not only where the toolbar is shown.
    - `input` is the command's input. A string that is exactly
      `{{param.<name>}}` or `{{row.<column>}}` becomes that value, typed
      (a number stays a number); a string containing them has them spliced
      in as text. The values go into the command's input, never into SQL.
      One tokenizer reads them (`extensions::placeholders` /
      `whole_placeholder`), for load-time validation and run-time binding
      alike, so the two can't disagree.
      At load, a `{{param.x}}` must name a declared param and `{{row.x}}`
      needs `row: true`; `validate_extension` checks `{{row.x}}` against
      the result's columns; the command name must be well-formed. Any
      other shape is a field error.
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
    - Running a collector is the command **`collector.sync { owner, id }`**
      (External, `Invokers::ALL`; `collector_runner::CollectorRunner::sync`,
      which the scheduler uses too). It **never approves**: approving is
      `collector_runner::approve_reviewed` behind the UI-only
      `approve_collector` RPC (Settings → Data's Approve & Run approves,
      then runs the command). MCP `run_collector` runs the command as the
      agent.
    - UI: `LensToolbar` and the row menu in `LensResultView.tsx`; logic in
      `lens/lensActions.ts` (`performLensAction`, `copyLens`,
      `addLensToContext`, `rowRecord`).
    - The GitHub example's PR lens has a Sync PRs action
      (`command: collector.sync`).
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
    (an agent runs the command itself). Only a form and a grid (which renders its children, never rows of its own) omit `query`. The loader drops a
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
    git_ref?)` then the `extension.install { git_url, git_ref?,
    reviewed_sha, stream? }` command (`commands/extension_install.rs`,
    P8.A9: `External`, `Confirm::Always` — an agent's run becomes a
    proposal a person approves; Settings → Extensions' install box runs it
    confirmed, the review being the confirmation).
    - **Review first (tsk378).** `review_extension` (or `(name)` for an
      update) clones into `.oxplow/tmp/` and returns `ExtensionReview`:
      the extension as it would load (`errors` = load errors), its commit
      `sha`, and `problems` from a dry run of its lenses/advisories
      (reported, not blocking: a lens over an unsynced source can't run
      yet). Settings shows it as an inline panel spelling out each exec
      source's program, hosts and credentials, derived sources,
      advisories, fact collectors and slots (`reviewModel`); Install/Update
      confirms. `extension.install` / `extension.update` take the
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
    - `extension.update { name, reviewed_sha }` re-clones from the recorded source. The old
      folder is replaced only after the new clone validates, and only for
      git-installed extensions.
    - Installing is a write tool on MCP. The skill says to do it only when
      the user asks, and to offer to commit the result.
    - Installing runs nothing. An exec source (and a shared extension's
      advisories) runs only after a person approves it in Settings →
      Data. Approvals are stored per machine outside the repo, MACed under
      a keychain key, and bound to a hash of that version of the program,
      so a changed script needs approving again (`exec_consent.rs`,
      `collector_runner.rs`; see [architecture.md](./architecture.md) → "A
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
    - **Save as Lens** runs `lens.keep` with the query as a `spec` (P11,
      tsk943 — on the bus like Keep This, in the stream's worktree; its
      shape checked as `lens.show` checks one, its query through the SQL
      gateway as the explorer runs it, so a `metric_grid()` chart is kept,
      tsk987), then opens the new lens.
      It creates the extension if missing
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
      answer stores and what `lens.keep` writes (`Lens::from_spec`,
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
      reads beyond models. The target goes through
      `extensions::writable_extension_dir` first — the one check every
      writer of an extension's files runs (a valid name, not bundled, not
      installed), so a path or reserved name never reaches the
      filesystem — and a refused share removes only the files it wrote
      (never a directory that was there before).
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
    - Explore Data's **Save as Lens** (`lens.keep { spec, stream }`) uses
      the lens's title as a new extension's `intent.purpose`, and the
      caller's thread, when it has one, as its `intent.origin`.
  - **Lens tiles.** A lens can be pinned to a dashboard: "Pin to
    Dashboard" on a lens page, or `dashboard.add_item { kind: "lens",
    lens_id }`.
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
    - `extension.install` refuses the name;
    - `extensions::save_lens` (the writer behind `lens.keep`) refuses to
      write into a bundled extension.
- **Slots (current: see "Slots" below).**
  - `extension.yaml` declares `ui: { slots: [{slot, lens}] }` (v1:
    top-level `slots`). `SLOTS` in
    `extensions.rs` names each slot and the params it **offers**; a
    mounted lens gets the ones it declares and must declare at least one,
    or the mount is an error. Loaded as `Extension.ui.slots`.
  - `src/lens/LensSlots.tsx` renders a slot (each mount marked
    `data-slot="<slot>"`, which the "every enhancement off" smoke test
    looks for): every mounted lens, run with
    the slot params it declares (`slotRuns`), re-run on data events.
    DiffViewPage offers `effort_id` and `change_id`; TaskPage and
    WorkItemPage `ref` and `task_id` (body and sidebar); PlanPane
    `thread_id` (the compact `strip` variant, which hides lenses with no
    rows); GitCommitPage and UncommittedChangesPage `change_id`, plus
    UncommittedChangesPage's strip and GitHistoryPage's side column
    `stream_id` (a side column only when something mounts there —
    `useSlotMounted`).
    Numeric ids come from `numericRowId` (`tsk42` → 42).
  - **Latest request wins** (`src/request-guard.ts`, tsk370). LensSlots,
    LensPage and ExploreDataPage `begin()` each fetch and apply its result
    only while it's still the newest; new params (another thread, lens or
    stream) clear the old results first, so thread A's rows never show
    under thread B.
  - `change_id` comes from `src/lens/useChange.ts`: it calls
    `ensure_change` on mount and again whenever `v_change` changes
    (`shouldReensure`): the analysis landed, or the `change.analyze`
    consumer recomputed a working tree or open effort.
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
  `effort.review.details`:
  - Decisions Made (`v_decision`, `provenance = 'recorded'`)
  - Decisions Oxplow Noticed (`v_decision`, `provenance = 'inferred'`)
  - Unverified Claims (`v_claim` where `verified = 0`), each row with
    **Mark Verified** (`effort.verify_claim`)
  - Decisions Oxplow Noticed's rows carry **Confirm** / **Dismiss**
    (`effort.confirm_decision` / `effort.dismiss_decision`)
  - What Deviated (its own model `v_oxplow_review_deviation`: each
    effort's files against its work item's title and body — a file is in
    the area when the text names it or one of its directories at least
    two levels deep; none when the item names no area. Live, not
    materialized: 0.2 s over all 5.5k effort files of this repo's
    database)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `vcs.commit.details` and `vcs.status.details` slots)
  - Struggled Here (`v_struggle`)
  - Review Prompt (a copyable markdown prompt for reviewing the effort
    with a second harness: the task, the agent's summary, the files it
    changed, its claims and recorded decisions, and what to report; an
    effort on another provider's work item has no task row, so the lens
    LEFT JOINs `v_task` and names the `work_item` ref instead, tsk458.
    What Deviated stays silent there: with no task text, no area is
    stated)
  - Context Read (`v_context_read`)
  - Verify a Claim With Evidence (hidden, `viz: form` over
    `effort.verify_claim`, `claim` a param)

  **Its verdicts (P7.C5, the first bundled `commands:`)**, on an
  effort's page under **Commands** (`ui.commands` about `effort`):
  `oxplow_review.accept { ref, force? }` comments the review on the
  effort's work item then transitions it to `done`, refusing (`{ refuse }`)
  while a claim is unverified or an inferred decision unreviewed unless
  `force` (then it lists them in the comment);
  `oxplow_review.request_changes { ref, note? }` comments a checklist —
  each unverified claim, inferred decision, file outside the area, and
  the note — then transitions to `todo`. Both read one `input` query
  (`v_effort` + `json_group_array`s over `v_claim`, `v_decision`,
  `v_oxplow_review_deviation`), compose `work_item.comment` and
  `work_item.transition` — one transaction (and one undo) on oxplow's own
  item, steps through the provider on another provider's (not undoable;
  [commands.md](./commands.md) → "Composition", tsk713) — refuse an
  effort without a work item by name, are `confirm: always`, and are a person's or
  a lens's — never an agent's. `questions.yaml` + `README.md` (its skill)
  say what an agent can read of the packet; a bundled extension's
  questions are checked against a running registry by
  `the_bundled_extensions_answer_their_questions` (oxplow-sdk).

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

- **P6b (epic tsk592, 2026-10-01).** An extension can also *act* and
  *extend the shell*: `commands:` (Starlark composing core commands, see
  "Commands"), `ui:` — `slots` in one dotted namespace ("Slots"),
  `commands` in core menus ("Commands in core menus"), `decorators`
  (experimental) — and `custom_components:` rendered sandboxed by
  `viz: custom` lenses ("Custom components"). Installs, updates and
  provider approvals are reviewed by what they change ("Reviewing by
  effect"). An agent's run that needs a person waits as a proposal
  ([commands.md](./commands.md), "Proposals").
- **P8 (2026-10-02).** The experimental kinds run, in a private
  extension: `event_types:` ("Event types"), `ref_kinds:` ("Ref kinds")
  and `effects:` ("Effects"), through one swappable vocabulary. A change
  is reviewed by its rows, collector outputs and effects at any revision,
  on the CLI (`plugin check --effects`) and in an effort's review
  ("Reviewing by effect"). Models declare keys and may materialize on a
  clock or incrementally ([semantic-layer.md](./semantic-layer.md)).
  Installs and updates are the `extension.install` / `extension.update`
  commands.

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
`oxplow/collectors/*.star`). Extensions follow that convention, so they are
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
dimensions:  [...]
collectors:  [...]   # entity collectors (exec / starlark / jaq / read → entities) and
                     # fact collectors (`facts:`; starlark / jaq only; facts must be declared here or oxplow.*)
                     # see semantic-layer.md "Collectors"
ui:                  # what it adds to the core UI
  slots:             # lenses mounted into core pages; see "Slots"
    - { slot: effort.review.details, lens: change-review }
  commands: …        # its commands in core menus (stable; P6b)
  decorators: …      # labels from its models on core refs (stable since P10; see "Decorators")
  replacements: …    # experimental: a lens in place of a named core component (see "Replacements")
advisories:  [...]   # see "Advisories"
launcher:            # entries for non-lens targets; a lens uses its own launcher: block
  - { label: …, category: Data, target: { ref: page:… } }        # a page (a ref of a kind that opens as one)
  - { label: …, category: Work, target: { command: …, input: { … } } }   # a command, run as the person
  - { label: …, category: Code, target: { prompt: … } }          # a one-line prompt, put in the agent's input
models:     [...]   # SQL models: ModelDecl entries + models/<name>.sql → v_<ext>_<name>; `key: [cols]`; `materialize: on_change | { every: 1h } | { incremental: <col> }` stores one (semantic-layer.md "Extension models", "Materialized models")
pages: …  panels: …  # running (P6.G1/G2): see "Panels" and "Pages"
commands:  [...]   # Starlark scripts composing core commands (see "Commands")
config: …          # parsed as data
ref_kinds: …        # kinds of thing a ref can name (stable since P10; see "Ref kinds")
event_types: …      # its own namespace's event types (stable; see "Event types", P8)
# experimental kinds — a PRIVATE extension only
providers: [...]    # external providers over the provider protocol — a program, or an MCP server behind oxplow's adapter (providers.md)
effects: …          # scripts reacting to logged events by composing commands (see "Effects", P8)
custom_components: …   # sandboxed components for `viz: custom` lenses (see "Custom components")
# and ui.replacements above
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
- cross-references: a `ui.slots` lens exists (and declares a param
  the slot binds); a grid's `children` exist — in this extension or,
  once everything is loaded, in another; a `launcher` target is a
  canonical ref. A metric's `sourceMeasure` or a fact collector's `facts` that
  is neither declared in the extension nor an `oxplow.*` built-in is a
  **warning** (it may come from the project's or another extension's
  `measures:`, which resolves when the catalog is assembled).

**Decision (2026-09-28): `advisories` is a stable kind.** The plan sketch
had it experimental, but the bundled `oxplow-analytics` — shared by
definition — ships on it, and a first-party extension depending on a
kind is exactly the evidence promotion requires (target §10.1, §12).

**Only v2 loads (tsk865).** A manifest without `manifest: 2` is a load
error (`extension.yaml:1`); so is any key v2 doesn't have (`sources:`, a
top-level `slots:`, `gauges:` — `deny_unknown_fields`). There is no
reader or migrator for v1, and no recogniser for an old shape: a slot
name that isn't one is an unknown slot, an action that isn't `{ id,
label, command, input?, row? }` a field error (tsk920).
`Extension.manifest_version` is the
version the file declares.

## The SDK

`crates/oxplow-sdk` (P1.14, tsk416; target §10.5) is the one
implementation behind three doors: the `oxplow plugin new|check|test`
CLI, the RPC/MCP `validate_extension`, and `save_lens`'s manifest. Every
door gives an author the same report, so an agent editing from a terminal
and one calling MCP read identical `file:line: what — fix` lines.

- **`scaffold(root, kind, name, origin)`** (`plugin new <kind> <name>`)
  writes `oxplow/extensions/<name>/extension.yaml` (v2, `sharing:
  private`, an `intent` whose `origin` is the ref passed with `--origin`,
  one example) and, but for a bare extension, that example's fixture
  `fixtures/basic.yaml` — then the kind's starter (P7.C6):
  - `lens` — `lenses/<name>.yaml`, open tasks in the viewer's stream, with
    a Start row action (`work_item.transition`, a `ref` column it selects
    but doesn't show);
  - `collector` — a Starlark collector `items` over `v_task` declaring
    entity `item`, a model `open_items` over `ref('item')` and a lens over
    the model; its example runs the collector over fixture rows;
  - `command` — `commands: [note]` whose `handlers/note.star` composes
    `work_item.comment`, with an example and a `ui.commands` entry on
    `work_item`; its intent example dry-runs it;
  - `effect` (P8.D12) — `effects: [on-done]` on `work_item.transitioned`
    `where: { to: done }`, whose `effects/on-done.star` composes
    `work_item.comment`; its intent example dry-runs it on a fixture
    event;
  - `provider` — below; `extension` — the manifest only.
  It refuses an existing folder, a bad name and a non-ref origin. **What
  it writes passes `check` with no warnings and `plugin test` clean**
  (a provider once a program replaces its stub) —
  `crates/oxplow-sdk/tests/just_works.rs` holds each kind to it, through
  to loading in a real oxplow: the lens's row action names a registered
  command, the collector syncs and its model publishes, the command runs
  through the bus.
- **Recorded fresh-agent runs** (`crates/oxplow-sdk/fixtures/just-works/
  <kind>/`): `scripts/record-just-works.sh <kind>` gives `prompt.md` to
  `claude -p` in an empty git project — `--safe-mode`, so nothing but
  the oxplow-extension skill (appended to its system prompt) and the
  `oxplow` CLI built from the checkout — and keeps `run.json` (without
  the denied commands, which carry local paths, the session id or the
  cost — `recorded_runs_carry_no_local_paths_session_or_cost`), the
  extension it wrote (`produced/`), and `check.txt` / `test.txt`;
  `notes.md` is written by hand. `recorded_agent_runs_still_check_and_
  test_clean` replays every `produced/` against today's oxplow. Recorded
  once (it costs a real run): `collector` (2026-10-02, 64 turns, clean).
- **`check(root, name, catalog, layer: Option<&SqlGateway>)`** is
  `catalog.named` (manifest shape, lifecycle, cross-refs, lens shape;
  a Starlark collector's script must parse and define `transform`, an
  error at the collector's line) plus `extensions::validate_extension`'s
  dry run — **always** (P7.C6): on `layer` (the project's database,
  read-only), else on a fresh `Database::in_memory()` (`DryRun::{Project,
  EmptyDatabase}`). The dry run first compiles, in a rolled-back read,
  the extension's models beside the other enabled extensions' (P4.9 —
  resolution, lineage, and the contract, so a changed contract at a
  published version fails here before it publishes) over **empty
  stand-ins for every declared entity that hasn't synced**
  (`models::EntityStub`: a typed temp table under the entity's view);
  `models::check_extensions` returns those views (`CheckedModels.views`).
  Every later query of the check — command inputs and examples,
  advisories, lenses — reads through them as an **overlay**
  (`SqlGateway::with_overlay`, `SqlQuery::temp_views`: recreated on the
  query's own connection and dropped after), so a fresh collector →
  model → lens checks clean before its first sync and a lens may read
  its own unpublished model. It returns a `CheckReport { ok, errors,
  warnings, dry_run, extension }`; `render_findings` prints it as text
  (`error: <file:line …>` lines then a one-line summary) or JSON.
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
  `skill` is a markdown file in the extension, SQL runs on the test's
  throwaway oxplow, and a command is one on its bus — core's or the
  extension's own `commands:` — or one of its providers' `<id>.<name>`
  (schema from its declarations).

**`oxplow plugin test <name>`** (P5.D5, P7.C6; `plugin_test.rs`) runs on
**a throwaway oxplow** (`Host`): a temp project with a copy of the
project's `oxplow/extensions/`, `Services::in_memory` over it, every
declared entity published empty (`collector_runner::publish_declared_empty`
— as if each collector had run and found nothing; never on a real
database), the extension models published and commands registered as at
boot. It never opens the project's database. It runs `check` there
(with that registry, so `commands:` examples dry-run and `ui.commands` /
launcher commands are checked), then — only if `check` is clean — each
`intent.examples[*]` by its fixture `fixtures/<name>.yaml`:

- `input: { lens: <slug>, params? }`, `expect: { columns?, rows: n |
  $any }` — the lens runs; its row count (and columns, when expected)
  must match;
- `input: { collector: <id>, rows? }`, `expect: { entities: { <name>:
  n | $any } }` — a derived collector runs over `rows` (standing in for
  its `input` query, `preview_collector(…, rows)`), storing nothing, its
  rows typed against the declaration; an exec collector's example is a
  warning, not run (it needs a person's approval);
- `input: { command, input }` — a provider's, run in its session
  (providers.md "The conformance kit");
- `input: { effect: <id>, event: { type, payload, subject? }, rows? }`,
  `expect: { commands: [names] } | { skip: <part of the reason> } | {
  reacts: false }` (P8.D12) — whether the effect reacts (`on`/`where`),
  then `effects::dry_run`: its `input` rows (or `rows`), its script, the
  composed calls checked against the registry; nothing runs;
- `input: { event_type, v?, payload }`, `expect: { valid: true | false,
  upcast?: <payload> }` — the payload against the extension's declared
  schema (the newest version without `v`), and its upcast;
- `input: { wikilink }`, `expect: { ref: <canonical ref> | null }` — what
  `[[…]]` names with the extension's ref kinds beside core's;
- no fixture is a warning; an `input` naming none of these is an error.

Then `questions.yaml`, each provider's conformance kit, and (P8.B5) each
`materialize: { incremental: <column> }` model on its fixture
`fixtures/model-<name>.yaml` — `before:` and `after:`, each `{ <table>:
[{ column: value }] }`: in a rehearsal (rolled back), `before` is written
and the model built whole into a temp table keyed like its `m_<view>`,
`after` is written, the rows past its watermark are appended, and what
it holds is compared with its SELECT's rows now
(`models::incremental_matches_full`). A difference is an error at the
model's line in `extension.yaml` ("misses n row(s) a full refill holds"
— a watermark that doesn't grow as rows arrive drops a row below it
unseen, which nothing at run time can tell); an append that hits the key
passes (the runtime refills then); no fixture is a warning.

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
  `steps`, `hunks`, `form` (`form: { command, defaults? }`; no `query`
  needed) or — experimental, private only — `custom` (`custom: {
  component, props? }`, one of the extension's `custom_components`; see
  "Custom components"). Each component names the columns it draws from in
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
- `alert`: a row-count or threshold condition that shows a panel badge
  (nudging the agent is what advisories are for).

> **Not built.** From the original design: a column `format`; a
> `decision` link kind; `followup-comment` and `open-diff` actions.
> (`hunks` and `steps` are built, above; `copy-review-prompt` became
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
the page is plain. **Slot names are one dotted namespace,
`<capability>.<page>.<region>`** (P6b.C1): a mount naming anything else
is an unknown slot (listing the known ones), and a top-level
`slot_mounts:` / `decorators:` / `replacements:` is an unknown field —
they live under `ui:`.

| Slot | Core page | Params |
|---|---|---|
| `effort.review.details` | diff-view for an effort | `effort_id`, `change_id` |
| `work_item.detail.body` | TaskPage / WorkItemPage, below the body | `ref`, `task_id` (null for another provider's item) |
| `work_item.detail.sidebar` | TaskPage / WorkItemPage, in the side rail | `ref`, `task_id` |
| `thread.plan.header` | PlanPane (compact strip) | `thread_id` |
| `vcs.commit.details` | GitCommitPage | `change_id` |
| `vcs.status.header` | UncommittedChangesPage, a strip above everything | `stream_id` |
| `vcs.status.details` | UncommittedChangesPage | `change_id` |
| `vcs.history.sidebar` | GitHistoryPage, a side column shown only when something mounts there (`useSlotMounted`) | `stream_id` |
| `diff.file.header` | a file diff (`DiffPane`), a strip under its header (P9.A2; `DiffFileHeaderSlot`) | `path`, `left_revision`, `right_revision` (each a revision in its wire form: `working`, `snap:<id>`, `git:<sha>`), `stream_id` |
| `settings.section` | Settings: a section per mounting extension, titled with its name, before AI (tsk330; `SettingsSlotSections`, `slotRuns(…, extension)`) | none |

The launcher isn't a slot: a lens lists itself with `launcher.category`.

## Replacements (experimental)

`ui.replacements` (a private extension only; P9.A1,
`extensions/replacements.rs`) puts a **lens in place of a named core
sub-component** — never a whole page:

```yaml
ui:
  replacements:
    - { target: work_item.board, lens: board }
```

The targets are one table, `oxplow_domain::replaceable::REPLACEABLE`:
each a `target`, the `capability` it belongs to, and its **props
contract** — the params the replacement lens is given instead of any
host state.

| Target | Core component | Capability | Props |
|---|---|---|---|
| `work_item.board` | the Board's columns of cards (`WorkBoard`) | `work_items` | `scope` (`thread` / `backlog` / `all`), `thread_id` (the thread when `scope` is `thread`, else null) |
| `work_item.detail.state` | a work item's State (`WorkItemPage`'s rail; P10) — never its Move To, which stays oxplow's canonical states, so a replacement can't strand an item (tsk919) | `work_items` | `ref` (the item) |

The history graph and the conflict resolver §11.2 names wait for a
provider that needs them. Every target has a label a person reads
— its `label` in the same table ("board", "state control"), carried on
each `UiReplacement` so the desktop names it from there (tsk935; no
second list to keep in step). The Linear example replaces
both: its Board by Linear's states, and the state control with its
`state` lens — the Linear states a synced issue of the team is in (Linear
publishes no list of them), each with **Move Here** (`work_item.transition`
with the `native_state`). A state no synced issue is in has no row, so
oxplow's Move To stays beside it: the canonical states every provider
takes.

**At load** (`parse_replacements`, errors at the entry's line): the
target is in the table; the lens is one of the extension's and declares
**every** prop (a slot's lens declares at least one; a replacement takes
the whole contract — a Board that ignores `scope` isn't the Board); the
extension brings a provider of the target's capability (`providers:`);
a target is replaced once per extension. The viewer's stream isn't a
prop: a lens that declares `stream_id` gets it, as everywhere. Valid ones are
`Extension.ui.replacements` (`UiReplacement { id: <ext>/<target>,
extension, target, capability, lensId }`).

**A person can turn one off**: the project key `replacementsOff:
[work_item.board]` (person-only, like `activeProviders` — it decides
what renders a core region; validated against the table, an unknown
target is an error listing the real ones). Listed, oxplow's own
component shows. Settings → Integrations lists the replaced components
under the active-provider chooser, each with "Always use oxplow's own
<component>" (`config.set` / `config.unset` as the person).

**At render** (`lens/useReplacement.ts`, `components/Replaceable.tsx`).
A core page wraps the component: `<Replaceable target props streamId
fallback={<WorkBoard …/>}>` (`BoardPage`). `useReplacement` reads the
enabled extensions' replacements of the target and, only when one
exists, the capability's providers (`v_capability_provider`, re-read
when it changes) and `replacementsOff` (re-read on `configChanged`):
- **only the deciding provider's extension replaces** — the candidate
  whose `extension` is that provider's row's. A component showing one
  provider's thing passes it (`<Replaceable provider={item.provider}>`
  on `work_item.detail.state`): the **item's own provider** decides,
  whichever is active, so another provider's item never gets a lens
  that sends this one's states (tsk918). A component showing the
  capability as a whole (the Board) passes none: the **active** row
  decides, here and not at load, since the active provider changes
  with `activeProviders` and no reload. Either way there is one
  candidate;
- no candidate, a deciding provider no extension brings (oxplow's own),
  or the target turned off → `fallback`, untouched;
- chosen → its lens runs with the props it declares (`childParams`) and
  renders through `LensResultView` (the one render path: kit, or `viz:
  custom` with its sandbox and "custom" badge) under a **"replaced by
  <extension>"** badge (`replacement-<target>`, `replacement-badge`),
  re-running like any lens. The page's own chrome (the Board's scope
  picker) stays oxplow's;
- it can't load — the lens run fails, or its custom component doesn't
  start (`CustomComponentViz.onFailure` → `LensResultView.onCustomFailure`
  → `useReplacement`'s `fail`, which makes the replacement `failed`) →
  **the core component**, under a line saying whose it was and why
  ("linear's board couldn't load: …. Showing oxplow's.",
  `replacement-fallback`) — never the lens's table, and **outside** the
  replacement's frame: no badge, none of the lens's toolbar (tsk855);
- while any of that isn't known yet, nothing renders, so the wrong
  component never flashes. Reads of who is active overlap (one per
  change); the newest one asked decides, however they resolve
  (`useReplacement`'s sequence, as its lens run has, tsk857).
Showing one records `usage { kind: "replacement", key: <ext>/<target> }`
(the evidence a kind needs to be promoted) — a kit lens when it ran, a
custom one when its component says `ready` (`onCustomReady` → `shown`),
so one that never loads isn't counted. A replacement is a lens, so
its text rendering is the lens's; agents read work items from
`v_work_item` either way. The capability smoke test renders the Board
with every enhancement off (no `replacement-*`), and with two extensions
replacing it shows that only the active provider's does; a work item's
state control, with two extensions replacing it, shows the item's own
provider's, not the active one's.

The first real one is the Linear example
(`examples/extensions/linear`): `lenses/board.yaml` lists the team's
issues under Linear's own workflow states, which oxplow's Board folds
into "to do"; its kit test loads it with the replacement.

## Reviewing by effect

An extension is reviewed by what it would **change**, not only by what
it declares (P6b.E; `extension_effects.rs`). `EffectReport` holds, each
with a `Change` (`added`, `removed`, `changed`, `unchanged`):

- `lenses` — each lens's rendered text before and after (`LensEffect`);
- `models` — each view, the parts that differ (`changed`: `query`,
  `columns`, `description`, `tests`, `version`, `deprecated`), its
  columns before and after, the first contract difference
  (`contract_change`; a SQL-only change keeps the contract) and its
  `downstream` readers (`models_diff`); the review names the parts
  ("its description and tests changed") when the contract held;
- `collectors` — each collector's `Grants` (entry, runtime, hosts,
  credentials, env) before and after and the views it fills
  (`collectors_diff`);
- `providers` — grants, each declared command by name (`CommandChange`)
  and the capability's features before and after, plus where spec and
  declarations first differ (`first_difference`, so a change no grant,
  command or feature shows still reads as something) (`providers_diff`,
  over each spec and its checked-in declarations);
- `config` — the instance config schema, with the property keys added,
  removed or changed and the first change outside `properties`
  (`other_change`: `required`, …) (`config_diff`).

`extension_effects::json_difference` is the one "where do two JSON
values first differ" walk (the provider host's `first_difference` uses
it too). A provider's changes are worded once, in Rust
(`extension_effects::provider_phrases`, P8.C5): the review's `summary`
prefixes each phrase with `Provider <id>:`, and `approval_lines` — sent
as `ProviderEffect.lines` — capitalizes them for the Data section's
approval row.

The report is built from the two loaded versions: **it never runs a
program, an exec collector or a provider** — consent forbids running a
version nobody approved. What it does run is read-only or sandboxed:
lens and model queries on each version's overlay, and derived
collectors' and effects' scripts (below).

`extension_effects::effects(layer, before, after)` (P6b.E2) builds it:
each version carries its lenses **already run once**
(`extensions::run_lenses` → `LensRuns`: default params, no viewer
context) — for the candidate, the very runs `check_extension` checked,
so a review runs each lens once — and each is rendered from its run
(`lens_text::render`, a grid's children rendering empty) against **its
own version's models** (each side's overlay, P8.C2 — so a lens over a
model the candidate changes renders against the changed SQL). A lens
that fails the same way on both sides is `unchanged`. Each
model's `downstream` is its direct readers from `v_model_lineage`
(`downstream_of`) other than this extension's own views, by exact name;
a provider's declarations are read from each version's files.
`review_extension` fills `ExtensionReview.effects`, with the installed
version as `before` when it replaces one (`review_update`), else none
(everything is `added`); a candidate that doesn't load gets **no**
report (`effects: None` — its errors say why, and it can't be
installed). Settings → Extensions shows `EffectReport.lines` —
`extension_effects::summary` (P8.C5), the one wording the install
review, `plugin check --effects` and an effort's review share:
collectors' and providers' grants first — "now reaches x (was y)", a
provider command added (destructive) — then models with their readers,
lenses, the config keys) and each changed lens's text before and after,
side by side (`EffectDiff`; no line diff yet). Model rows, collector
dry runs, `plugin check --effects` and the effort-review view followed
in P8.C (below).

**An extension at any revision** (P8.C1). `extension_at(trees, ws, rev,
name)` loads project extension `name` as revision `rev` of the
workspace holds it — a commit (`git:HEAD`), a snapshot, the working
tree — through `Trees::corpus` into an in-memory `Tree` (the
`ExtensionFiles` the loader reads). A custom component's bundle is
looked up three ways (`BundleLook`): on disk it's `Found` or `Absent` (an
error); in a revision's tree it's checked from the tree's own paths when
they're there, and `Unknown` — taken as declared, not an error — when
they aren't, since a built bundle usually isn't committed (tsk784). At `git:HEAD` of a clean worktree it equals the
disk load; `None` when that revision has no `extension.yaml`. It's what
lets a review compare two revisions of an extension, neither of which
need be on disk.

**Each side on its own overlay** (P8.C2). A check is `prepare` →
`Prepared { lenses, overlay }`: the extension's models (with the other
enabled extensions') compile to temp views, published nowhere, and its
lenses, advisories and commands read through that overlay. A review
prepares **both** sides — the candidate and the installed version — so a
lens over a model whose SQL changed renders against each version's own
SQL, whatever the database has published (a disabled extension, a model
that failed, another worktree's copy).

**A model's rows** (P8.C3). Each changed, added or removed model's
`ModelEffect.rows` is a `RowDiff { before, after, keyed?, note? }`: each
side's rows read through that side's own overlay (up to 10 000 — the read gateway's own cap, `MAX_ROW_LIMIT`, tsk779;
`ROW_DIFF_LIMIT`). When both versions declare the same non-empty `key`
(P8.B1) it's a merge-join by key — `KeyedDiff { key, added, removed,
changed, samples }`, up to 20 sample rows in key order, each with its key
and its row before and after (key order is SQLite's: numbers by value,
then text); otherwise, or past the limit, or when a side's key has a
NULL part or repeats (one row would stand for several — the key test
runs only at publish, tsk792), counts with a note saying why (or that a
side's query failed). An unchanged model has
none. The rows are the review's only reads of model data; nothing is
written.

**A collector's outputs** (P8.C4). Each derived collector (Starlark,
jaq) whose script or declaration changed is dry-run on the same inputs
in both versions — each version's intent-example fixtures that name it
(`fixtures/<example>.yaml`, `input: { collector, rows }`), the latest
five events its `on:` trigger matches, else its `input:` query once — by
`collector_runner::dry_run_collector`: each version's own script text,
its `input:` read through that version's own overlay (tsk782),
storing nothing, with a `RefusingOracle` answering every `ai_*` builtin
with an error (a review never spends or sends), under the command
scripts' `COMMAND_SCRIPT_BUDGET` (5 s, not a live collector's 120 s:
someone waits on a review). `CollectorEffect.outputs`
holds each input's `Ran { counts, rows (20 per entity), error? }` before
and after. An exec or read collector is never run — approved or not —
and says so in `not_run`. A collector whose entry's text changed though
its declaration didn't is `changed` with `script_changed`, and says "its
script changed" even when there's nothing to run it on (tsk783).

**An effect's reactions** (P8.D12). `EffectReport.effects` lists each
effect by id with its trigger before and after (`EffectTrigger { on,
filter, input }`; a script-only change is `changed` too) and, when it
changed, `outputs`: each input — both versions' fixtures that name it
(`input: { effect, event, rows? }`) and the latest five events of its
`on` types — composed by each version that reacts to it
(`effects::dry_run` through that side's overlay, nothing runs):
`Composes { commands, skip?, error? }`. The lines read "Effect x: added — on t where k = v", "Effect x: on …
→ on …", "Effect x on fixture basic: runs [a] → skips (why)".

**A bounded review** (tsk791). Script dry runs — collectors' and
effects' — share one deadline, `REVIEW_DEADLINE` (60 s, from the start
of `effects`; `effects_within` takes another): past it, no further input
runs, and each collector or effect that didn't get through its inputs
is in `EffectReport.out_of_time` (`collector <id>`, `effect <id>`), with
the line "Out of time: collector a and effect b — the review stops
running scripts after 60s". Only the later side is *checked*
(`prepare`: commands and their examples, components, advisories, lens
shapes); the earlier side is only *read* (`read_side`: its models'
overlay and its lenses rendered on it) — what it was, not whether it
was right.

**`oxplow plugin check <name> --effects [--against <rev>]`** (P8.C6).
The same review on the CLI: `oxplow_sdk::check(…, against)` — the check
and its effects with one throwaway oxplow's command registry — loads the
extension at git `HEAD` (or `--against`) — `extension_tree_at` through a
`Trees` over the VCS alone — as `before` and the working tree as `after`,
and runs `extensions::effects_between` (the install review's path too):
each side on its own overlay over the project's database read
read-only (else an empty one), so it **writes nothing** — the database
file is byte-identical after. Text output adds `effects against <rev>:`
and the report's lines; `--json` adds `effects` (the `EffectReport`) and
`against` to the check's JSON.

**On an effort's review** (P8.C7). DiffViewPage shows an "Extension
Changes" section when the change's files include `oxplow/extensions/**`
(`changedExtensions`): RPC `extension_effects_between { streamId, start,
end }` → `extensions::extension_changes_between` names each extension
whose files differ between the two revisions (`Trees::diff`), loads both
versions as those revisions hold them (`extension_tree_at`) and reviews
them with `effects_between` — an `ExtensionChange { name, change,
effects?, errors }` each (a removed one has no report). The section
renders each with `EffectReportView` (the server's lines, then each
changed lens before and after), the component the install review uses
too. While the RPC runs it says "Reviewing acme…"; when it fails, "Could
not review acme: <why>" (tsk793) — never a spinner that doesn't end.

## Commands in core menus (`ui.commands`)

`ui.commands` (stable, P6b.C4; `extensions/ui_commands.rs`) puts a
command — any registered one, or one of the extension's own provider's —
in core menus, **for a ref**:

```yaml
ui:
  commands:
    - { command: fake.estimate, label: "Estimate in Fake…", about: work_item, placement: [menu, context] }
    - { command: work_item.transition, label: Move to Done, about: work_item, input: { ref: "{{ref}}", to: done } }
```

`about` (a core ref kind) is required; `placement` is `menu` (the page
nav bar's **Commands** menu for the page's ref — `RefCommandsMenu` beside
Ask, zero per-page wiring) and/or `context` (a row's right-click menu
for the row's ref — lens rows, by the first ref the row links to
(`rowRef`), and Board cards), both by default. `input` defaults to `{
ref: "{{ref}}" }`; its strings may be exactly `{{ref}}` or `{{ref.id}}`
and nothing else (`bindRefInput`). There is no launcher placement: a
launcher has no current ref, and `launcher: [{ target: { command } }]`
already covers a ref-less command. A command whose namespace is one of
the extension's providers groups under that provider's id, anything else
under the extension's name. `check_commands` (it replaced
`check_launcher_commands`) checks launcher and `ui.commands` entries
alike against the registry — or, for one of the extension's own
providers (not on the bus until its instance runs), against its
declarations (`provider_command_schema`). The desktop reads them from
the extensions list (`useUiCommands`) and runs each as the person
through `personCommands` (`components/uiCommands.ts`).

**The desktop reads the extensions from one store**
(`extensionsStore.ts`, `useExtensions(streamId)`): one `listExtensions`
per stream and one event subscription however many readers are mounted
(slots, `useSlotMounted`, `SettingsSlotSections`, `ui.commands` menus,
decorators, rail panels, extension pages, the launcher), reloaded on
`extensionsChanged`; an entry lives while something reads it, so a later
mount loads afresh. Only Settings → Extensions lists them itself — it
reloads after its own installs and updates.

## Custom components

`custom_components:` (P6b.D1; **stable since P11**;
`extensions/custom_components.rs`) are web bundles a `viz: custom` lens
renders in a sandboxed frame. **The sandbox bounds what it reaches**:
the frame has an opaque origin (the iframe's `sandbox="allow-scripts"`
*and* the daemon's `sandbox allow-scripts` CSP directive), no daemon
token and no way to send data — no fetch/XHR/WebSocket (`connect-src
'none'`), no forms, no storage. The one way out is navigating itself:
the page's `frame-src` (`http://127.0.0.1:*`, `http://localhost:*`,
`http://[::1]:*` — each loopback name the daemon serves a bundle to,
tsk1003) bounds that to this machine —
Tauri's CSP in the app, and a one-directive meta CSP in `index.html`
for a plain browser, where whatever serves `dist/` sends no header
(`lens/frameBound.test.ts` keeps them one value; only `frame-src`,
since a fuller policy would bound what the page connects to, and a
daemon over a tunnel isn't on loopback) — and the host ends the
component on its second `load`. So it reaches only the lenses it may
query (`assets`) and the commands it may invoke (`commands`).

**A component that acts is a program a person approves** (P11,
tsk960). `invoke` runs a command with the viewer's rights, so a
component that declares `commands` is listed on Settings → Data →
Programs (`ProgramKind::Component`, key `component:<ext>/<id>`,
`exec_consent::component_program`): the approval covers every file of
its bundle and the commands it may run, and either changing asks
again. Unapproved, it still renders and queries; its `invoke` is
`Denied` with the approval message, and the host shows that reason
under the frame (`custom-component-refused`, `componentRefusal`) — the
component's own code may not. A component that declares no commands
only shows and reads, and isn't a program.

**What runs is what was approved** (tsk984). Before it shows a frame the
host loads the component's bundle (`load_component`, from the worktree
of the stream the lens is shown in — never the primary's for a stream
that's gone) into `Services::component_bundles`
(`component_bundles.rs`): every file of the folder, read once, keyed by
its **version** — the component's approval hash over exactly those files
and its commands (`ProjectProgram::component_hash`, the same function
Programs lists the version with). The daemon serves the frame from that
snapshot alone (`/components/v/<version>/…`), and the frame's `invoke`
names the version it loaded: it runs only when that version is
approved and declares the command. So a bundle edited on disk after the
frame loaded changes nothing that runs, and one loaded while edited
can't act on the approval of what the disk says later. The folder is
read as served — the extension's path joined with the manifest's
`bundle` — so a spelling that differs from the disk's only in case still
covers every file. The snapshots kept are the 16 newest; an older
frame's invoke is refused with "reload its lens".

```yaml
custom_components:
  - id: burndown                       # [a-z0-9-]+
    title: Burndown
    bundle: components/burndown        # default components/<id>; holds index.html
    assets: [open-tasks, oxplow-analytics/visits]   # lens ids; a bare slug is this extension's
    commands: [work_item.transition]
```
```yaml
# lenses/burn.yaml
title: Burndown
query: SELECT day, remaining FROM v_x_burndown WHERE stream_id = :stream_id
params: [{ name: stream_id }]
viz: custom
custom: { component: burndown, props: { color: accent } }
```

**Assets are lens ids, not models**: accepting a model would mean
accepting SQL from the frame; a component that needs a model writes a
`hidden: true` lens over it. Load checks, each at its line: the id, a
duplicate, `bundle` a relative folder without `..` that holds
`index.html`, no symlink anywhere in it, at most 256 files and 5 MiB
(`stat_bundle`; a bundled extension has none), each asset a lens id
alone (no `?params`, `@rev` or `#fragment` — `query(asset)` names the
lens) — this extension's must be among its lenses — and each command a
command name. A `custom` lens needs `custom.component` naming one of the
extension's components and a `query` (its rows are what the component
shows and what an agent reads); `spec_problem` refuses `custom` (an
answer can't carry one); an agent reads it through `lens_text` as its
table, prefixed ``(custom component `<ext>/<id>`; its table rendering)``.
`check_extension` checks that the declared commands are registered and
warns when a custom lens also fills a kit role block (`chart`, `tree`,
`timeline`, `steps`, `hunks`) — the kit may already render it; that is
the honest extent of a "this reimplements the kit" lint.

**The bridged calls** (P6b.D2, `lens_actions.rs`; the frame reaches them
only through the host): `run_component_query { id, asset, params,
stream_id }` runs `asset` — a lens id, a bare slug meaning the
component's extension — when it's one of the component's `assets`
(`Invalid` naming them otherwise), through `run_lens`: the lens's own
read-only, parameterised query, never SQL from the frame.
`invoke_component_command { id, command, input, stream_id, confirmed }`
runs `command` when it's one of the component's `commands` and a person
approved the component as it is now, as
`Actor::Lens { lens_id: id, on_behalf_of: Human }`, so every policy
applies as if the person ran it; the input is literal (no placeholders);
a command that asks comes back `NEEDS_CONFIRMATION` and the **host**
asks, never the frame. Both are UI RPCs (`ui` in surface parity).

**The host** (P6b.D4, `lens/CustomComponentViz.tsx` +
`lens/componentBridge.ts`): `<iframe sandbox="allow-scripts"
referrerPolicy="no-referrer">` at `componentBundleUrl(base, version)`
(the daemon's `/components/v/<version>/`, once `load_component` answered;
a bundle that can't be loaded shows the table and why; with no daemon
base — `remoteBaseUrl()` null — the lens shows its table), under a
**custom** badge. On the frame's first
`load` the host posts `init { protocol, run, props, tokens }`
(`initMessage`, the one builder) with one end
of a `MessageChannel` (to `"*"`: the frame's origin is opaque; the port
goes to that frame alone) and listens on the other end only. Frame
messages (`parseFrameMessage`): `ready`, `{ id, method: query, asset,
params }` (params SqlCell values only), `{ id, method: invoke, command,
input }`, `{ id, method: navigate, ref }` (an oxplow page only:
`componentNavigationTarget` refuses an external-url ref with `INVALID`,
since an outside tab would carry whatever the frame put in its URL out);
replies `{ id, ok, result |
error: { code, message } }`; a re-run sends `update { run }` — only
when its params or result differ from what the frame last got
(`createBridgeHost` takes the run `init` carried); a second `ready` is
ignored. The frame is keyed by its bundle URL (`ComponentFrame`): a
stream switch mounts a new frame whose first `load` is its own, and a
failure is remembered per URL. An invoke that asks shows the host's
`CommandConfirm`; confirming re-runs it confirmed, declining answers
`CANCELLED`. No `ready` within 3 s, or a second `load` (the frame
navigated itself), tears it down and shows the table. `ready` records
`usage { kind: "custom_component", key: <ext>/<id> }` — the evidence a
kind needs to be promoted. `tokens` are the root's CSS custom properties
(`tokensFromStyle`). No gesture check on invoke: the frame's
clicks don't reliably activate the host across webviews, and the real
bounds are the declared list, the person's policy and the host's
confirmation.

**The client library** (P9.A4). A bundle doesn't speak the protocol by
hand: the daemon serves `oxplow-component.js` (and its `.d.ts`) at
`/component-lib/<file>` — embedded in the daemon
(`crates/oxplow-daemon/assets/`, `components::component_lib`), served
like a bundle (loopback `Host` only, ungated, outside CORS, `nosniff`),
and named in every bundle's CSP `script-src` beside the bundle's own
folder (`bundle_csp(source, lib)`). There is no `'self'` in that CSP
(tsk983): even in a sandboxed frame it matches the daemon's whole origin
— the response URL's, checked in Chromium and WebKit — so it would let
an approved component load another bundle's code; and the check lint
reports a page path that leaves the bundle (`..` past its folder, an
absolute path other than `/component-lib/`, a `data:` script or sheet).
It is a **classic script that defines the
global `oxplow`**, not a module: module scripts are fetched with CORS,
which an opaque origin never passes and nothing served to a frame allows
— the same reason a bundle's own scripts are classic. `oxplow.connect()`
waits for `init`, answers `ready`, and resolves with `{ run (the latest),
props, tokens, protocol, onUpdate(fn) → unsubscribe, query(asset,
params), invoke(command, input), navigate(ref), applyTheme(doc?) }`; a
failed request rejects with the host's `{ code, message }`. `init`
carries `protocol` (`BRIDGE_PROTOCOL` in `componentBridge.ts`, `PROTOCOL`
in the library — one number, bumped when a message's shape changes), and
`connect` rejects a host speaking another. One app is one library
version. `componentClient.test.ts` drives the served file against the
real `createBridgeHost` and the real `init` (`initMessage`, which
`CustomComponentViz` posts), so the two can't drift (tsk856). `oxplow plugin new
component <name>` scaffolds a private extension — the component, its
`viz: custom` lens and a bundle using the library — that checks and
tests clean (`just_works.rs`). The reference is in
`docs/guide/lenses.md`.

**`custom_components` is stable** (P11, tsk962), on the evidence rule:
a shared extension's component acts with it. The github example's **PR
Lifetimes** (`lenses/pr-lifetimes.yaml`, `components/pr-lifetimes/`)
draws each pull request as a bar from opened to merged — a range the
kit's charts don't draw; its filters re-run its own lens (`query`), a
bar opens `github_pr:<n>` (`navigate`), and Sync runs `collector.sync`
(`invoke`) — refused until a person approves the component on
Programs. The loader no longer limits components to private extensions
(a bundled one is still an error: its bundle is never served). The
browser suite's `specs/components/pr-lifetimes.spec.ts` runs it in
Chromium **and WebKit** (the engines of browser mode and the macOS
window; the suite's one WebKit project): the frame loads, the filters
and navigation work, approval gates `invoke`, and a self-navigation off
this machine sends no request in either engine — the blocked page's
load then ends the component and its table shows.

**The kit's stylesheet** (P11, tsk961). `/component-lib/oxplow-kit.css`
(`assets/oxplow-kit.css`, served beside the library as `text/css`, and
named in every bundle's CSP `style-src`) is a few `ox-` classes in
oxplow's look — text, card, badge, buttons, table, states, chart
series — that read only the theme's tokens (`kitSheet.test.ts`: every
`var(--x)` is one the app's root defines, and no literal color). A
bundle links it and calls `component.applyTheme()`, which sets each of
`init`'s `tokens` on the frame's root through the CSSOM — no `<style>`
text crosses the bridge. `init` used to carry the CSS as `kitCss`; that
went, and the protocol is 2.

**Check lints** (P11, tsk961, tsk994): what a bundle's CSP refuses
without a word, or what ends the component, is an error at check, at its
file (`bundle_problems` / `page_problems`, from `check_components`):

- an inline `<script>` that runs (no `type`, or a JavaScript one — a
  `type="application/json"` data block is fine) or an event handler on a
  standard element (a custom element's `on…` attribute is its own);
- a `type="module"` script;
- a load from outside the bundle (`src`, each `srcset` candidate, a
  `<link>`'s `href`) — a scheme other than `data:`, `//host`, an absolute
  path other than `/component-lib/` (matched before `?`/`#`), or `..`
  past the folder;
- a `<base>`, an `<iframe>`/`<frame>`, an `<object>`/`<embed>`, a
  `<form>`, a refresh `<meta>`;
- an `index.html` that never loads the client library.

Only each bundle's **`index.html`** is linted — the one page a frame
shows, since a nested frame is refused and navigating away ends the
component — and it is read from the **version under check**: the
candidate of a review (`ReviewSide::read`), the folder on disk for a
plain check. A version without the built page (a revision's tree) has
nothing to lint; loading reports a bundle with no `index.html`. A
component in an extension that comes with oxplow is an error too: the
daemon never serves a bundled extension's bundle. The scan is a
start-tag reader, comments skipped and raw-text elements (`script`,
`style`, `textarea`, `title`) read to their close, not a full HTML
parser.

## Decorators

`ui.decorators` (P6b.C5; **stable since P10**; `extensions/decorators.rs`)
put labels from one of the extension's **models** on core refs:

```yaml
ui:
  decorators:
    - { model: flags, kind: work_item, placement: ref-chip, label: label, color: color }
```

The model (`v_<extension>_<model>`) must declare a `ref` column and the
`label` (and optional `color`) columns named — checked at load against
the model's declared columns, which are its contract; column names must
be plain identifiers, since the desktop names them in its query. The
desktop (`components/decorators.ts`, `useDecorations`) runs each
decorator's queries over the refs it shows (`SELECT ref, "<label>" AS
label[, "<color>" AS color] FROM <view> WHERE ref IN (…)`), re-run when
the model changes. **Bounded** (tsk934): the refs go 200 to a query
(`REFS_PER_QUERY`), each query with a row limit that has room for all of
them, so a large table loses no badge; one extension adds at most 3
decorations to one ref (`MAX_DECORATIONS_PER_REF`), each label at most 40
characters (`MAX_LABEL`, then `…`). `ref-chip`: a chip in the header of a page whose ref the model
lists, after the page's own chips (`Page`, by the page's ref from its
navigation context). `row-badge`: a `RefBadge` (tone `label`) after a
lens cell that links to a listed ref. A color is used only when it's a
plain one (`#rgb…` or a CSS color name, `safeColor`). Decorations are
additive: a decorator whose query fails shows nothing.

**Promoted to stable (P10)** on its first-party use: bundled oxplow-review
shows each effort's latest verdict on the effort — a `ref-chip` on its
page and a `row-badge` where a lens row links to it. Its model `verdict`
(`Accepted`, `Accepted (forced)` or `Changes requested`; green, orange,
red) reads `verdicts`, which keeps every verdict event
(`materialize: { incremental: seq }`, appended as each lands; a rewrite of
the log — retention's payload expiry, a restart's refill — refills it
whole). It reads only the events' **envelopes** (tsk886): the type says
the verdict (`oxplow_review.accepted` / `.changes_requested`) and the
subject what it was about — the effort first, then its work item, then
each claim and decision an acceptance took unchecked (any makes it
forced). Retention keeps envelopes and a plugin's payloads go after 30
days at most, so a verdict a decorator shows must not live in its
payload. It shows; it never acts.
`STABLE_KINDS` lists `ui.decorators`; a shared extension may declare
them.

## Event types

`event_types:` (P8.D3; **stable since P9.D6**, so a shared or bundled
extension may use it — below; `extension_event_types.rs`,
`vocabulary_reactor.rs`) declares event types
the log accepts, under the extension's own namespace — its name with `-`
read as `_` (`acme-pr` → `acme_pr.*`):

```yaml
event_types:
  retention: { payload_days: 7, content_days: 3 }   # optional; only shorter than 30 / 14
  types:
    - type: acme_pr.merged
      v: 1
      schema: event_types/merged.v1.json   # the payload's JSON Schema, in the folder
      summary: A pull request merged.
    - type: acme_pr.merged
      v: 2
      schema: event_types/merged.v2.json
      summary: A pull request merged, with its reviewers.
      upcast: event_types/merged.star      # required past v1: transform({from_v, payload}) → the v2 payload
```

**Promoted to stable (P9.D6)** on the evidence rule every kind is held to
— a first-party, shared extension depending on it: oxplow-review's
`oxplow_review.accepted@1 { unverified, inferred, deviated }` and
`oxplow_review.changes_requested@1 { unverified, inferred, deviated,
note? }`, which `accept.star` and `request_changes.star` return in
`events:` (subjects: the effort, its work item and, for an acceptance, the
claims and decisions it took unchecked; caused by the run's
`command.executed`). They replaced `oxplow_review.verdict@1`, whose
verdict lived in an expiring payload (tsk886).
Before them, a verdict lived only in a comment's text; now the effort's
timeline carries who decided what, and another extension can react to it
("Reacting to another extension's types"). What makes the kind safe to
promise is already built: a published `type@v` is a contract
(`event_type_contract` refuses a changed schema — stronger than a
model's drift warning), a new shape is a new version with an upcast, and
a removed type's rows stay readable. `STABLE_KINDS` lists it;
`EXPERIMENTAL_KINDS` keeps `providers` and `ui.replacements`
(`ui.decorators` and `ref_kinds` were promoted in P10, "Decorators" and
"Ref kinds"; `effects` and `custom_components` in P11, below and
"Custom components"). `ManifestV2::experimental_kinds_used` reads the
table, so promoting a kind is moving it from one table to the other.

**`effects` is stable** (P11, tsk956), on the evidence rule: a bundled
extension acts with one. oxplow-review's **`verify-unchecked`** reacts to
`oxplow_review.accepted`: its `input` reads the acceptance's subject
(`:event_id`, tsk955) — the effort, its item, and each claim and decision
accepted unchecked, still unverified or inferred now — and is empty when
the effect already filed (or proposed) a follow-up for an earlier
acceptance of the same effort (its `v_effect_run` reaction to that event
is `ok` or `proposed`, tsk991; an earlier acceptance from before its
approval, or whose reaction skipped or failed, doesn't count). With
nothing unchecked it skips; otherwise it files **one** item on the
reviewed item's provider — "Verify what the review of <effort> accepted
unchecked", a checklist naming each claim and decision. Each line is
text, not markdown: whitespace becomes one space, markdown's punctuation
is escaped and a long one is cut at 300 characters, and the list stops
at 50 items, saying how many more the review has. A bundled effect
is approved like any (K1, tsk953: its embedded files are hashed alike),
so it runs only once a person approves it. A task an effect (or oxplow
itself) files has no `author` — it isn't the person's; its
`work_item.created` and the run's audit name the effect (`v_task` v2
says so). The loader no longer limits effects to private extensions.

**Retention** (P8.D5) is the namespace's window for payloads and large
content (data-model.md "event_log" retention): either omitted part is the
default, and a longer one is a load error. The reactor records it in
`plugin_event_retention`, which keeps it after the extension is gone.

**Loading** checks each type the way the vocabulary registers it (a
scratch `register_declared`): a core or foreign namespace, a schema file
that's missing, isn't JSON or doesn't compile, v0, a version past 1
without an upcast, an upcast that's missing or doesn't define
`transform`, a duplicate — each `extension.yaml:<line>`.

**The vocabulary follows the catalog.** `VocabularyService` (on
`Services`, spawned at boot) rebuilds the whole `Vocabulary` — core
types and kinds plus every enabled extension's declarations — on the
catalog's change signal and swaps it in (`VocabularyHandle`); a writer
already in a transaction keeps its snapshot. Refused at that point,
listed among the extension's errors (`Services::listed_extensions`, the
one listing that adds model and vocabulary health):
- a schema that differs from the one `event_type_contract` recorded at
  that `type@v` — a published shape is a contract, so a new shape is a
  new version with an upcast;
- two extensions whose names make one namespace (`acme-pr`, `acme_pr`) —
  neither registers.

The same pass restates `event_type_contract` (read as `v_event_type`:
type, v, extension, summary, `registered`, `latest`, schema). A removed
extension's types stay listed with `registered = 0`: their rows can't be
appended any more but still read, and the pump delivers them at their
logged version (`data-model.md` "event_log"). `registered` is 0 too for a
type whose extension is there but whose declaration was refused (a
schema changed at a recorded version, a namespace collision) — its
health says why (`v_event_type` v2's column doc says so, tsk800).

**Appending them (P8.D4).** Nothing else appends an extension's types:
a command script's result (and an effect's, P8.D10) may carry
`events: [{ type, payload, subject? }]`. `own_events` turns each into an
envelope at its type's newest version, from
`extension:<extension>/<command>`, refusing (the run is `Invalid` and
writes nothing) any type the running vocabulary doesn't list as this
extension's own — a core type, another extension's, or one in its
namespace it doesn't declare. They ride the run's `Composition.events`
and are appended after the children's events, caused by the run's
`command.executed`, in the run's transaction; on the steps path (a call
outside the transaction) they're recorded with the run only when every
step landed.

**A collector may emit them too (P9.D2).** An entity collector's output
— a derived script's result, an exec collector's stdout, parsed alike —
may carry `events: [{ type, payload, subject? }]` beside `entities`.
`collector_runner::plan_events` holds them to the same rule
(`own_events`: the extension's own declared types, at their newest
version), from `collector:<owner>/<id>`, and the payload is validated
before anything is written. They are appended **in the run's
transaction**, after its rows, its `collector_run` and its
`collector.synced`, each **caused by that `collector.synced`** — with
the run or not at all. A run whose events aren't allowed fails whole
(nothing stored, the run recorded `error`). Refused besides: a
`project` or `built-in` collector's (no extension, no namespace), a type
the collector's own `trigger.on` names (its run would trigger itself),
and more than `MAX_RUN_EVENTS` (100) a run. A fact collector doesn't
emit. A preview (`CollectorPreview.events`) and a `plugin test` example
(`expect: { events: [{ type, payload? }] }`) show what would be logged;
a review's dry run counts them per type ("2 pr; emits 2 acme_pr.merged").

**The boundary.** A collector **ingests**: it turns the outside (or
existing state) into rows, and its events say *what it saw* — "a pull
request merged". It has no rights to act, needs no approval beyond its
program's, and may run again and again over the same input. An effect
**reacts**: it composes commands with an agent's rights, is approved by a
person, and runs at most once per event. So "when a PR merges, close its
work item" is a collector that emits `acme_pr.merged` and an effect that
reacts to it — never an effect with no commands whose only job is to emit
from data, and never a collector that acts.

**The loop guard** (`event_lineage.rs`) is one for both kinds of
event-triggered code. Walking an event's causes, a *hop* is an effect's
run (`command.executed` from `effect:…`) or a collector's run for an
event (`collector.synced` with `trigger: on`). Nothing reacts to an event
its **own** run led to, or to one `MAX_CHAIN` (4) hops already led to. A
guarded effect reaction is `skipped` in `effect_run`; a guarded collector
run is `collector_run.status = skipped` with the reason in `error` (V154;
its last good counts stay, and it isn't a failure toward a disable).

**Reacting to another extension's types (P9.D1).** A collector's
`trigger: { on: [...] }` and an effect's `on:` may name core types, the
extension's own declared ones, and **another extension's**
(`acme_pr.merged` from an extension that isn't `acme-pr`). There is no
`depends:` key: the qualified type name is the dependency, as a model's
`ref('<ext>/<name>')` is. `extensions::subscribes` sorts each name:
- **known** — a core type or one the extension declares;
- **foreign** — a well-formed name in a namespace that is neither core's
  nor its own: accepted, kept on `Extension.subscriptions`
  (`ForeignSubscription { by, event_type, declared_at }`), and a
  **warning** at load ("…another extension's event type; it runs once an
  enabled extension registers that type") — which is all `oxplow plugin
  check` says, since a check sees one extension;
- **unknown** — a core namespace's type that doesn't exist, its own
  namespace's that it doesn't declare, a name that isn't a type's: an
  error, as before.

The vocabulary pass then holds each foreign subscription against the
vocabulary it just built: one **no enabled extension registers** is an
error on the *subscriber* (`extension.yaml:<line>: effect \`note\` reacts
to …`), in the same list a refused declaration shows in. It clears when
the owner is installed and enabled, and comes back if the owner goes or
its declaration is refused. Until then the reaction simply never fires:
nothing can append a type that isn't registered. There is no `type@v` in
`on:` — the pump hands every consumer an event at its type's newest
registered version (`at_latest`, the owner's upcasts), so a subscriber is
written against the owner's latest shape and a breaking change is the
owner's new version plus upcast. A subscriber still **appends** only its
own types (`own_events` is unchanged), and the effects' loop guard
already counts every extension's runs.

**Health is the extension's errors, not `plugin_health`** — a refused
declaration is a load problem, like a model's contract drift, not a
failing run that counts toward a disable.

## Ref kinds

`ref_kinds:` (P8.D6; **stable since P10**; `extension_ref_kinds.rs`,
`vocabulary_reactor.rs`) adds kinds of thing a ref can name. It was
promoted on the github example's pull requests (an example counts as a
first-party use): kind `github_pr` over model `pull_request`, page `pr`,
`[[pr:12]]`. What is promised: the manifest keys below, the portable id
pattern subset, the page's `?ref=`, and `v_ref_kind`'s columns.

```yaml
ref_kinds:
  - kind: acme_pr             # <namespace>_<name>
    label: Pull request
    id: '^\d+$'               # anchored
    resolve: prs              # one of its models, with `ref` and `title` columns
    page: pr                  # one of its pages, opened with `?ref=<ref>`
    wikilink: pr              # optional `[[pr:12]]` sugar
    searchable: found         # optional: one of its models, with `ref`, `title` and `body`
    icon: git-pull-request    # one of REF_KIND_ICONS
```

**Loading** (after models and pages) refuses, at `extension.yaml:<line>`:
a kind outside the extension's namespace, an unanchored, broken or
non-portable id regex (the desktop runs it in JS's backtracking engine,
so it's held to a subset both engines read alike and JS can't be made
to hang on — tsk797, tsk917 — parsed by `id_pattern`: printable ASCII,
at most 256 characters, anchored `^…$` (an escaped `\$` isn't the
anchor); characters, classes (not negated, no `[` inside, no `&&` `--`
`~~` set operations), `\d`, `\w` and escaped punctuation (not `\<`
`\>`, word boundaries in Rust), each with at most one quantifier (`? *
+ {n} {n,} {n,m}`, at most 256; no lazy `?`, no `{,m}`); no groups,
alternation, `.` or other escapes. Two repeats of varying length must be
fenced by a character the first can't match — `^[A-Z]+-\d+$` loads,
`^\w+_\w+$` doesn't — so a failing match gives each back at most once,
and no id length needs capping. What is kept, and what both the registry
and `v_ref_kind.id_pattern` carry, spells every set out — `\d` is
`[0-9]`, never Rust's Unicode digits — escaping exactly what either
engine reads as syntax), a `resolve` that isn't one of its models with `ref` and `title`, a
`page` that isn't one of its pages, a `wikilink` core already reads
(a core kind, `git`, `dir`, `finding`, `tsk`), an icon not in
`REF_KIND_ICONS` (lucide names the desktop maps), a duplicate.

**Registering** is the vocabulary reactor's: core kinds plus each
extension's, built by one constructor (`extension_ref_kinds::kind_spec`,
which `plugin test` uses too), so `validate_ref`, `[[acme_pr:12]]` and
`[[pr:12]]` know them while the extension is installed and not after. A
kind's namespace is a string prefix, so one may hold another's (`acme`'s
`acme_` holds `acme-pr`'s `acme_pr_`): a kind in both is **the more
specific namespace's** (tsk933) — `acme-pr` keeps `acme_pr_x`, and
`acme`'s declaration of it is an error naming whose namespace it is.
Otherwise a kind two extensions both declare is an error on each, and
neither registers it. A `wikilink:` that is one of the extension's own
kinds is refused at load and costs only the sugar (both kinds load), so no name is both a
kind and a sugar, and both resolvers read a sugar before a kind's own
name (`canonical_wikilink`, `pluginWikilinkRef`). A **`wikilink:` prefix** another extension also uses (as
its prefix, or as its kind) is an error on each and **costs only the
sugar** (P10): `[[pr:…]]` links neither, but each namespaced kind still
registers and links as `[[acme_pr:…]]`, so installing one extension never
unlinks another's refs. A kind or prefix core holds is that extension's
error. The pass
restates `ref_kind` (V146) whole, read as `v_ref_kind` (kind, extension,
label, id pattern, revisioned, wikilinks, resolve, page, icon); unlike
event types, a removed extension's kinds leave, and refs to them are
unrecognized again.

**The desktop** (P8.D7, `apps/desktop/src/refKinds.ts`) reads
`v_ref_kind`'s extension rows once (`useRefKindsLoader`, in `App`,
re-read on `ModelsChanged`) into one process-wide list that the pure
helpers consult and `useRefKinds` subscribes to:
- `pageKindIconComponent` / `pageKindLabel` fall back to a kind's icon
  (`REF_KIND_ICONS`, a lucide component per allowed name — a Rust test
  keeps the two lists equal) and label;
- `refFromTabId("acme_pr:12")` opens `page:ext.acme.pr?ref=acme_pr:12`
  (`extPageRef` carries params; `ExtensionPageView` hands them to the
  lens as `initialParams`);
- `preprocessWikilinks` turns `[[acme_pr:12]]` / `[[pr:12]]` into a link
  when the id matches the kind's pattern (`pluginWikilinkRef`, as the
  backend's `canonical_wikilink` does), `urlTransform` lets the kind's
  scheme through, and the link's text becomes its title from the
  `resolve` model (`usePluginRefTitle`) unless the author labelled it.

**Searchable kinds** (P9.D3, `kind_search.rs`). `searchable: <model>`
names one of the extension's models with `ref`, `title` and `body`
(checked at load, like `resolve`); `v_ref_kind.searchable` is its view
(V155). Its rows are in the site-wide search index (`search_fts`) under
the kind, so the launcher finds a pull request by its title or text and
the hit opens the kind's page (`searchHitTarget`, the one place a hit is
routed; the row shows the kind's icon and label).

- It is **index-time ingestion, as an asset** — not a query-time UNION:
  `search_fts` owns the text it ranks (BM25 and `snippet()` need it in
  the index), and a UNION would run every extension's SQL per keystroke,
  unranked. One `Materializer` per searchable kind (`search:<kind>`), its
  inputs the tables behind the model's view (the registry's lineage; a
  materialized model's own table): a commit touching one re-reads the
  view after the assets' quiet window and restates the kind's entries in
  its own transaction — never the writer's.
- `Assets::sync_search_kinds` keeps them in step with `ref_kind` and the
  model registry (the change loop runs it at start and when `ref_kind`,
  `model` or `model_input` change). A kind that stops being searchable —
  its extension removed, the key dropped — leaves the index with its
  entries; so does one that went while oxplow wasn't running (an
  `asset_state` row `search:<kind>` with no kind behind it).
- Bounded: 20,000 rows a kind, 16 KiB of body a row. A row whose `ref`
  isn't `<kind>:<id>` with an id matching the kind's pattern is skipped
  (a hit must be something that opens), with a warning in the log.
- Entries are project-global (`stream_id` NULL): a model is.

**Revisioned plugin kinds: designed, not built** (deferred in P10 by
decision — nothing first-party has revisions to read). Today a plugin
kind's ref takes no `@rev`: a revision is read by its reader (`git:`,
`snap:` — the VCS and snapshot stores), and no plugin kind has one. The
design, so building it is filling in, not deciding:

- **The declaration:** `revisioned: true` on the kind. The loader then
  requires its `resolve` model to declare a **`rev`** column: one row per
  revision of a thing, `rev` NULL for the current one (so an unrevisioned
  ref keeps resolving as now: `WHERE ref = :ref AND rev IS NULL`).
  `KindSpec::revisioned` follows from it.
- **The page:** its lens declares a `rev` param, and the page opens as
  `page:ext.<ext>.<page>?ref=<ref>&rev=<rev>` (`rev` omitted: current).
- **Refs:** `[[pr:12@git:abc1234]]` and `github_pr:12@git:abc1234` parse
  through the one ref grammar (refs.md); the registry's `validate`
  accepts `@rev` only for a revisioned kind, as for core ones. A rev is
  opaque to the kind — the kind's model decides what it matches.
- **Search** entries stay rev-less: a hit is the thing, at its current
  revision (`searchable:` reads the `rev IS NULL` rows).
- **What would make it real:** the github collector keeping one row per
  pull request **head commit** (`rev` = `git:<head sha>`, the latest
  also stored with `rev` NULL), and a consumer that wants a pull request
  as it stood — a review or an effort linking the PR at the commit it
  reviewed. Until something reads it, building it would be an unproven
  contract (target-architecture §15 "Left for P11").

## Effects (experimental)

`effects:` (a private extension only; `effects.rs`) are scripts that react
to a logged event by composing commands, run as
`Actor::Effect { effect: "<extension>/<id>" }` — an agent's invoker
rights, no thread, never a confirmation (commands.md "Only a person
confirms"):

```yaml
effects:
  - id: announce-done          # [a-z0-9-]+, unique in the extension
    summary: Note a finished item on its thread.
    on: [work_item.transitioned]   # core types, its own, or another extension's ("Event types")
    where: { to: done }            # optional: payload fields equal to these
    input: "SELECT title FROM v_work_item WHERE ref = :work_item"   # optional; payload fields bound, and :event_id / :event_seq
    entry: effects/announce.star   # transform({event, rows}) → {commands, events?} | {skip}
    after: [page_ref.work_item]    # optional: consumers it waits for
```

**Loading** (P8.D9) reads `on`/`where` through the collectors'
`parse_trigger` (one rule for both) against core types and the
extension's own `event_types`, and needs the script in the folder,
defining `transform`; what's wrong is `extension.yaml:<line>`.

**Consent and start position** (P8.D9). An effect is a project program
(`exec_consent::ProgramKind::Effect`, key `effect:<extension>/<id>`,
listed in Settings → Data with what approving means): its approval hash
covers the script and **every file of its extension's folder** — the
manifest decides when and with what it runs — so any edit stops it until
a person approves it again. Approving (the RPC, a person's only) also
sets `effect_state.start_after_seq` (V148) to the log's head
(`effects::approved`): an effect never reacts to an event logged before
its latest approval — not the backlog, not what happened while it waited
to be re-approved. In the same transaction it drops the effect's scheduled
automatic retries (`drop_retries_of_tx`, tsk990): what they would send
was composed by the script as it was, and the approval is of the script
as it is; those reactions stay failed, for a person's Retry, which
composes afresh. `effects::gate(approved, start_after, seq)` is the one
check: `Unapproved`, `BeforeApproval` or `Runs`. The consumer hashes an
extension's folder once per catalog load (`FolderHashes`, keyed by the
catalog's `Arc`), not per event: an edit reloads the catalog, so it's
hashed again and stops the effect (tsk798). The approvals file is read
on each check, so revoking one takes effect at once.

**A bundled extension's effect is approved the same way** (P11, tsk953).
What an approval covers is read through one resolver,
`extensions::files_at(project_dir, path)`: a bundled extension's
embedded files for `bundled:<name>`, else the folder on disk. The digest
(`exec_consent::files_hash`, over `ExtensionFiles::paths` / `bytes`) is
the same for a folder and its embedded copy, and stays byte-identical
for folders (path order component by component; pinned by
`a_folders_digest_is_pinned`). So a bundled effect has a version to
approve, there is no bundled branch in the gate, and a new oxplow that
changes the extension's files asks again. Settings → Data says the
effect comes with oxplow and offers **Read the script** — the
`program_source` RPC (UI only) returns any program's entry from where it
lives.

**Running** (P8.D10, `effect_triggers.rs`): one async pump consumer,
`effect.triggers` (registered at boot beside `collector.triggers`, with
the same `after`/`after_for` handling of each effect's `after:` — one
naming no consumer is warned about once per effect and name, not on each
ordering question the pump asks). Per
event, each enabled effect whose `on`/`where` match **reacts at most
once**, keyed by `effect_run (effect, event_id)` (V149, `v_effect_run`):
1. a row exists — a redelivery: nothing. A `started` row the live
   consumer claimed is a run that claimed a step outside oxplow and was
   cut off (`effect_triggers::cut_off`): recorded `failed`
   ("interrupted"). When its claim kept what it composed — every step a
   write to a provider keeping `idempotent_writes` (`claim_tx` writes
   `resend_json` then, P11, tsk954) — it is **sent again by itself** like
   a failure that may pass, its delay counted from when it started (so
   one found more than an hour late is a person's, and so is one whose
   start can't be read — tsk999); otherwise it is a person's, never sent
   again by itself. Recording the failure and scheduling its retry are one
   transaction (`effect_triggers::fail`, tsk999), here and for a failure
   that may pass. One a person's retry or
   backfill claimed is theirs, under way, and the pump leaves it be
   (tsk847). A person's retry or backfill — or an automatic attempt — is
   no pump delivery, so nothing redelivers one cut off between its claim
   and its record: at start (`boot.rs`,
   `effect_triggers::recover_interrupted`) every attempt still `started`
   whose origin isn't `live` is recovered the same way, with its
   `effect.result` saying what started it — Delivery lists it, and a
   person may retry it (tsk845) — and counted toward the effect's health
   as the pump counts one it finds (tsk999);
2. `effects::gate` isn't `Runs`: nothing;
3. the loop guard (`lineage`, walking the event's `cause` chain): an
   event its own run caused (source `effect:<extension>/<id>`) never
   triggers it; one that `MAX_CHAIN` (4) effect runs already led to is
   `skipped`;
4. the script runs sandboxed over `{ event: { id, type, v, seq, source,
   subject, payload }, rows }` (`input` with the payload's fields bound,
   and the event itself as `:event_id` (its id) and `:event_seq` (its
   seq), the same names and meanings a collector's `input` binds — the
   event's own win over payload fields of those names, and a dry run's
   fixture without them binds NULL (tsk1002) — to read its own
   `v_event` row, its subject or cause; P11, tsk955 — one
   `effects::input_query` for the run and `dry_run` alike):
   `{ skip: "why" }` is `skipped`; `{ commands, events? }` runs.
   The script and its `input` rows run on a read **before** the run's
   transaction: the composed calls are fixed by then, so a row can be
   stale against what the run then sees (each command's own validation
   is what holds — the composer doesn't re-read).

A run is the registered `command.sequence` — its spec and compiled input
schema, shared (`Command::with_handler`) — over what the script composed
(calls, and its own events through `own_events`), run by
`CommandBus::run_effect` as `Actor::Effect` with
`RunOrigin::Effect(key)`: its `command.executed@2` is caused by the
triggering event, and the reaction's `effect_run` row and
`effect.result@4 { effect, event, outcome, reason?, proposal?, attempt,
origin }` (caused by that run) land **in the run's transaction**; with a step outside it, a
`started` claim commits first (`claim`) and the record finishes it; a
command that asks becomes a proposal whose transaction records
`proposed`. A failure before any command ran (the script, the input,
denied, invalid) is recorded `failed` by the consumer. An effect that
fails never dead-letters the event. An approved proposal runs its calls
as the person; the script's own events go only with a run that doesn't
ask.

**Attempts and a person's retry** (P9.D4). A reaction is `(effect,
event)`; `effect_run` holds its **attempts** (`attempt` from 1, `origin`
`live | retry | backfill | auto`, V156/V160), and the reaction's state is
its latest attempt's (`v_effect_run.latest`). The live consumer makes one
attempt and never another; a failed one is sent again **by itself** only
when that is safe (below). Otherwise a failed reaction is attempted
again only by a person: `effect.retry { effect, event }` (`commands/effect.rs` —
human-only, `Confirm::Always`, `External`, registered with the consumer
at boot) runs `effect_triggers::run_reaction(…, ReactionOrigin::Retry)`,
the same steps as a live reaction from the loop guard on, as the next
attempt:
- the reaction's latest attempt must be `failed` (`Invalid` otherwise: one
  that ran, was skipped or left a proposal isn't retried; one never made
  has nothing to retry);
- the effect must be enabled and **approved as it is now** — a retry runs
  today's script, composing afresh from the event, not what the failed
  attempt composed;
- when the event was logged doesn't matter (`start_after_seq` is the live
  consumer's rule; the person named the event);
- it counts toward the effect's health like any attempt, and is recorded
  `effect.result@4 { …, attempt, origin: retry }` (v3 upcasts keeping its
  attempt and origin; v2 as `attempt: 1, origin: live`).

It asks every time because of what a failure can hide: an attempt
interrupted with a step outside oxplow under way may have landed that
step, and a retry sends it again. The confirmation's text says so.
Settings → Data → Delivery lists the reactions whose latest attempt
failed (`useFailedReactions`), each with its reason and **Retry**
(`InlineConfirm`).

**Sent again by itself** (P10, `effect_triggers::auto_retry_due`). An
attempt that failed in a way that may pass (`CommandError::Unavailable`,
tsk914: a step's provider erred, timed out, lost its reply, or asked to
wait — a composite keeps its failing step's kind) while **every** step it
composed was a write
to a provider keeping `idempotent_writes` (`safe_to_resend`, through
`work_item::provider_for`) is scheduled again — `effect_run.retry_at`, at
most two in a row (`RETRY_DELAYS`: 10 s, then 60 s after the failure
before — or later when the service asked for longer; one asking for more
than `MAX_ASKED_WAIT`, 15 minutes, is a person's). A refused input, a
refusal of credentials renewal can't fix, a method or configuration the
provider lacks — `Failed` — is never retried by itself. The failed attempt keeps what it composed (`effect_run.
resend_json`, V162) for as long as its retry is scheduled — finishing an
attempt drops it, scheduling a retry writes it, and claiming the next
attempt moves it there, so no composition outlives its retry (tsk999) — and the automatic attempt **sends exactly that**
instead of running the script again (tsk887): composing afresh could read
changed rows, change a step's input and so its key, and make a write that
landed again. It is sent only while every step still goes to a provider
keeping the promise; otherwise it isn't sent (`Reacted::NotResent`) and the
failure counts, a person's. A composed `work_item.create` that names no
provider is **pinned** to the one active when it was composed
(`pin_providers`, tsk999): its retry files where the first attempt meant
to, not wherever the active provider is by then. Each step carries its idempotency key, the
same on every attempt ([commands.md](./commands.md) `effect_step_key`), so
a write that landed lands once. A person's **Retry** composes afresh from
what the effect reads now — the person decides it should. A loop every 5 s (`spawn_auto_retry`, at boot — its first tick only
after the provider registry's first reconcile, so the providers the
config names are up; tsk915 — one row's error logged and the rest
going on, tsk932) runs what is
due as the next attempt, `origin: auto` (`effect.result@4`), on the event
at its newest version (`at_latest`, as every runner hands it over; one that
no longer upcasts drops the retry — tsk911), when the effect is still
there, enabled and approved as it is now and the retry is at most an hour
overdue (`MAX_RETRY_LATENESS`: one due longer ago — oxplow was closed — is
a person's, its failure counted; tsk915; a due time that can't be read
counts as late, tsk999) — otherwise the
retry is dropped and the failure is a person's. An attempt awaiting its
retry isn't counted against the effect's health (`Reacted::Retrying`);
the attempt that exhausts the retries counts once. A step inside oxplow, or
a provider that doesn't declare the promise, waits for a person. An
attempt cut off by oxplow stopping is sent again by itself on the same
terms: its claim keeps what it composed when every step is safe to send
again, and the store keeps it after only for a failed attempt
(`finish_tx`). Delivery
says "sent again by itself shortly" on a reaction awaiting its retry; a
person may still retry it first.

**Backfill** (P9.D5). The live consumer never reacts to what was logged
before an effect's approval. A person has it react to that past with
`effect.backfill { effect, from_seq? | since?, to_seq? }` (human-only,
`Confirm::Always`, `External`): the events in the range that match its
`on` and `where` — each read at its type's newest version, as the pump
delivers it — and that it **never reacted to** (`unreacted_tx`: no
`effect_run` row) and its own run didn't lead to (the loop guard's `own`,
`event_lineage::lineage_tx`: such an event is never its trigger, so it
isn't planned — an effect that changes what it reacts to would otherwise
meet its own changes on every backfill and never get past them, tsk846),
oldest first, **up to the effect's `start_after_seq`** — what was logged
after its approval is the live consumer's, so the two never attempt one
event at once; a range past it is capped (tsk847) — each through
`run_reaction(…, ReactionOrigin::Backfill)`: attempt 1 with `origin:
backfill`, the same dedupe, loop guard, approval and health as a live
reaction. So a second backfill finds nothing, the live consumer never
reacts to a backfilled event again, and three failures in a row disable
the effect and stop the run (`stopped`) — three of the run's own in a row
stop it too, counting ones that will be sent again by themselves (whose
health waits for the retry; tsk932), so a backfill doesn't press on into
an outage. A run makes at most
`BACKFILL_BATCH` (200) reactions; the result `{ planned, ran, skipped,
proposed, failed, remaining, stopped? }` says what is left for another
run. The effect must be enabled and approved as it is now.

`effect.backfill_plan` (a read, anyone's) answers `{ planned, from_seq,
to_seq, batch }` for the same input: what a backfill would react to. Both read
the candidates by log position a page at a time (`SCAN_PAGE`) to the end
of the range, applying `where` to every one — never to a first window of
them, which a selective `where` over a busy type would leave empty for
good (tsk848) — counting all and keeping the batch's first. The bus's
confirmation shows a command's summary and input, not a count, so the
count is the plan's: an approved effect's row in Settings → Data →
Programs has **Backfill…**, which reads the plan, says how many events
the effect never reacted to and that it may call outside oxplow for each
(`backfillAsk`), and runs only on the second click — on the range it
showed (`to_seq` from the plan, so what was logged in between isn't in
it), its button saying what one run does: "Run on the first 200 of 500"
when the plan is more than a `batch` (`backfillRunLabel`, tsk849).

**There is no consumer-level replay.** Core's consumers are re-derivable
(a projection is rebuilt, not replayed); replaying the log through every
consumer would re-fire collectors noisily and effects without consent.
Reacting to the past exists only as `effect.backfill`.

## Commands

An extension's `commands:` (a stable kind, P6b; `extension_commands.rs`)
are commands on the bus whose handler is a Starlark script that
**composes core commands** — no I/O of its own:

```yaml
commands:
  - name: finish_review                # [a-z][a-z0-9_]*; registered as <namespace>.finish_review
    summary: Mark the task done and leave a note.
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }
    entry: handlers/finish_review.star # defines transform(x), x = { input, rows }
    input: "SELECT ref, status FROM v_task WHERE ref = :ref"   # optional; one read; :fields of the input
    confirm: never                     # never (default) | always | destructive; children only add
    effect: write                      # write (default) | record; `read` is refused (a lens reads)
    invokers: { human: true, agent: true, lens: true }   # default: all
    examples:
      - { name: happy, input: { ref: "work_item:oxplow:tsk1" }, expect_commands: [work_item.transition] }
```

`transform` returns `{ commands: [{ name, input }], result?, events? }`, or
`{ refuse: "<why>" }` to decline — the run is `Invalid` with that reason
and writes nothing (`composed` → `Composed::{Run, Refused}`). An example
may give `rows:` — standing in for the `input` query's result, so it
doesn't depend on the project's data (the query is still compiled) — and
may expect a refusal with `refuses: <part of the reason>` instead of
`expect_commands` (not both). The **namespace** is the extension's name with `-` → `_`
(`command_namespace`). Who holds a namespace is the bus's to say
(`CommandBus::namespace_owner`: `oxplow` for core commands,
`extension:<name>` / `provider:<instance>` for one registered whole with
`register_namespace`) — there is no hand-kept list: checked against the
running registry (`RunningCommands`, which the bus implements), a
namespace something else holds is an error, and at run time the
reconciler's `register_namespace` refuses it all-or-nothing under one
lock (reported as the extension's problem). Two enabled extensions
mapping to one namespace are both refused at load
(`refuse_shared_namespaces`, after disabling applies). Each entry is
checked at load, its error at its line (`entry_line`: the line whose
`name:` is exactly that name — `a` never lands on `abc`; ids, lenses,
models and `ui.commands` entries find their lines the same way): the name, `effect`, `confirm`,
the schema compiles, `input` is one read, and the entry is a file in the
extension that parses and defines `transform` (`check_starlark`), and
it declares at most `MAX_EXAMPLES` (10) examples. The
script's text is kept on the `ExtensionCommand` (not serialized).
`check_extension` (Settings → Extensions, `oxplow plugin check`) runs
`check_commands`: each `input` query compiles under the models'
authorizer (`SqlGateway::check`; a raw table is an error at the
command), examples or not; then, with a running oxplow's registry, each
example is dry-run — the `input` query's rows (`input_query`: bound from
the example's fields, capped at `INPUT_ROW_CAP`), then `compose_calls`, and
what it composes against the registry — every command exists, its input
fits, the names are `expect_commands` in order. Without a registry it
warns that the examples weren't checked. **`compose_calls`** is the one
compose step, for the dry run and the handler alike: the script in the
sandbox under `COMMAND_SCRIPT_BUDGET` (5 s, not the collectors' 120 s
runaway catch, because at run time the script holds the bus's write
transaction; no files, no `ai_*`), then the `{ commands, result? }` shape
(`composed`; any other key is `Invalid`).

**Running** (`extension_command`): each is a composite
(`Handler::Compose`, atomicity `Dispatch`) `<namespace>.<name>`
(summary "… (extension `x`)", the declared invokers / confirm / effect,
`Lifecycle::Experimental`). Its composer reads the `input` rows on the
connection it's given (`semantic_layer::read_on`: the `query_sql`
authorizer, row cap and timeout, the read session restored — `query_only`
off — before any writes) and runs `compose_calls` (pure, so the bus may
compose more than once: once to route, again in the run's transaction).
The bus runs what it composes ([commands.md](./commands.md) →
"Composition"): when every call stays in oxplow's records, as children
in one transaction (`run_nested`) — each child's invokers, policy and
confirmation apply (an agent's run whose child asks becomes a proposal
with the children as its dry run), one audit row with `{ result,
children }`, the children's events caused by the run, the reversed
children as its undo; when one leaves it (`work_item.*` on another
provider's item, tsk713), as **steps** — all checked first, then run in
order, each landing as it runs, stopping at the first failure, one audit
row, no undo. A script can't do I/O: `files()` sees
nothing and `ai_*` is refused without a host. **Registration**
(`ExtensionCommands`, `Services.extension_commands`): the enabled
extensions of the **primary worktree** (one bus, like providers — a
command authored in another stream registers once merged), one
namespace at a time, all-or-nothing; reconciled at boot and on the
extension catalog's change signal (`ExtensionCatalog::changes`, P7.B6):
the workspace watcher fires it for a file under `oxplow/extensions/`
**in the primary worktree** only (another stream's edits wait for the
merge), the `config.extensions` reactor for the `extensions` key
(enabling or disabling). The extension models, the provider registry and
the metric catalog follow the same signal; a namespace something else
already holds (a provider) is refused and kept as the extension's
`problem`. A launcher `{ command }` entry may name one.

## Contributing metrics (current)

`extension.yaml` takes `measures:`, `metrics:` and `dimensions:` in the
`.oxplow/project.yaml` schema, checked with the same
`oxplow_config::validate_*` functions, and fact collectors (a `collectors:`
entry with `facts:`, parsed by `oxplow_config::collectors` like the
project's), plus:

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
- Fact collectors run `starlark` / `jaq` only, with no `env` /
  `credentials` / `network`. `exec` is refused: a program that records
  facts runs only from the project's own `collectors:`, with approval;
  an extension's programs are entity collectors, which a person approves.
  `entry` must exist in the extension.
- Precedence is built-in < global < each extension < project
  (`oxplow_config::resolve_{metrics,measures,dimensions}` take an
  `extensions` layer; scope `extension:<name>`). A collector id two owners
  declare runs once — project over extension over built-in
  (`MetricsService::fact_collectors`). An enabled extension's
  metrics are **on** unless the project mentions the key (a `use:`
  override or disable marker, or its own definition).
- `MetricsService::extension_catalog` loads enabled extensions from the
  primary worktree on each resolve; fact-collector scripts are read through
  `extensions::read_extension_file` (bundled or disk), and a collector's
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

## Health, disable and repair (P7.C1–C3)

Every contribution that runs — a provider instance, a collector, an effect
(P8.D11, kind `effect`, V150) — shares one failure policy
(`plugin_health.rs`, the `plugin_health` table read as
`v_plugin_health`):

- **Counting.** A failed run or call counts; a good one starts the count
  over. A refused input, a missing approval, or a cancel isn't a failure.
- **Disabling.** The third failure in a row disables it on this machine
  and logs `plugin.disabled@1` (the row and the event commit together). A
  provider instance stops. A disabled collector doesn't run: the
  scheduler, the `collector.triggers` consumer and the snapshot sweep skip
  it, and `collector.sync` refuses it with its reason (`Invalid` at
  `/id`). A disabled effect stops reacting (`effect.triggers` skips it).
  For an effect only a `failed` reaction counts — an interrupted one
  included, a lost race to another delivery not; `skipped` and `proposed`
  never do, and a reaction whose commands ran starts the count over. Nothing cascades: a lens over its view still runs, carrying a
  warning (`LensRun.warnings`) that its rows aren't refreshing.
- **Repairing.** The `plugin.repair` pump consumer files a work item on
  the active work-items provider, as the system: title `Repair <plugin>
  <contribution>: <reason>`, body the repair prompt (`plugin_repair::render`,
  golden `crates/oxplow-app/tests/fixtures/repair-prompt.md`: what failed,
  its intent and declaration, its recent failures, what `plugin check`
  reports, its intent examples, `engine:` against the running oxplow, and
  what to do). When the active provider can't take it — not running, or
  the very contribution just disabled — it's filed on oxplow's own tasks
  with a line saying why (tsk714), so a repair item always exists. A later
  disable while the item is open comments on it;
  once it's done or canceled the next disable files a new one
  (`plugin_health.repair_item`, `v_plugin_health.repair_item` while open).
  oxplow never sends it to an agent.
- **Enabling.** Only a person: `plugin.enable { plugin, kind,
  contribution }` (human-only). It names the kind — a provider and a
  collector may share an id; there must be a failed one of that kind (or
  a provider instance the registry knows).
- **In the app (C3).** Settings → Extensions shows one health line per
  contribution under its extension (`pluginHealth.ts`, live over
  `v_plugin_health`): `OK` with its average time, a missed schedule and
  undelivered events; `Failing (N in a row): <error>`; or `Disabled:
  <reason>` with **Enable Again** (`plugin.enable` through
  `personCommands`) and, while its repair item is open, **Repair with
  the Agent** — one line in the agent's input, `Repair the extension
  described in [oxplow ref <repair item>] — read it first.`, never sent
  (the repair brief is the item's body; `no-agent-input-automation.test.ts`
  pins the button as its only caller). Settings → Data → **Delivery**
  lists the pending dead letters (`delivery.ts`, live over
  `v_event_dead_letter`) with Retry and Discard (`InlineConfirm`); the
  rail's Alerts shows "N events couldn't be delivered" while any wait.
- **For agents.** `questions/plugins.yaml` and the extension skill's
  "Health and repair" section: read `v_plugin_health`,
  `v_collector_run` and `v_event_dead_letter`, read the repair item
  first, fix, check, test, `collector.sync` — and ask the person to
  Enable Again (an agent can't run `plugin.enable`).

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
    manifest (checked: non-empty, **one line** — a pasted line break is
    Enter in the terminal, so a multi-line prompt would send itself —
    and `about` a registered ref kind).
  - `prompt_catalog::prompt_catalog(extensions)` merges them (core first,
    then enabled extensions by name); RPC `prompt_catalog { stream_id }`
    (UI only) serves it.
  - The UI: the **Catalog** page (`page:catalog`, `pages/CatalogPage.tsx`:
    prompts by source, `v_model` by owner, `config.list_keys`), the nav
    bar's Ask menu (suggested prompts for the page's ref kind), and
    `EmptyState`'s prompts (usability.md → "Empty states").
- **Launcher entries (current, P6.D1):** the manifest's `launcher:` lists
  what isn't a lens, each `{ label, category, target }`, where `target` is
  exactly one of `{ ref }` (a canonical ref the kind registry validates —
  known kind, well-formed id — of a kind that opens as a page,
  `manifest_v2::PAGE_KINDS`, which mirrors the UI's `pageKindOf`; anything
  else is a load error rather than an entry the launcher drops silently),
  `{ command, input? }` (run as the person, asking first when the command
  asks — `personCommands.ts` + `PersonCommandConfirm`, mounted once in
  `App`) or `{ prompt }` (put in the agent's input, never sent; one line,
  like `intent.prompts`, checked by `manifest_v2::prompt_line_problem`).
  `manifest_v2::launcher_entries` types them at load (a bad one is an
  error at its line and is dropped); the dry run (`validate_extension`,
  `review_extension`, with `CommandBus::input_schema` as
  `CommandSchemas`) checks that a command is registered and its input
  fits. `oxplow plugin check` has no running app to ask, so it says the
  commands weren't checked and where to check them (Settings →
  Extensions) — with or without a project database
  (`check_commands` (launcher and `ui.commands` entries) runs on both of `oxplow_sdk::check`'s
  branches). The launcher (`components/extensionLauncher.ts`)
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
- `list_extensions`, `list_collectors`, `run_collector(owner, id)`
  (never approves).
- **Worktree streams: preview, don't run (tsk377).** Collected data is
  project-wide (one `ext__<ext>__<entity>` table), so a run (the
  `collector.sync` command over `CollectorRunner::sync`, from any caller)
  always runs the **primary** worktree's copy. An agent
  writing a collector in a worktree stream checks it with MCP
  `preview_collector(owner, id, stream_id)`: `collector_runner::
  preview_collector` runs that worktree's version through the same
  `produce` step (same consent: an exec collector needs a person's
  approval of that exact hash; derived collectors don't) and returns the coerced
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

## Example extensions (`examples/extensions/`)

Each loads clean (`documented_examples_load_without_errors`):

- **`github`** — an `exec` collector bringing pull requests in as the
  entity `v_github_pr`, with a lens joining them to streams and tasks;
  and (P10) a pull request as a **ref kind**, `github_pr` — model
  `pull_request` (`ref`, `title`, `body`; it resolves and is searchable),
  page `pr` (the `pr` lens, given `?ref=`), `[[pr:12]]`. `crates/oxplow-
  sdk/tests/examples.rs` checks and tests it and opens a pull request.
- **`linear`** (P7.A5) — the reference external **provider**: Linear
  issues as work items, through `crates/oxplow-provider-linear`
  (private, since `providers:` is experimental). `tests/kit.rs` in that
  crate runs it through `oxplow plugin test`; `scripts/install-linear.sh`
  installs it into a project. See [providers.md](./providers.md) →
  "The Linear provider".

## The `oxplow-analytics` example extension

What moves out of core, and what it becomes:

| Today (core) | Becomes |
|---|---|
| Planning / Review / Quality dashboards | `grid` lenses (**done**: `planning`, `review`, `quality`) |
| Code-quality runner, dup scan, FindingPage, DuplicateBlockPage | **done:** the `findings` / `duplicate-blocks` lenses; the dup scan runs in core's change analysis; `DuplicateBlockPage` stays core as the compare page |
| Change-analysis cards (treemap, look-here-first, functions, co-change, zones) | **done:** the `change-review` grid in the `effort.review.details` / `vcs.commit.details` / `vcs.status.details` slots; core keeps a changed-files tree (`ChangedFilesTree`, `useChangedFiles`) |
| Fact collectors (`oxplow/collectors/*.star`, idiom `.star`; gauges until P7.B3) | extensions can declare fact collectors (tsk311); the built-in catalog stays core as the opt-in standard library |
| Metric-threshold nudges | **done:** the `threshold-crossed` advisory (with `coverage-target` and `metric-deltas`) |
| Usage / page analytics / token pages, `ThreadTokenTotal`, `EffortTokenUsage` | **done:** the `usage` grid; `task-tokens` (`work_item.detail.body` slot) and `thread-tokens` (`thread.plan.header` slot) |
| Local history dashboard | stays core (snapshots are substrate); the `recent-snapshots` lens in `review` covers the at-a-glance view |
| Effort metrics block, effort coverage page, tests-run and nudge blocks | **done:** `effort.review.details` slot lenses `effort-tests` (grid: coverage, untested files, test runs, failed tests, analysis findings), `effort-metric-deltas`, `effort-nudges` |

**Owns what reads tables (P7.B5).** Derived data computed only from
core's rows is the extension's, as SQL: the "look here first" score is
its model `change_interest` (`v_oxplow_analytics_change_interest`, over
`ref('change_file')` and `ref('change_function')`), which the
`change-look-here` lens reads; co-change surprises are its models
`co_change_pair` (`materialize: { every: 1h }` over the commit index —
"the last 180 days" moves with the clock) and
`change_co_change`, which the `change-co-change` lens reads — and the
`file-co-change` lens ("Usually Changes With", mounted at
`diff.file.header`, P9.A2) reads `co_change_pair` for the file a diff
shows; an
effort's churn is its fact collector `oxplow_analytics.effort_churn`
(`on: [effort.finished]`, `after: [change.analyze]`, reading the
effort's change files) recording `oxplow_analytics.effort_churn_lines`.
Full-tree duplicated lines (tsk388) read the tree, not tables, so they
are a core built-in collector, `oxplow.duplicate_lines`, over the
public `duplicate_blocks(min_lines)` builtin — what any extension could
write. What needs two revisions' trees — files, functions, imports, test
signals, per-function churn, the scoped duplicate scan — stays in core's
change analysis.

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
`effort.review.details` slot; TaskPage keeps the `work_item.detail.body` slot.

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
- `work_item.detail.body` and `thread.plan.header` slots, with slot params checked at load.
- Lens `launcher.category` and `hidden`.
- Lens `actions:` as commands (P6.B1; the tsk329 registry retired).
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see "Advisories").
- Lens tabs carry params: `lens:<ext>/<slug>?k=v` (`lensRef(id, params)`),
  so a slot lens's heading opens its page with the slot's values, and
  history and bookmarks keep them.
