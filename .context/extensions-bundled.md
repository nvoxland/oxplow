# Extensions: Bundled and example extensions

The `oxplow-foundation` and `oxplow-bundled` extensions, the review packet, and the example extensions. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Bundled extensions and the review packet

- **Bundled extensions.** There are two. `oxplow-foundation` is required: oxplow's own
  commands, AI providers and harness implementations, the default work list,
  effort policy, snapshots and wiki, and the
  measures, dimensions and metrics oxplow records itself (tokens, efforts, tests,
  coverage, analysis, work items: its `measures:` / `dimensions:` / `metrics:`,
  scope `extension:oxplow-foundation`; the producers' Rust only records the facts).
  `oxplow-bundled` holds the
  review packet, the analytics pages and the agent's advisories in one
  extension, because the point is building oxplow on its own extension
  platform, not letting people pick between its features. It can still be
  disabled like any other. `oxplow-bundled` also holds the code metrics measured from each snapshot's
  tree (TODO markers, function count, complexity and length, doc coverage,
  duplicated lines and the per-language idiom counts) with their collectors,
  metrics and the measures only they write (`oxplow.todo`, `oxplow.ast_hit`,
  `oxplow.duplicate_lines`, `oxplow.doc_coverage`); foundation has none of
  them. Complexity, function length and parameter count stay foundation's
  measures: core's change analysis records them too. Both share the `oxplow`
  namespace, so a script or metric in one names the other's `oxplow.*`
  measures without a warning.
  - Their sources live in the repo at `extensions/<name>/`, and they
    ship as described under "Shipped deployments" below.
  - `origin` is `bundled` and their path is `bundled:<name>`.
  - Their names are reserved:
    - a project folder with that name is listed with an error and never
      shadows the bundled one;
    - `oxplow.extension.deploy` of the name resolves to the shipped deployment;
    - `extensions::writable_extension_dir` (the check behind
      `oxplow.extension.write_page`) refuses to write into a bundled
      extension.

- **The review packet** (in `oxplow-bundled`). Its pages, mounted in
  the `sections` slots of `oxplow-foundation/diff-view` (the Changes
  page), `commit` and `uncommitted-changes`:
  - Decisions Made (`v_decision`, `provenance = 'recorded'`)
  - Decisions Oxplow Noticed (`v_decision`, `provenance = 'inferred'`)
  - Unverified Claims (`v_claim` whose `v_oxplow_bundled_claim_status`
    says `verified = 0`: it cites nothing and no person verified it, and
    for a `tests_pass` claim its effort's latest test run didn't pass
    with at least one test — the bundled policy's rule; core keeps only
    what a claim cites and its `review`; a dataset reads the claims from
    the agent's answers, nothing records them), plus the dismissed ones
    (which count for nothing, shown so they can be reopened), each row
    with its review state and **Mark Verified**
    (`oxplow.effort.verify_claim`), **Dismiss**
    (`oxplow.effort.dismiss_claim`) and **Reopen**
    (`oxplow.effort.reopen_claim`)
  - Decisions Oxplow Noticed's rows carry **Confirm** / **Dismiss**
    (`oxplow.effort.confirm_decision` / `oxplow.effort.dismiss_decision`)
  - What Deviated (its own dataset `v_oxplow_bundled_deviation`: each
    effort's files against its work item's title and body — a file is in
    the area when the text names it or one of its directories at least
    two levels deep; none when the item names no area. Live, not
    materialized: 0.2 s over all 5.5k effort files of this repo's
    database)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `oxplow-foundation/commit.sections` and `oxplow-foundation/uncommitted.sections` slots)
  - Struggled Here (`v_oxplow_bundled_struggle`, the bundled dataset: a file edited 5+ times or 3+ failed shell commands per effort)
  - Review Prompt (a copyable markdown prompt for reviewing the effort
    with a second harness: the task, the agent's summary, the files it
    changed, its claims and recorded decisions, and what to report; an
    effort's item comes from the active work list (`v_work_item`), and an
    effort on no item names itself instead.
    What Deviated stays silent there: with no task text, no area is
    stated)
  - Context Read (`v_context_read`)
  - Verify a Claim With Evidence (hidden, `viz: form` over
    `oxplow.effort.verify_claim`, `claim` a param)

  **Its verdicts (P7.C5, the first bundled `commands:`)**, on an
  effort's page under **Commands** (their `ui.about: effort`, offered while the effort has a work item,
  `about_when: work_item`):
  `oxplow.review.accept { effort, force? }` comments the review on the
  effort's work item then transitions it to `done`, refusing (`{ refuse }`)
  while a claim is unverified (a dismissed one doesn't count) or an
  inferred decision unreviewed unless
  `force` (then it lists them in the comment);
  `oxplow.review.request_changes { effort, note? }` comments a checklist —
  each unverified claim, inferred decision, file outside the area, and
  the note — then transitions to `todo`. A claim, decision or path is
  spliced in through `md_text` (one inert line: an agent's claim can't
  add headings, links or items to the reviewer's comment); the note is
  the reviewer's own markdown. Both read one `input` query
  (`v_effort` + `json_group_array`s over `v_claim` and its
  `v_oxplow_bundled_claim_status`, `v_decision`,
  `v_oxplow_bundled_deviation`), compose `oxplow.work_item.comment` and
  `oxplow.work_item.transition` — as steps, every list's verbs running
  outside the transaction (not undoable; [commands.md](./commands.md) →
  "Composition") — refuse an
  effort without a work item by name, are `confirm: always`, and are a person's or
  a page's — never an agent's. `questions.yaml` + `README.md` (its skill)
  say what an agent can read of the packet; a bundled extension's
  questions are checked against a running registry by
  `the_bundled_extensions_answer_their_questions` (oxplow-sdk).

  It also has a Waiting on Me page, reachable from the launcher:
  what your agents are waiting on you for (`v_agent_status` with
  `awaiting_user`: a question or plan they put to you, a permission
  prompt, a final message ending in a question), blocked tasks, and open
  notes. Each empty state says what's *good*, e.g.
  "Every claim for this effort is backed by evidence."

  The slot pages above need `effort_id` / `change_id`, so they're
  `hidden` from the launcher (opened there they'd show NULL-param empty
  states). The launcher gets stream-level starters instead, under Work:
  **Recent Decisions** and **Unbacked Claims**, scoped by the implicit
  `stream_id`.

## The rule: data in foundation, person-facing screens here

oxplow-foundation holds the data of the work list and the wiki: their
datasets, the `work_item` and `wiki` resources, the `oxplow:tasks` and
`oxplow:wiki` implementations, the bus commands (with no `ui:`), skills,
metrics and the agent's tool pages (`work-next`, `thread-work`). It has
no page that a person uses to read or edit them.

oxplow-bundled holds that UI: the pages above and the menu commands.
With oxplow-bundled off, a work item or wiki link opens core's generic
resource page. That is intended. Foundation must never link to a bundled
page, because it cannot name oxplow-bundled. The Rust test
`foundation_has_no_task_or_wiki_ui` and
`foundation_has_no_zones_dashboards_index_timeline_or_metrics_list` guard this.

## Example extensions (`examples/extensions/`)

Each loads clean (`documented_examples_load_without_errors`):

- **`github`** — an `exec` collector bringing pull requests in as the
  entity `v_github_pr`, with pages joining them to streams and tasks
  (`prs-by-task`, `pr-lifetimes`); and (P10) a pull request as a
  **resource**, `github/pr` — dataset `pull_request` (`uri`, `title`,
  `body`; it resolves and is searchable), the page `pr` (`for: github/pr`,
  given `uri=<uri>`), `[[pr:12]]`. `crates/oxplow-
  sdk/tests/just_works.rs` checks and tests it; its `a-pull-request-resolves-and-opens-on-its-page` fixture opens a pull request.
- **`session-history`** — a read-only page of the thread's earlier prompts
  from `v_agent_turn`, mounted at `new-session.sections` after `start`.
  `the_session_history_example_adds_a_section_to_the_new_session_page`
  checks the example and proves the mounted loader reads a recorded prompt.
- **`schema-review`** — a review policy of its own: a fact collector
  counting DDL per migration file (its own measure and dimension), a
  dataset joining those facts with core's evidence (`v_effort_file`,
  `v_test_run` and its provenance, `v_effort_observation`'s diff
  coverage, `v_change_function`) into a verdict, and a page
  (`effort-verdict`) replacing `effort.verdict` with it — no core change.
  `the_schema_review_example_joins_its_facts_with_core_evidence` runs it
  over a real effort.

## The `oxplow-bundled` extension

**Two shipped extensions, on purpose** (decided 2026-10-28). oxplow ships
`oxplow-foundation` (required) and `oxplow-bundled` (optional, on by
default), both in the `oxplow` namespace so a contribution can move
between them — or the two merge — without a rename. Foundation is kept
as small as it can be while still useful: the host's pages, the default
capability implementations (work items, effort policy, snapshots,
knowledge) and the measures core's change analysis records. Everything
optional — the review policy, analytics, the code metrics and their
collectors — is bundled's, and turns off with it. Don't add a third
shipped extension; bundled's contents are the next thing to sort out.

Everything oxplow ships that measures, visualizes or reviews is this one
extension (`extensions/oxplow-bundled/`): its namespace is
`oxplow_bundled` (commands, event types, measures, collectors) and its
datasets publish as `v_oxplow_bundled_*`.

What moved out of core, and what it is now. All of it is pages (files in
`pages/`, read by `v_page`); oxplow-foundation's pages draw the app's own
screens and oxplow-bundled's add the review and analysis:

- **A page for each kind of contribution.** Every contribution kind an
  extension declares (command, skill, custom component, slot, collector,
  measure, dimension, resource, implementation) has a foundation page
  (`for: oxplow/<kind>`) that draws its `v_contribution.uri` with its full
  definition, its extension and where it is declared. The Extension page's
  tables link a row to it.
- **Data is the default view, Definition the other mode.** A kind that has
  data opens on a simple raw page, `<kind>-data` (`dataset-data`,
  `measure-data`, `dimension-data`, `command-data`, `collector-data`,
  `resource-data`), with `default: true`; the
  kind's definition page above (`dataset-definition` for a dataset) is the
  alternate, in Open With. A metric is the other way round: `metric` (its
  definition, every fact about it, and its trend at the bottom) is the
  default, and `metric-data` (its recordings alone) the other mode. A data page is a title,
  the subject's name and one line, then the rows in a table with a few
  filters in the rail; a dataset's rows come from `oxplow.sql.query` over a
  query a `sql` loader builds from the page's params, only from names in
  `v_dataset` and `v_field`, with values quoted. `resource-data` is
  offered only for a kind that resolves to a dataset (`accepts: { content:
  resolved }`), so the others have Definition alone. Each data page's See
  also has Definition and each definition page's has Data, linked as the
  same URI with `#oxplow.renderer=oxplow-foundation/<page>`. A page about
  definitions (the Extension page's tables, the definition pages' links to
  each other, the data catalog) links the same way, pinning the definition
  renderer, because the bare URI opens the data view.
- **Dashboards and review.** `planning`, `review`, `quality` and `usage`;
  the code-quality `findings`; the `change-review` page (treemap,
  look-here-first, functions, co-change, zones) mounted in the
  `oxplow-foundation/diff-view.sections`, `commit.sections` and
  `uncommitted-changes.sections` slots. The duplicate scan runs in core's
  change analysis; the compare page is foundation's `duplicate-block`.
- **Effort blocks.** `effort-tests` (coverage, untested files, test runs,
  failed tests, analysis findings), `effort-metric-deltas`, `effort-nudges`
  and `effort-plan`, mounted at `oxplow-foundation/diff-view.sections`.
  `task-tokens` mounts at `oxplow-bundled/work-item.sections`;
  `thread-tokens` at `oxplow-bundled/tasks.sections`.
- **Metric nudges.** The `threshold-crossed` advisory (with
  `metric-deltas`).
- **Git History's commit graph.** The `history-graph` component, replacing
  core's plain commit list (`vcs.history.graph`, a `ui.mounts`
  entry).
