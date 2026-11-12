# Extensions: Bundled and example extensions

The `oxplow-foundation` and `oxplow-bundled` extensions, the review packet, and the example extensions. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Bundled extensions and the review packet

- **Bundled extensions.** There are two (see "The `oxplow-bundled` extension" for the split). `oxplow-foundation` is required: oxplow's own
  commands, AI providers and harness implementations, the default task tracker,
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

- **The knowledge metrics** (in `oxplow-bundled`). The `oxplow.knowledge_pages` collector restates the page counts (`knowledge.pages`, `knowledge.stale_pages`, sliced by `oxplow.knowledge_store` and `oxplow.knowledge_bucket`) when a page is written, recorded or deleted and when a snapshot lands; the `oxplow.request_read_knowledge` collector records, per stopped request that changed files, whether an agent read a page (`knowledge.read_requests`). Metrics: `knowledge.pages`, `knowledge.stale_pct`, `knowledge.read_requests_pct`. See `metrics.md` (knowledge pages, request read knowledge).
- **The tracking metric** (in `oxplow-bundled`). The `oxplow.request_tracked` collector records, per stopped request that changed files, whether it has a task linked; the `tasks.tracked_requests_pct` metric is the share. See `metrics.md` (request tracked).
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

oxplow-foundation holds the data of the task tracker and knowledge: their
datasets, the `task` and `knowledge` resources, the `oxplow:oxwiki`
implementation and its setting (`configuration: oxwiki`, default on; turned
off by `oxplow.knowledge.set_oxwiki`, a script composing a settings write),
the bus commands (with no `ui:`), skills and metrics. oxplow's own
extensions share the namespace `oxplow`, so they share one configuration
(at one `version`) and one pair of commands over it, foundation's
`oxplow.settings.read` / `.write`; bundled's task workflow writes through
them too. It has
no page that a person uses to read or edit them. The task tracker's
commands (`oxplow.task.*`) and the generic `tasks` skill (`needs:
[task_tracker]`) are foundation's, serving any tracker; **oxtask itself is
oxplow-bundled's**: the `oxplow:oxtask` implementation, its `/next-task`
skill and the agent's tool pages `next-task` and `thread-tasks`. With only
foundation running there is no task tracker.

oxplow-bundled holds that UI: the pages above and the menu commands.
With oxplow-bundled off, a task or knowledge page link opens core's generic
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
- **`schema-review`** — a fact collector counting DDL per migration
  file (its own measure and dimension), and a slot page (Schema Changes)
  joining those facts with core's `v_turn_file` to list the migration
  files a request changed — no core change.
  `the_schema_review_example_joins_its_facts_with_core_evidence` runs it
  over a real request.

## The `oxplow-bundled` extension

**Two shipped extensions, and a holder that doesn't ship.** oxplow ships
`oxplow-foundation` (required) and `oxplow-bundled` (optional, on by default), both
in the `oxplow` namespace so a contribution can move between them without a rename.
Foundation is kept as small as it can be while still useful: the host's pages, the
default capability implementations (tasks, snapshots, knowledge)
and the measures core's change analysis records. `oxplow-bundled` is what has been
reviewed and kept.

`.oxplow/extensions/oxplow-bundled-todo` holds everything that used to be in
`oxplow-bundled` and hasn't been reviewed: this repo's, off, and not part of oxplow.
A contribution moves from it to `oxplow-bundled` one piece at a time, as it is
accepted, and the holder goes away when it is empty. Add nothing new there.

- **It's off, and can't run.** `disableExtensions` in `.oxplow/project.yaml` keeps it
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
- **Dashboards and review.** `review`, `quality` and `usage`;
  the code-quality `findings`; the `change-review` page (treemap,
  look-here-first, functions, co-change, zones) mounted in the
  `oxplow/diff-view.sections`, `commit.sections` and
  `uncommitted-changes.sections` slots. The duplicate scan runs in core's
  change analysis; the compare page is foundation's `duplicate-block`.
- **Request blocks.** Blocks for a request's tests (coverage, untested
  files, test runs, failed tests, analysis findings), its metric deltas, its
  nudges and its plan, mounted at `oxplow/diff-view.sections`.
  `thread-plan` and the two handoff strips mounted
  on the old Tasks page; its board has no slot, so they mount nowhere
  until they move.
- **Metric nudges.** The `threshold-crossed` advisory (with
  `metric-deltas`).
- **Git History's commit graph.** The `history-graph` component, replacing
  core's plain commit list (`vcs.history.graph`, a `ui.mounts`
  entry).
- **The hand-off screen.** `hand-off`. Its menu entry is the command
  `handoff.offer`. It runs the same operation as a foundation command and
  adds the label, shortcut and form a person uses. (The wiki screens have
  moved to `oxplow-bundled`: see "The knowledge pages" below.)
- **Explore Data.** The `explore-data` page and its component (source in
  `src/explore-data/`, built into `dist/` by `package.json`; the manifest's
  `build:` runs it). Foundation's dataset pages do not link to it.
- **Timeline.** The `timeline` page and its component (requests, sessions,
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
  `metric-data`, `measure`), which do not link to the list, and the
  `metrics-dimensions` query (the `metric` page's Slice by reads it;
  the list names it as `oxplow/metrics-dimensions`). People find
  a metric from the launcher instead: `metric` and `measure` are
  `searchable:` ([metrics.md](./metrics.md) "UI").
