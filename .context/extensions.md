# Extensions

This doc covers how anything that **measures or visualizes** is added to
oxplow: the `extension.yaml` format, lenses, slots, actions and alerts, and
the bundled `oxplow-analytics` example extension.

> **Status: target design (epic tsk275).** Nothing here is implemented
> yet. The extension host is tsk278; extracting today's analytics UI
> into `oxplow-analytics` is tsk280. When a piece ships, move it from
> "target" to "current" here, in the same commit.

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
- `.oxplow/extensions/<name>/`: project extensions, authored by the user
  or their agent, committed with the repo.

Each is enabled or disabled per project in `.oxplow/project.yaml`. Loading
respects the workspace isolation rule
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
- `get_open_lens(thread_id)`: which lens (slug + params) the human
  currently has open in that thread, if any. This is what lets "look at
  what I'm looking at" work.
- `run_lens_action(slug, action, row_key)`: trigger one of the lens's
  registered actions. The same fixed registry as the UI, so an agent can
  never do more through a lens than a human could.

**Building**

- Agents author extensions and lenses **by editing files** under
  `.oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  filing guard. The loader hot-reloads.
- `scaffold_extension(name)` and `scaffold_lens(ext, slug, query?)` write a
  valid starting point.
- `validate_extension(name)` returns load errors, schema errors and a dry
  run of every lens query, so the agent can check its work without the UI.
- `list_extensions`, `run_source(id)`.

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-plugin/assets`)
teaches the format, the `v_*` contract and the loop "scaffold → edit →
validate → run_lens". "Improve with agent" on a lens pastes
`[oxplow lens <slug>]` plus its params into the agent's context.

## The `oxplow-analytics` example extension

What moves out of core, and what it becomes:

| Today (core) | Becomes |
|---|---|
| Metrics / MetricDetail / Recording pages | `metrics-explorer`, `metric-detail` lenses |
| Custom dashboards; Planning / Review / Quality dashboards | `grid` lenses; `dashboard` tables retired after migration |
| Code-quality runner, dup scan, FindingPage, DuplicateBlockPage | exec source + `findings` / `duplicates` lenses |
| Change-analysis cards (treemap, look-here-first, functions, co-change, zones) | `effort-review` / `commit` / `uncommitted` slot lenses |
| Gauges (`oxplow/gauges/*.star`, idiom `.star`) | extension sources (already Starlark) |
| Gauge-threshold nudges | extension alerts → core nudge primitive |
| Usage / page analytics / token pages, `ThreadTokenTotal`, `EffortTokenUsage` | lenses + `task-detail` / `rail` slot lenses |
| Local history dashboard | lens over `v_snapshot` |
| Effort metrics block, effort coverage page | `effort-review` slot lenses |

**Stays in core** (it is substrate other features need): snapshots,
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
