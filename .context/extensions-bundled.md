# Extensions: Bundled and example extensions

The `oxplow-foundation` and `oxplow-bundled` extensions, the review packet, and the example extensions. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Bundled extensions and the review packet

- **Bundled extensions.** There are two (see "The `oxplow-bundled` extension" for the split). `oxplow-foundation` is required: oxplow's own
  commands, AI providers and harness implementations, the default
  snapshots and knowledge, and the
  measures, dimensions and metrics oxplow records itself (tokens, requests, tests,
  coverage, analysis, tasks: its `measures:` / `dimensions:` / `metrics:`,
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

- **The shipped extensions.** Beside the two above ship oxtask and oxwiki
  (on by default: they are in `DEFAULT_EXTENSIONS`) and the language packs
  (off by default, except `oxplow-json`, `oxplow-yaml` and `oxplow-web`,
  which are in `DEFAULT_EXTENSIONS`; `.context/language-support.md`): `oxplow-rust`,
  `-typescript`, `-python`, `-go`, `-java`, `-c`, `-csharp`, `-clojure`,
  `-starlark`, and the server-only `-lua`, `-ruby`, `-bash`, `-zig`, `-json`,
  `-yaml` and `-web` (HTML, CSS). All are in `BUNDLED` and `build.rs`'s list; one that is off is turned on
  with its `oxplow.extensions.<name>.enabled` scalar. oxtask and oxwiki have
  `requires: [oxplow-bundled]`; a language pack requires nothing. `requires:` is by extension name, so it
  crosses namespaces (`oxtask` and `oxwiki` are not in `oxplow`'s). With
  `oxplow-bundled` off they are held (listed with why), never a boot
  failure. Their sources are in this repo's `extensionSources`.
  - `oxplow-foundation` (`oxplow`, required): oxplow's own commands, providers and harnesses.
  - `oxplow-bundled` (`oxplow`): the review packet, analytics pages and advisories.
  - `oxtask` (`oxtask`): the tasks store, oxplow's own task tracker: it fills `oxplow/task_record` (store `oxtask`) from its own SQLite file and holds the commands `oxtask.task.*`.
    It holds the opinion about how its tasks are worked, so bundled names none
    of its commands:
    - **The task workflow**: the setting `tasks.workflow` (`off`,
      `person_files`, `agent_tracks` the default, `custom`) with
      `tasks.definitionOfDone` (free text), in oxtask's own `configuration:`
      (namespace `oxtask`, not `oxplow`'s). `oxtask.tasks.set_workflow
      { workflow?, definition_of_done? }` (a script: it merges into the
      project layer) writes it through oxtask's `oxtask.settings.write`
      (read it with `v_config_value`). Dataset
      `v_oxtask_task_workflow` (one row: `workflow`, `definition_of_done`);
      Settings → Integrations shows it as **Task workflow** (the part
      `task-workflow-settings`, mounted at `oxplow/settings.integrations`):
      the four workflows, the current one marked, Use This to choose one, and
      a form for what done means.
    - **Skills, offered by the workflow** (`when:` on each, and only while the
      task model has a live store): `oxtask` (the commands; always offered
      then), `track-work` (`agent_tracks`: take or file a task for new work,
      in progress before editing, done when fully done), `work-tasks`
      (`person_files`: work what is assigned or queued; file only follow-ups)
      and the slash command `/next-task` (every preset but `off`). Both
      working skills say: the harness's own plan list is for steps inside a
      task, discovered work is a backlog task linked `discovered_from`, and
      done includes the definition of done. The Rust test
      `the_task_skills_follow_the_task_workflow` checks the offer per preset.
    - **The ready queue**: dataset `v_oxtask_ready_queue` (`todo` with no open
      blocker; per thread, its assigned ready items in queue order, an epic
      with its ready children as one unit; `thread_id` NULL is the pool),
      read by the page `next-task` (the agent tool `oxtask_next_task`, with
      `thread: "thr3"`) and by the context below.
    - **The agent context `task-state`**: the thread's task in progress, the
      next three in its ready queue and the definition of done (nothing when
      there is none of them; core's `CONTEXT_CLEARED` tells an agent told
      before): once, then on change, and again after a compaction
      (extensions-commands.md "agent contexts"). Offered in every preset but
      `off`.
    - **The advisory `untracked-work`** (`agent_tracks` only): a request that
      changed files (a claimed or sole `v_turn_file`) while the thread had no
      task in progress and no task linked is reminded once per request.
    Fixtures are oxtask's: `task-state-*`, `task-workflow-settings-*`,
    `the-task-workflow-dataset-reads-the-setting-as-it-changes`,
    `setting-the-task-workflow-keeps-the-definition-of-done`,
    `the-reminder-is-only-for-the-agent-tracks-workflow`, `next-task-*`.
  - `oxwiki` (`oxwiki`): the knowledge store as an extension (on by default): `.oxplow/oxwiki/<slug>.md` pages, `oxwiki.page.write` / `delete` / `restore`, a collector filling bundled's `knowledge_record` model, the `oxwiki` skill and `/note`. `knowledge.md` "The oxwiki extension". Fixtures are its own.

- **The knowledge metrics** (in `oxplow-bundled`). The `oxplow.knowledge_pages` collector restates the page counts (`knowledge.pages`, `knowledge.stale_pages`, sliced by `oxplow.knowledge_store` and `oxplow.knowledge_kind`) when a page of the model changes (`oxplow.knowledge_record.changed`) and when a snapshot changes a file a page cites (`snapshot.ref_drifted`); the `oxplow.request_read_knowledge` collector records, per stopped request that changed files, whether an agent read a page (`knowledge.read_requests`). Metrics: `knowledge.pages`, `knowledge.stale_pct`, `knowledge.read_requests_pct`. See `metrics.md` (knowledge pages, request read knowledge).
- **The tracking metric** (in `oxplow-bundled`). The `oxplow.request_tracked` collector records, per stopped request that changed files, whether it has a task linked; the `tasks.tracked_requests_pct` metric is the share. See `metrics.md` (request tracked).
- **Code metrics, the Metrics list and Duplicated Code** (in
  `oxplow-bundled`). The 16 code metrics with their Starlark collectors
  (`collectors/{code,rust,ts,clojure,csharp}/`) and the measures only they
  write ([metrics.md](./metrics.md)); five are on by default. The
  `metrics-recorded` page (the grouped list of every catalogued metric with
  trend and latest value) and its row expander `metrics-slices`. Foundation
  keeps one metric's pages (`metric`, `metric-data`, `measure`), which do not
  link to the list, and the `metrics-dimensions` query (the `metric` page's
  Slice by reads it; the list names it as `oxplow/metrics-dimensions`).
  `duplicated-code` (Code) lists each block the stream's latest whole-tree
  duplicate scan found (the `oxplow.duplicate_lines` facts, each side's
  `detail` its copy) once, biggest first, with a compare link; not the
  change analyzer's scans, which are per change and never age out. The
  holder's Findings and Review pages, which showed those per-change rows,
  were dropped. Fixtures: the collectors' own, `the-metrics-list-*`,
  `a-metric-with-no-recordings-*`, `duplicated-code-*`.
- **Dashboards and Usage** (pages in `oxplow-bundled`). `dashboards` is the
  Dashboards index in the launcher's Data section: every dashboard page,
  most recently opened first, with its extension, who sees it and its opens
  ([dashboards.md](./dashboards.md)). New Dashboard… and Pin to Dashboard
  stay in foundation. `usage` (Activity) is where the agents' tokens and the
  person's attention went over a `range` (default 30 days): fresh tokens
  (input, output, cache writes) with cache reads and the hit ratio apart, by
  day, model, thread and request, oxplow's own model calls (`v_ai_call`) and
  the most opened pages with how long each was shown
  (`v_page_visit.duration_ms`). Cache reads are kept out of every "fresh"
  sum: on a long session they are most of the prompt, and a sum with them
  hides where tokens really went. Fixtures: `dashboards-*`, `usage-*`.
- **The change review** (`pages/change-review.yaml`, "Review", param
  `change_id`). One page per analysed change: **Asked**, what it was asked
  to do (a request's or turn's prompt, a commit's message; first a line
  when the analysis is still running or failed), then **Look Here**, one
  row per file with a `high` or `medium` signal (a signal with no file is
  a "The whole change" row), worst first, then by
  `v_oxplow_change_interest`, at most five signals a row; each row expands
  to `change-review-file` (all its signals: the whole file's first, then by
  line). **By Area** counts signals per `area`, and a `sections` slot
  (`oxplow/change-review.sections`) takes other extensions' sections.
  - The signals are the open dataset `review_signal`
    (`v_oxplow_review_signal`: `change_id, source, area, signal, severity,
    path, line, end_line, title, why, uri`). Any extension adds rows with a
    dataset that says `adds_to: oxplow/review_signal` (semantic-layer.md
    "Open datasets"). bundled's own sources: `review_signal_tests` (test
    file deleted; test deleted where the file now has fewer; fewer
    assertions; skip added; all `high`, and read only in test files, since
    the counts are text heuristics) and `review_signal_api` (public
    signature changed, public function removed; `medium`).
    More sources, one dataset each (`datasets/review_signal_*.sql`):
    - `review_signal_harness`: a changed file that matches a `test-harness`,
      `ci` or `lint-config` rule of `v_file_kind_rule` (`glob_match`); `high`,
      area `tests`, one row per file and kind.
    - `review_signal_dependencies`: a `dependency-manifest` or `lockfile`
      file changed; `high`, area `dependencies`.
    - `review_signal_sensitive`: a `sensitive` or `migration` file changed;
      `medium`, area `code`.
    - `review_signal_scope`: files a request changed outside its task's area
      (`v_oxplow_deviation`), on the request's change (`target` is the
      request id as text); `info`, area `scope`.
    - `review_signal_shape`: files with a reason in
      `v_oxplow_change_interest` (large, complex, long new functions);
      `info`, `why` is the reasons text.
    - `review_signal_history`: `v_oxplow_change_co_change` rows, as
      `partners-unchanged` and `dormant-file`; a file the change adds is
      never a signal (no history, so it always read as dormant); `info`.
    - `review_signal_duplication`: `v_change_duplicate` blocks of the
      change's files, with the copy in `why`; `info`. The scan runs after the
      rest of the analysis, so these rows come late.
    - `review_signal_verification`: for a request, from the runs oxplow read
      (never the agent's word): its latest test run failed (`high`), it ran no
      test though it changed code (`medium`), and per file the changed lines
      its latest diff coverage found no test ran (`medium`).
    - `review_signal_findings`: lint-like facts (a `rule` or `severity` at a
      line: the packs' idiom collectors, a project's fact collectors) on lines
      the change touched (`v_change_hunk`), and a request's analyzer findings
      on changed lines. Facts are the current tree's, so fact signals are read
      only for a change headed at the working tree or its stream's latest
      snapshot; an older head gets analyzer findings only.
    - `review_signal_unused`: a **check**, not a SQL source over stored
      rows: the collector `oxplow.review.unused` (`collectors/review/unused.star`,
      `on: [change.analyzed]`, `where: { phase: files }`, `sync: upsert`,
      `needs: [tree.read, sql.read]`) reads `change_diff` for the lines the
      change removed, takes the call-shaped names (`name(`) that aren't also
      called on an added line of the same file, keeps those the head tree
      defines (a `grep_tree` hit that starts a function of that name by `code_metrics`;
      a library call has none) and finds no other mention of with `grep_tree`
      (a text search of the change's head tree, `anchors.revision`, not the worktree's,
      so it says "possibly"), and writes one row per function:
      `code`, `medium`, `possibly-unused`, at the definition. Its entity
      `review_unused` (key `change_id|path|line|signal`) holds the rows; a rerun
      reads the change's earlier keys and tombstones the ones it no longer
      produces. `review_signal_unused` adds them to the list.
    - `v_oxplow_review_check` (`datasets/review_check.sql`): one row per
      analysed change and per check (a collector on `change.analyzed`):
      `change_id, collector, collector_uri, state, last_run_at, error`.
      `state` is `pending` until the check has run for the change's latest
      `change.analyzed` (files) event (its `collector_run.last_event_id` is
      older than that event's seq), `failed` when that run failed, else `ok`.
      The Review page shows a pending check as still running.
  - **Any two points.** The page reads one change, of any kind: a commit, a
    request, a turn, the working tree, or a `range` between any two points
    (`git:<sha>`, `snap:<id>`, `working`), which the Changes page makes for
    the From/To it is given (its lists offer the recent commits beside the
    snapshots). Verification and scope come from a request's runs and task,
    so a range has none.
  - **The About rail** also says where the change's checks stand
    (`v_oxplow_review_check`: "Done", "1 running", "1 failed", or "None
    ran") and which language packs are off for the files it changed (from
    `v_language`, with a link to turn each on).
  - **The pages read `review_signal_shown`** (`v_oxplow_review_signal_shown`),
    never the union itself: every row with the project's severity for its
    kind from the setting `review.signals` (`[{ signal, severity: high |
    medium | info | off }]`, the last row for a kind wins; `off` drops it),
    and the source's own in `source_severity`. So one setting ranks every
    extension's signals. The union's column types follow its contributors,
    so `shown` casts every column but `change_id` (left as is so a filter on
    it reaches each source) to a fixed type.
  - `severity`: `high` and `medium` put a file in Look Here; `info` is
    counted in By Area and listed in the file's expansion.
  - The brief (`parts/change-review-brief.yaml`) is one line under the first
    section of the Changes page (`after: header`), the commit page (`after:
    detail`) and Uncommitted Changes (`after: clean`): "**3 high** · 2
    files to look at · Open Review".
  - Page SQL binds `CAST(:change_id AS INTEGER)`: the union's columns are
    untyped, and an MCP caller may pass the id as text.
  - Three pages go deeper, each with param `change_id`, all reached from
    By Area (the area cell links to Signals narrowed to that area, a
    Details cell links to Tests or Code) and from the Go Deeper line:
    - `change-review-signals` ("Signals"): every signal in one table,
      worst first, then path and line. Filter params `area`, `severity`
      and `source` (role `filter`, default `''` = all; a link sets one with
      `#area=tests`).
    - `change-review-tests` ("Tests"): the `tests` signals, each test file's
      tests/assertions/skips before and after (`v_change_test_file`), a
      request's test runs (`v_test_run` through `v_agent_turn.request_id`)
      and its `uncovered-lines` signals. Linked from the `tests` and
      `verification` rows.
    - `change-review-code` ("Code"): the non-test functions the change
      touched with their deltas (`v_change_function`), the
      `review_signal_api` signals and `v_change_duplicate` blocks. Linked
      from the `code` row.
  - `file-co-change` ("Usually Changes With", `parts/file-co-change.yaml`):
    one line above a file's diff (slot `oxplow/diff.file-header`, params
    `path`, `left`, `right`): "Usually changes with: a.rs, b.rs (unchanged
    here)", from `v_oxplow_change_co_change` (`usual-co-changers-absent`)
    of the change whose base and head revisions are `left` and `right`.
    No row, so nothing drawn, when there is nothing to say.
  - Fixtures: `review-signals-*`, `the-review-page-*`, `the-signals-page-*`,
    `the-tests-*`, `the-diff-header-*`. A commit's message
    in a fixture needs `observe: { commit: { sha, subject } }` (the
    indexer doesn't run there).
- **The review packet**. Its pages, mounted in
  the `sections` slots of `oxplow/diff-view` (the Changes
  page), `commit` and `uncommitted-changes`. What deviated and Struggled
  here are `oxplow-bundled`'s parts (`what-deviated`, `struggled`, on the
  Changes page only; they take its `uri` and show a request's rows, none
  for a snapshot's or a turn's); Tests Weakened is still the holder's:
  - What deviated (its own dataset `v_oxplow_deviation`: each
    request's files (its turns' `v_turn_file` rows, claimed or sole)
    against its tasks' titles and bodies — a file is in
    the area when the text names it or one of its directories at least
    two levels deep; none when the item names no area. Live, not
    materialized)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `oxplow/commit.sections` and `oxplow/uncommitted.sections` slots)
  - Struggled here (`v_oxplow_struggle`, the bundled dataset: a file edited 5+ times or 3+ failed shell commands per request, read through each tool call's turn)
  - Context Read (`v_context_read`)

  `questions.yaml` + `README.md` (its skill)
  say what an agent can read of the packet; a bundled extension's
  questions are checked against a running registry by
  `the_bundled_extensions_answer_their_questions` (oxplow-sdk).


  The slot pages above need a `change_id`, so they're
  parts, not in the launcher (opened there they'd show NULL-param empty
  states).

## The rule: data in foundation, person-facing screens here

oxplow-foundation holds the data of core's built-in knowledge (until it goes): its
datasets, the bus commands (with no `ui:`), skills and metrics. oxplow's own
extensions share the namespace `oxplow`, so they share one configuration
(at one `version`) and one write command over it, foundation's
`oxplow.settings.write`. It has
no page that a person uses to read or edit them. Core and foundation have
no tasks: the task model `oxplow/task_record`, its datasets (`v_oxplow_task`
…), the `task` resource, the store-agnostic `tasks` skill (`needs:
['model:oxplow/task_record']`: how to read the model, buckets, link types,
assigning; it names no store's command) and the agent's tool page
`thread-tasks` are **oxplow-bundled's**, and the shipped store is the
`oxtask` extension, which holds the opinion about how its tasks are worked
(the next-task queue, the workflow setting and its skills; below). With only foundation running there is no
task tracker ([tasks.md](./tasks.md)).

oxplow-bundled holds that UI: the pages above and the menu commands.
With oxplow-bundled off, a task or knowledge page link opens core's generic
resource page. That is intended. Foundation must never link to a bundled
page, because it cannot name oxplow-bundled. The Rust test
`foundation_has_no_task_or_wiki_ui` and
`foundation_has_no_dashboards_index_timeline_or_metrics_list` guard this.

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
- **`schema-review`** — a fact collector counting DDL per migration
  file (its own measure and dimension), and a slot page (Schema Changes)
  joining those facts with core's `v_turn_file` to list the migration
  files a request changed — no core change.
  `the_schema_review_example_joins_its_facts_with_core_evidence` runs it
  over a real request.

- **`shelf`, `shelf-folder`, `shelf-db`, `shelf-score`** — the shared-models
  example set ([models.md](./models.md)). `shelf` owns the open model `book`
  (change event, search, a pinned markdown body), the action point
  `shelf/book.new` and a read-only page. `shelf-folder` fills it from
  `shelf/*.md` (a `start` snapshot, `files` upserts and deletes);
  `shelf-db` fills it from its own SQLite file (a `sqlite` trigger, a
  cursor, a `rating` field); both write their tool by composing
  `oxplow.extension_files.write` / `oxplow.extension_store.write`.
  `shelf-score` adds a `score` field to any store's book through
  `oxplow.model.set_fields`. The test
  `the_shelf_examples_pass_their_own_fixtures` copies the four into one
  project and runs every fixture. A fixture names a collector as
  `<namespace>/<id>` and may name a required extension's collector.

## The `oxplow-bundled` extension

**Two shipped extensions, and a holder that doesn't ship.** oxplow ships
`oxplow-foundation` (required) and `oxplow-bundled` (optional, on by default), both
in the `oxplow` namespace so a contribution can move between them without a rename.
Foundation is kept as small as it can be while still useful: the host's pages, the
default capability implementations (snapshots, knowledge)
and the measures core's change analysis records. `oxplow-bundled` is what has been
reviewed and kept.

`.oxplow/extensions/oxplow-bundled-todo` holds everything that used to be in
`oxplow-bundled` and hasn't been reviewed: this repo's, off, and not part of oxplow.
A contribution moves from it to `oxplow-bundled` one piece at a time, as it is
accepted, and the holder goes away when it is empty. Add nothing new there.

- **It's off, and can't run.** `oxplow.extensions.oxplow-bundled-todo.enabled: false` in `.oxplow/project.yaml` keeps it
  off while it sits in `.oxplow/extensions/`, so nothing loads it or tests it. It
  keeps `namespace: oxplow` so what moves needs no rename, and that namespace is
  reserved for shipped extensions. Its fixtures stop running too,
  so it may fall behind core: a piece gets its fixtures working when it moves.
  `desktop-tests/` holds frontend tests of its components, and `e2e-specs/` the
  browser specs of its pages. They are reference for that trip, not specs to restore:
  what they check becomes fixtures, and only what fits "What belongs here"
  ([working-in-this-repo.md](./working-in-this-repo.md) "The browser suite") goes back.
- **Nothing may depend on it.** No test, example or doc names its contents. Core
  machinery a test needs a collector or a component for has its own test data
  (`crates/oxplow-app/testdata/`) or uses foundation's.
- **Names don't move.** Every global name comes from the `oxplow` namespace
  ([extensions-manifest.md](./extensions-manifest.md) "Global names come from the
  namespace"), so a dataset is `v_oxplow_<name>` and an event type `oxplow.<…>`
  wherever it sits. The datasets, event types and the commands that emit them moved
  first anyway: a script emits only the event types its own extension declares, and a
  decorator reads only its own extension's datasets.
- **Other docs.** Where `.context/` describes a screen, an advisory or a metric as
  `oxplow-bundled`'s (or `oxplow/...`) and `extensions/oxplow-bundled/`
  doesn't have it, it is in the holder and doesn't run today.

What moved out of core, and what it is now. All of it is pages (files in
`pages/`, read by `v_page`); oxplow-foundation's pages draw the app's own
screens and the holder's add the review and analysis:

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
  alternate, in View as. A metric is the other way round: `metric` (its
  definition, every fact about it, and its trend at the bottom) is the
  default, and `metric-data` (its recordings alone) the other mode. A data page is a title,
  the subject's name and one line, then the rows in a table with a few
  filters in the rail; a dataset's rows come from `oxplow.sql.query` over a
  query a `sql` loader builds from the page's params, only from names in
  `v_dataset` and `v_field`, with values quoted. `resource-data` is
  offered only for a kind that resolves to a dataset (`accepts: { content:
  resolved }`), so the others have Definition alone. Each data page's See
  also has Definition and each definition page's has Data, linked as the
  same URI with `#oxplow.renderer=oxplow/<page>`. A page about
  definitions (the Extension page's tables, the definition pages' links to
  each other, the data catalog) links the same way, pinning the definition
  renderer, because the bare URI opens the data view.
- **Change review.** The `change-review` page (treemap,
  look-here-first, functions, co-change) mounted in the
  `oxplow/diff-view.sections`, `commit.sections` and
  `uncommitted-changes.sections` slots. The duplicate scan runs in core's
  change analysis; the compare page is foundation's `duplicate-block`.
- **Request blocks.** Blocks for a request's tests (coverage, untested
  files, test runs, failed tests, analysis findings), its metric deltas, its
  nudges and its plan, mounted at `oxplow/diff-view.sections`.
- **Metric nudges.** The `threshold-crossed` advisory (with
  `metric-deltas`).
- **Git History's commit graph.** The `history-graph` component, replacing
  core's plain commit list (`vcs.history.graph`, a `ui.mounts`
  entry).
- **The wiki screens** have moved to `oxplow-bundled`: see "The knowledge
  pages" below.
- **Explore Data.** The `explore-data` page and its component (source in
  `src/explore-data/`, built into `dist/` by `package.json`; the manifest's
  `build:` runs it). Foundation's dataset pages do not link to it.
- **No Timeline page.** The `timeline` page and its component (and the
  `vis-timeline` dependency) were removed. Foundation has no build step, no
  `package.json` and no `node_modules`; its only custom component is the
  hand-written `integrations`.
- **The app's screens.** foundation's `alerts`,
  `catalog`, `commit`,
  `diff`, `directory`, `file-viewer`, `markdown` (a `.md` file drawn), `media` (an image or a PDF shown), `csv` and `tsv` (a CSV or TSV file as a table), `data-file` (a JSON or YAML file as a tree of its values), `diagram` (a mermaid file drawn), `files` (the launcher's way into `directory`), `git-dashboard`,
  `settings` (an `area` param: overview, agents, lsp, extensions, data,
  integrations, ai), `uncommitted-changes`, and more. Metrics in [metrics.md](./metrics.md).
- **Rail panels.** A panel lives in the extension that has its page:
  foundation's `panels:` has Go To (`go-to-panel`, collapsed `go-to-icons`;
  its header opens the `go-to` page beside them). Both parts read
  foundation's `thread-go-to` query (recent and most visited: `limit`
  rows, 20 for the body, 6 for the icons), which reads the `thread-bookmarks`
  query, the one statement of which bookmarks a thread sees; the Go To page
  reads it too. Git (`git`, collapsed `git-line`, count `git-count`) is
  foundation's too; its header opens Git Status (`git-dashboard`).
  Waiting on you (`waiting-on-you`, count `waiting-count`, project
  scope) is foundation's: what only the person can move — a question an
  agent waits on (`v_agent_status` `awaiting_user`, linked to its thread).
  Its
  alert fires while anything waits. Tasks assigned to the person are
  bundled's Assigned to you, never repeated here. Fixtures:
  `waiting-on-you-*`. Its
  files parts, the `uncommitted-changes` page and Directory read foundation's `worktree-files` query, the one statement of
  which files a stream shows: `show` is `changed`, `changed_untracked` or
  `all` (one folder), `target` the change (`''`: the working tree). Its rows
  are paths with their status, drawn by a path tree (`tree: { path, status
  }`). Where a branch stands is foundation's `branch-status` query, the
  one statement of it: `status` (the row and which buttons apply) and
  `facts` / `marks` / `line` (↑ ahead, ↓ behind its upstream and, on a worktree, the
  default branch, as coloured marks). Its `facts` (`status` with each
  comparison's marks) are drawn, one line each with its buttons, by the
  `git-branch` part, which Git Status and the panel's
  Overview both embed (the panel with `with_branch: '0'`). The panel's
  header toggle (`show`: uncommitted with untracked files, without, or
  `overview`) is declared by all three parts, so the body and the line
  (each a `switch` on it) and the count (the uncommitted files; the
  Overview counts the untracked ones) agree. The body draws its buttons;
  the collapsed line has none. A fixture's `expect.nodes` sees only the
  host page's own nodes' actions, not an embedded page's, so the part's
  buttons are tested on the part. Each has a `panel:` fixture:
  the panel run whole, as the rail shows it. Work (`work-panel`, collapsed
  `work-line`, no count; its header opens the Work board) is the
  thread's tasks, a project-manager view (never turns or requests): In
  progress is the tasks in progress assigned to it (under their epic),
  each with what the thread's latest request is doing now (the Now
  column: the panel's loader reads `v_request.state` for its own thread;
  `v_oxplow_thread_tasks` reads no request, because a request changes on
  every tool call and the list, rebuilt for every thread, would re-run
  with it, and the collapsed line shows no Now); Up next the ready and blocked tasks
  assigned to the thread in its queue's order; Finished the done tasks a
  request on the thread worked on (each once) and knowledge pages touched.
  **Assigned to you** (`assigned`; parts `assigned-panel`, `assigned-line`,
  `assigned-count`; page `assigned-to-you`, whose loaders `lines`, `line`
  and `open_count` the parts read) is the same view for the person: the
  open tasks with `assignee = oxplow:oxplow/person` (`v_oxplow_task`) - In
  progress, then Up next (todo and blocked) in the person's queue order
  (`rank IS NULL, rank, created_at`) - each with its stage, under its
  epic. A project-scoped panel (`scope: project`): the same in every
  thread, with no thread param. The header count is the open ones. It has **no Finished
  group**: a done task is unassigned and no record says who held it
  (`request_task` links threads' requests, a person's command is in no
  turn), so showing one would be a guess. Fixtures:
  `assigned-to-you-*`.
  Bundled holds `oxplow.tasks.assign { uri, to? }` and
  `oxplow.tasks.reorder { uri, before?|after? }` (scripts over the shared
  task model's `assignee` and `rank` fields, undone with their run).
  `assign` puts the task last in the new
  assignee's queue; `reorder` ranks it between its neighbours, numbering
  the whole queue first when a neighbour has no rank. Bundled names no
  store's command: how an agent uses tasks is oxtask's opinion (below).
  The Work panel is `oxplow-bundled`'s, beside its
  `thread_tasks` dataset and `tasks.clear_finished` command, with a `panel:`
  fixture. A panel moves the same way: its parts, a
  `panel:` fixture, and any rule it repeats with a foundation page made one
  query. Only the UI moved: stores, commands and views stay core. Each
  panel's `body` is a part.
- **The task page** (`task`, the renderer of `oxplow/task`;
  tasks.md "One page for every item"): About, Stage and Details in its
  rail, the description, the items under it, Activity (the `item-requests`
  page: the requests that worked on it), Comments and Links (the `task-comments` and
  `task-links` parts) and its sections slot, where bundled mounts
  `task-tokens` and `item-shared-work`. With it, `task-actions` (an
  agent's Mark done / Record a block… on one item).
- **The task screens.** Two pages over one query, `task-rows` (the items
  a page shows by `show` — assigned to this thread, its stream, you,
  nobody or anyone; a closed item, nobody's, by the requests that worked
  it — `state` (a stage of the list, or a bucket for its first stage,
  `open` or `all`), `priority` and `q`, each row's bucket, stage and stage
  title, the gates a row's actions read and the order to draw them in),
  the active list's stages (`task-stages`: `value`, `label`,
  `bucket`, `ord`, `is_default`, and `filter`, a bucket's first stage
  named by its bucket, so a static `todo` default fits any list), and New
  Item (`new-task`, with its `task-fields` query and the `task.new` command: New Task…,
  Cmd/Ctrl+Shift+N). **Work** (`work`) is the board, a custom page: the
  `board` component (`src/board/`, built into `dist/` by bundled's
  `package.json`; the manifest's `build:` runs it) draws its own toolbar,
  columns and card menu with the kit's classes and runs the task
  commands it declares; core draws its nav bar (New Task…), confirms a
  delete and records what fails. Its filters are `role: view` params, so the
  page has no rail and the board has the whole width. A card is its title and
  its description's first sentence, two lines each at most (an urgent or
  high priority a coloured edge, the rest on hover), a column per stage of
  the list but its backlog and canceled ones (the page's `stages` loader,
  bundled's `task-stages` query); a to-do stage is grouped under
  foldable epic headings, each epic the kit's nesting panel (`ox-nest`, as Task List's epics) (folds in page memory); Done is the last three
  days, twenty at most (the `board` loader), then "All N in Task List";
  each column scrolls on its own. It is the
  `task.board` replaceable, its filters shared with a stand-in.
  The board has no backlog column: the backlog is the Task List's. A
  card's menu assigns it (the stream's open threads, You, Nobody; the
  page's `assignees` loader).
  **Task List** (`task-list`, each row its title over its description's
  first sentence, an urgent or high item's coloured `edge`, who it's
  assigned to when anyone) is the list's items in one stage, whoever
  they're for — it lists tasks; assignment is the board's and the Work
  panel's (`State`, its options the stages, `todo` — the first to-do stage
  — unless asked; Done's link asks for done), its filters in the rail
  (State, Priority, Search), with a quick add at the rail's bottom (a
  title, in the stage it shows — the first to-do stage for a started or
  closed one — for nobody), with the drag and
  bulk actions (Mark ▸ the stages and Assign to ▸ the stream's threads and
  You, each one action with `options_from`; Unassign), and the agents' answers about what is in
  progress, in the backlog, ready or finished. The pure half of the
  board (`src/board/model.ts`; a card's description is the query's
  `summary`, `first_sentence(body)`) is tested from the desktop suite
  (`viz/boardModel.test.ts`); the frame by a `ui:` fixture.

**The knowledge pages** (written for URIs and stores, not slugs). They are
bundled's, so with `oxplow-bundled` off a knowledge page link opens core's
generic resource page; bundled also declares the `oxplow/knowledge` resource
(`resolve: knowledge_page`, `searchable: search_knowledge`) and the dataset
`knowledge_page` (`v_oxplow_knowledge_page`, the pages of every store of the
shared model with their `body`). `.context/knowledge.md` "The oxwiki
extension" has the read contract.

- `knowledge` (the Knowledge index, launcher section Knowledge): Pages
  (newest change first, or oldest with the Order filter; Search over title
  and body; a Store filter, every store of `oxplow/knowledge_record` from
  `v_model_store`; Delete in the row's right-click menu, confirmed, through
  the `oxplow/knowledge.delete` point, offered only for a store that joins
  it), Stale pages, and the pages that cite a file (its `path` param). New
  page opens `new-knowledge-page`. It shows no reads: those are on the
  datasets.
- **Claude Code's memories** (the store `claude-memory`): bundled fills
  `oxplow/knowledge_record` from the `memory/` folder of Claude Code's own
  data folder for the project (`files:` root `agent:claude`, read only), with
  the `oxplow.claude_memories` collector. `knowledge.md` "Claude Code's
  memories".
- Bundled's own models `knowledge_read_record` (written by the `oxplow.knowledge_read` collector from `record.read`) and `knowledge_touch_record` (by `oxplow.knowledge_touch`, from the change event's thread) back `v_oxplow_knowledge_read` / `_touch`; `v_oxplow_knowledge_ref` is `v_ref_pin` of the pages. Its `knowledge` skill reads and finds pages (gated on the knowledge model).
- `knowledge-page` (`for: oxplow/knowledge`, default): the tab is the
  page's title. The body is the markdown (read-only: a store's own page
  would replace it to edit). The rail has About (store, kind, author, last
  edited; for a page that cites files, its freshness and how many of the
  cited files changed since it pinned them, and Mark all verified) and Open in its store (when the page has a
  `url`). Referenced files has Mark verified per changed file (`oxplow.refs.verify`). See
  also: the index, the page's freshness, and what links to it.