- **The work list and wiki screens.** oxplow-bundled's `tasks`, `backlog`,
  `board`, `done-work`, `work-item` (and its helper pages), `new-task`,
  `wiki-index`, `wiki-page`, `wiki-freshness`, `hand-off` and the
  handoff strips on Tasks. Their menu entries are bundled's commands:
  `work_item.new`, `knowledge.new_page`, `handoff.offer` and
  `effort.link_to_task`. Each runs the same operation as a foundation
  command and adds the label, shortcut and form a person uses.
- **Explore Data.** The `explore-data` page and its component (source in
  `src/explore-data/`, built into `dist/` by `package.json`; the manifest's
  `build:` runs it). Foundation's dataset pages do not link to it.
- **Timeline.** The `timeline` page and its component (efforts, sessions,
  threads and commits on a zoomable axis; source in `src/timeline/`, built
  with the npm dependency `vis-timeline` by bundled's `package.json`). It
  moved with the rest of the person-facing screens. Foundation has no build
  step, no `package.json` and no `node_modules`; its only custom component
  is the hand-written `integrations`.
- **Dashboards index.** The `dashboards` page (launcher Data section), a
  list the launcher's Dashboards section already gives. New Dashboard… and
  Pin to Dashboard stay in foundation. See [dashboards.md](./dashboards.md).
- **The Metrics list.** The `metrics-recorded` page (the grouped list of
  every catalogued metric with trend and latest value) and its row expander
  `metrics-slices`. Foundation keeps one metric's pages (`metric`,
  `metric-data`, `measure`), which do not link to the list, and the hidden
  `metrics-dimensions` options page (the `metric` page's Slice by reads it;
  the list names it as `oxplow-foundation/metrics-dimensions`). People find
  a metric from the launcher instead: `metric` and `measure` are
  `searchable:` ([metrics.md](./metrics.md) "UI").