- **Zones.** The `zones` page (no category) holds the questions about the zone
  rules (`v_zone`, `v_zone_unmatched`); nothing links to it.
- **The app's screens.** foundation's `alerts`,
  `catalog`, `comments`, `commit`,
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
  agent waits on (`v_agent_status` `awaiting_user`, linked to its thread),
  an open handoff to a thread with no agent session, an open note. Its
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
  open tasks with `assignee = oxplow:oxplow/person` (`v_task`) - In
  progress, then Up next (todo and blocked) in the person's queue order
  (`rank IS NULL, rank, created_at`) - each with its stage, under its
  epic. A project-scoped panel (`scope: project`): the same in every
  thread, with no thread param. The header count is the open ones. It has **no Finished
  group**: a done task is unassigned and no record says who held it
  (`request_task` links threads' requests, a person's command is in no
  turn), so showing one would be a guess. Fixtures:
  `assigned-to-you-*`.
  How an agent uses tasks is the project's **task workflow**, the
  setting `tasks.workflow` of bundled's `configuration:` (version 2),
  with `tasks.definitionOfDone` (free text; empty is none). Set both with
  `oxplow.tasks.set_workflow { workflow?, definition_of_done? }` (a
  script: it merges into the project layer, keeping the other settings).
  The presets, by what oxtask offers (`when:` on each skill, offered only
  while oxtask is the active tracker):
  - `agent_tracks` (default): the `track-work` skill (take or file a task
    for new work, in progress before editing, done when fully done) and
    the `untracked-work` advisory: a request that changed files (a
    claimed or sole `v_turn_file`) while the thread had no task in
    progress is reminded once per request.
  - `person_files`: the `work-tasks` skill (work what is assigned or
    queued; file only follow-ups or when asked); no reminder.
  - `custom`: only `/next-task`; the project's own extension supplies its
    skill.
  - `off`: none of them.
  Both working skills say: the harness's own plan list is for steps inside
  a task, discovered work is a backlog task linked `discovered_from`, and
  done includes the definition of done. In every preset but `off`, while oxtask is
  the active tracker, its agent context `task-state` tells the agent the
  thread's task in progress, the next three in its ready queue (bundled's
  `v_oxplow_ready_queue`, the one definition `next-task` reads too: `todo`
  with no open blocker, an epic with its children as one unit, the pool
  with `thread_id` NULL) and the
  definition of done (nothing when there is none of them; core's
  `CONTEXT_CLEARED` tells an agent told before): once, then on change, and again after
  a compaction (extensions-commands.md "agent contexts"). Fixtures:
  `task-state-*`. Settings → Integrations shows it as **Task workflow**
  (`task-workflow-settings`, mounted at `oxplow/settings.integrations`):
  the four workflows with what the agent does in each, the current one
  marked, Use This to choose one for the project, and a form for what
  done means. Both it and `task-state` read the dataset
  `v_oxplow_task_workflow` (one row: `workflow`, `definition_of_done`).
  Fixtures: `task-workflow-settings-*`,
  `the-task-workflow-dataset-reads-the-setting-as-it-changes`. Fixtures:
  `the-reminder-is-only-for-the-agent-tracks-workflow`,
  `setting-the-task-workflow-keeps-the-definition-of-done`; the skills per
  preset are the Rust test
  `oxtask_offers_the_skills_of_the_projects_task_workflow`. It is `oxplow-bundled`'s, beside its
  `thread_tasks` dataset and `tasks.clear_finished` command, with a `panel:`
  fixture. Still in the holder: Comments (`comments-panel`). Each moves the same way: its parts, a
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

**The knowledge pages** (moved from the holder; written for URIs and
stores, not slugs). They are bundled's, so with `oxplow-bundled` off a
knowledge page link opens core's generic resource page. The data they read
is foundation's (`.context/knowledge.md` "Pages").

- `knowledge` (the Knowledge index, launcher section Knowledge): Stores
  (every knowledge row of `v_capability_provider`, on or off, with page
  and stale counts; with none on, a line says how to turn one on), Pages
  (newest change first; a filter over title and body; Delete in the row's
  right-click menu, confirmed, only for a store that can `edit`), Stale
  pages, and Recent reads and writes. New page opens `new-knowledge-page`.
  It also answers which pages cite a file (its `path` param).
- `knowledge-page` (`for: oxplow/knowledge`, default): the tab is the
  page's title. The body is the markdown, editable (Save) only when the
  page's store has `edit` on. The rail has About (store, kind, author, last
  edited, freshness, reads) and Open in its store (when the page has a
  `url`). Referenced files has Mark verified. See also: the index, the
  page's freshness, and what links to it.
- `knowledge-freshness` (params `store`, `id`): the page's files, stale
  ones first, Mark verified per file and for all.
- `new-knowledge-page`: a form of title and body. It has no id field, the
  store derives it. It asks for a store only when more than one store that
  oxplow writes is on; with one, the form sends that one. Its command is
  `knowledge.new_page` (menu: New Knowledge Page…), which opens the page the
  write answers (`open_after: {{result.page}}`).
- The body field is plain text in the form: the write's `body` carries no
  `contentMediaType: text/markdown`, which is what makes a form field the
  markdown editor. Fixtures: `knowledge-*`, `new-knowledge-page-*`,
  `the-knowledge-page-*`, `deleting-a-knowledge-page-*`. A store that can't
  edit is not a fixture: the throwaway has only oxwiki.

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