- `knowledge-freshness` (params `store`, `id`): the page's files, stale
  ones first, Mark verified per file and for all (`oxplow.refs.verify`).
- `new-knowledge-page`: the `oxplow/knowledge.new` point as a button (several
  stores, a menu); each store's own command and form answer it (oxwiki:
  `oxwiki.page.write`, form `oxwiki/new-page`).
- Fixtures: `knowledge-*`, `the-knowledge-page-*`, `deleting-a-knowledge-page-*`
  run on oxwiki (`requires: [oxwiki]`).

**Owns what reads tables (P7.B5).** Derived data computed only from
core's rows is the extension's, as SQL: the "look here first" score is
its dataset `change_interest` (`v_oxplow_change_interest`, over
`ref('change_file')` and `ref('change_function')`), which the
`change-review` page reads; co-change surprises are its datasets
`co_change_pair` (`materialize: { every: 1h }` over the commit index —
"the last 180 days" moves with the clock) and
`change_co_change`, which `change-review` reads — and the
`file-co-change` page ("Usually Changes With", mounted at the diff
page's `file-header` slot, `oxplow/diff.file-header`) reads `co_change_pair` for the file a diff
shows. Full-tree duplicated lines read the tree, not tables, so they
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
bookmarks (`v_bookmark`; the rail's Go To panel is foundation's, beside its page).

New in the extension: the **request review packet**, all live exception
pages:

- Waiting on me
- What deviated (files outside the task's stated area)
- Tests weakened (deleted/skipped tests, removed assertions)
- Missing co-change (files that historically change with these but didn't)
- Read this first (risk-ranked hunks with evidence badges)
- Struggled here

**Done when** core boots and is usable with `oxplow-bundled` disabled,
and enabling it restores the review and analysis pages.

## Task readers moved out of foundation (flip-order step 6, in part)

Foundation must load and work with bundled off, and only
bundled reads tasks. Both extensions have the namespace `oxplow`, so a
moved page, metric or dimension keeps its id.

- **Moved to bundled:** the metrics `task.open` and `task.completed` and the
  dimension `work.priority` (all read `v_oxplow_task`). A metric's key and its
  facts' measure keys carry no extension (`metric_spec.key` is unique and
  `extension` only records the owner), so history keeps its keys. The
  metric page's "See also" now links them to `oxplow-bundled`; the fixture
  moved with them.
- **Nothing task-shaped stays in foundation:** the `tasks` skill, the
  `task` resource and its descriptors moved to bundled, and core's
  `oxplow.task.*` commands are gone (the shipped store's are oxtask's
  `oxtask.task.*`).
- **Pages:** none of `closed-threads`, `diff-view`, `local-history`,
  `request-summary` and `thread-sessions` is a task page; each showed tasks
  in part. Foundation's versions now read no task, and bundled adds the
  task facts back through the existing extension points:
  - `request-summary`: its Tasks section is bundled's part `request-tasks`,
    mounted at `oxplow/request-summary.sections` after About (same look).
  - `diff-view` (Changes): the hidden `task` column, the Task row of the
    header and the Description section are gone; bundled's parts
    `change-task` and `change-task-description` are mounted at
    `oxplow/diff-view.sections` after the header (a request's, not a turn's
    or a range's, as before).
  - `closed-threads`: the Tasks column is gone, the thread title links to
    the thread's URI, and bundled's decorator on `oxplow/thread` rows
    (dataset `thread_work_badge`) puts "3 done · 1 open" after it.
  - `thread-sessions`: the Tasks column is gone, the session name links to
    the session's URI, a subagent's tool-call count has its own Activity
    column, and a decorator on `oxplow/agent_session` rows (dataset
    `session_task_badge`) puts one badge per task (its title) after it.
  - `local-history`: a snapshot's subject names the turn that took it
    ("completed: turn 2", "in flight: turn 3") and bundled's decorator on
    `oxplow/snapshot` rows (dataset `snapshot_work_badge`) adds "completed: X"
    / "in flight: X" badges per task. A small look change.
  The decorators' limits hold: a label is at most 40 characters and an
  extension puts at most 3 on one URI.

## Task reactions to thread and stream events

Two bundled effects keep the task model right when a thread closes or a
stream archives (`effects/release_on_close.star`). They read
`v_oxplow_task` through `sql.read` and write oxplow's two fields.

## Added for extraction

Features added to core because the extraction needed them,
available to every extension:

- `bar`, `line`, `treemap` and `grid` viz, with `chart` and `children`.
- `commit` and `metric` link kinds; `file` links with `line`. (A `task`
  kind came and went: a task links as `page`, by its URI.)
- the Task page's section slot, with slot params checked at load.
- Page `category`.
- Page `actions:` as commands.
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see extensions-commands.md "Advisories").
- Page tabs carry params: `oxplow:oxplow/page?page=<namespace>/<slug>&k=v#view`,
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