- **Zones.** The hidden `zones` page holds the questions about the zone
  rules (`v_zone`, `v_zone_unmatched`); nothing links to it.
- **The app's screens.** foundation's `alerts`,
  `catalog`, `changes-effort`, `comments`, `commit`,
  `diff`, `directory`, `file-viewer`, `files`, `git-dashboard`,
  `settings` (an `area` param: overview, agents, lsp, extensions, data,
  integrations, ai), `uncommitted-changes`, and more. Metrics in [metrics.md](./metrics.md).
- **Rail panels** (the `panels:` list of oxplow-bundled): Go To
  (`go-to-panel`, collapsed `go-to-bookmarks`), Waiting on You
  (`waiting-on-me`, a badge), Comments (`comments-panel`), Uncommitted
  (`uncommitted`, collapsed `uncommitted-line`, count `uncommitted-count`)
  and Work (`work`, collapsed `work-line`, count `work-count`). Only the UI
  moved: stores, commands and views stay core. Each panel's `body` is a
  page.
- **Effort reasoning.** The AI profile `reasoning` (fallback `main`), the
  job `reasoning`, the command `oxplow.effort.infer_reasoning` and the
  effects `infer-reasoning` / `store-reasoning`, writing through core's
  `oxplow.effort.replace_inferred_decisions` and
  `oxplow.effort.replace_inferred_claims`.

