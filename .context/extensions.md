# Extensions

This doc covers how anything that **measures or visualizes** is added to
oxplow: the `extension.yaml` format, lenses, slots, actions and alerts, and
the bundled `oxplow-analytics` example extension.

> **Status: partly built (epic tsk275).**
> - **Current:**
>   - loading project extensions and their lenses from
>     `oxplow/extensions/` (see "What works today" below);
>   - running and validating lenses over IPC and MCP;
>   - the `oxplow-extension` agent skill;
>   - the lens page and the launcher's "Lenses" section;
>   - sharing: team via the repo, world via `install_extension` /
>     `update_extension` and Settings → Extensions;
>   - the core explorer: the Explore Data page (with Save as Lens) and
>     lens tiles on dashboards.
> - **Target:** everything else here, including sources, dimensions,
>   metrics, slots, actions, alerts and the `oxplow-analytics` extraction
>   (tsk278 / tsk280).
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
  - A run failure shows the error inline, with a nudge to use "Improve
    with Agent".
  - The page re-runs, debounced, on data events and on edits under
    `oxplow/extensions/`; `shouldRerunLens` in `src/lens/lensModel.ts`
    decides which events count.
  - "Improve with Agent" inserts `[oxplow lens <id> k=v…]`, with only the
    changed params, through the standard add-to-context path.
  - The Cmd+P launcher re-reads lenses from the stream's worktree every
    time it opens and lists them under **Lenses**, so they're both
    browsable and searchable.
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
- **One skill list.** Every agent runtime writes its skills from the single
  `OXPLOW_SKILLS` list in `crates/oxplow-plugin/src/lib.rs`, so adding a
  skill takes one row.

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
  runs in the app window. Reason: the daemon's `/ipc` is unauthenticated
  and includes `forward_terminal_input`, so agent-written JS in the window
  could drive the agent. Revisit only after a scoped, read-only IPC
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
- `alert`: a row-count or threshold condition that shows a rail badge and
  can be marked `nudge: true` (see below).

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
| `rail` | rail HUD (badges) |
| `launcher` | Cmd+P launcher entries |
| `settings` | Settings |

## Nudges

Today nudges are wired to gauge thresholds inside core
(`collection.rs` → PostToolUse `additionalContext`). In the target design
core keeps only a generic **nudge primitive**: an extension `alert` marked
`nudge: true` becomes PostToolUse context, and an alert marked
`inject: prompt` is added on UserPromptSubmit. The threshold logic lives in
the extension.

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
- **Deferred:** a declarative `actions:` registry and
  `run_lens_action`, until a concrete need appears. Agents already act on
  what a lens shows through the ordinary MCP tools.

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

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-plugin/assets`)
teaches the format, the `v_*` contract and the loop "describe_schema →
query_sql → write files → validate_extension → run_lens". "Improve with agent" on a lens pastes
`[oxplow lens <slug>]` plus its params into the agent's context.

## The `oxplow-analytics` example extension

What moves out of core, and what it becomes:

| Today (core) | Becomes |
|---|---|
| Planning / Review / Quality dashboards | `grid` lenses |
| Code-quality runner, dup scan, FindingPage, DuplicateBlockPage | exec source + `findings` / `duplicates` lenses |
| Change-analysis cards (treemap, look-here-first, functions, co-change, zones) | `effort-review` / `commit` / `uncommitted` slot lenses |
| Gauges (`oxplow/gauges/*.star`, idiom `.star`) | extension sources (already Starlark) |
| Gauge-threshold nudges | extension alerts → core nudge primitive |
| Usage / page analytics / token pages, `ThreadTokenTotal`, `EffortTokenUsage` | lenses + `task-detail` / `rail` slot lenses |
| Local history dashboard | lens over `v_snapshot` |
| Effort metrics block, effort coverage page | `effort-review` slot lenses |

**Stays in core, deliberately simple:** a basic **metrics explorer** and
**simple dashboards** (Nathan, 2026-09-27). The base version must let
people see what metrics exist and pin a few to a dashboard as a starting
point, a bit like Metabase. It doesn't need to be fancy. The explorer
browses the semantic-layer catalog (entities, measures, metrics), shows a
table or simple chart sliced by a dimension, and can save the result as a
lens or pin it to a dashboard. Dashboards are a plain grid of pinned
metrics and lenses. Tracked in the "basic metrics explorer" task.

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

Capabilities added to core because the extraction needed them. Empty
until tsk280 starts.