**Owns what reads tables (P7.B5).** Derived data computed only from
core's rows is the extension's, as SQL: the "look here first" score is
its dataset `change_interest` (`v_oxplow_bundled_change_interest`, over
`ref('change_file')` and `ref('change_function')`), which the
`change-review` page reads; co-change surprises are its datasets
`co_change_pair` (`materialize: { every: 1h }` over the commit index —
"the last 180 days" moves with the clock) and
`change_co_change`, which `change-review` reads — and the
`file-co-change` page ("Usually Changes With", mounted at the diff
page's `file-header` slot, `oxplow-foundation/diff.file-header`) reads `co_change_pair` for the file a diff
shows; an
effort's churn is its fact collector `oxplow_bundled.effort_churn`
(`on: [effort.finished]`, `after: [change.analyze]`, reading the
effort's change files) recording `oxplow_bundled.effort_churn_lines`.
Full-tree duplicated lines read the tree, not tables, so they
are oxplow-foundation's collector, `oxplow.duplicate_lines`, over the
public `duplicate_blocks(min_lines)` builtin — what any extension could
write. What needs two revisions' trees — files, functions, imports, test
signals, per-function churn, the scoped duplicate scan — stays in core's
change analysis.

**Deliberately simple.** A basic **metrics explorer** and **simple
dashboards**. The base version lets people see what metrics exist and
pin a few to a dashboard as a starting point, a bit like Metabase. It does
not need to be fancy. Explore Data (oxplow-bundled's `explore-data`
page) browses the semantic-layer catalog (entities, measures, metrics),
shows a table or simple chart sliced by a dimension, and saves the result
as a page. A dashboard is a page with `category: Dashboards` and a grid
of nodes ([dashboards.md](./dashboards.md)). Metrics is a catalog (search,
enabled/all, range, branch, sparkline + latest value); the metric page
shows trend, recordings, stats and Enable.

**Also stays in core** (it is substrate other features need): snapshots,
collection ingest, attribution, token ingest, page visits (the rail and
launcher use them), the fact store and engine, the Go To page and
bookmarks (`v_bookmark`; the rail's Go To pane is the extension's).

New in the extension: the **effort review packet**, all live exception
pages:

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

**Done when** core boots and is usable with `oxplow-bundled` disabled,
and enabling it restores the review and analysis pages.

## Added for extraction

Features added to core because the extraction needed them,
available to every extension:

- `bar`, `line`, `treemap` and `grid` viz, with `chart` and `children`.
- `commit` and `metric` link kinds; `file` links with `line`. (A `task`
  kind came and went: a work item links as `page`, by its URI.)
- the Work Item and Tasks pages' section slots, with slot params checked at load.
- Page `category` and `hidden`.
- Page `actions:` as commands.
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see extensions-commands.md "Advisories").
- Page tabs carry params: `oxplow:oxplow/page?page=<ext>/<slug>&k=v#view`,
  so a slot page's heading opens its page with the slot's values, and
  history and bookmarks keep them.
- Panel `collapsed:` — a page shown as a collapsed panel's summary
  (for the Work panel).
- Panel `count:` — a page whose row count (or `number`) is the panel's
  header count without raising an alert.
- Column `icon:` naming any page kind draws that page's icon.
- Page param `options:` — a choice, shown as a toggle in a panel's
  header and a select on a page (for Go To's Recent / Most
  visited).
- Node `group: { by, link? }` on `list` / `table` — rows under linked
  sub-headings, also in compact panels — and actions with `group:` in a
  group's heading.
- Column `icon:` / `tone:` from fixed vocabularies, and node `emphasis:` /
  `depth:` — row highlight and indent.
- List, table and tree rows that link somewhere drag into the agent's
  context; tree labels honour their column's `link`.
- Collector pacing: an `on:` trigger's `settle` / `at_most` / `idle` /
  `force` (metrics.md "Pacing"), so an expensive collector runs
  once a burst settles or the project is idle — never losing a skipped
  snapshot's files — with `v_pending_run` saying it's updating.

Extension disk readers refuse absolute/traversing paths and symlinked folders
or path segments for source, bytes, directory listings and component bundle
metadata. Project fact inputs resolve under their checkout; symlinks outside
that checkout are refused before reading content. These host checks do not
provide OS confinement for an external program a deploy approved.
