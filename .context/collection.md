# Collection — effort-scoped observations

What this doc covers: oxplow's **collection** subsystem — structured,
provenance-tagged facts attached to a task effort (which tests ran, diff
coverage on the effort's changed lines, and static-analysis findings from
linters/analyzers). The same plumbing is meant to grow to perf deltas,
structure maps, etc.

## Why it exists

Everything else oxplow knows is either **computed by oxplow** (snapshots,
blame, code-quality scans) or **free text the agent wrote** (wiki). Test
results, coverage, and static-analysis findings are neither: they're
*structured* but language/framework/tool-specific, so oxplow can't compute
them generically. The bet is to split at the **standard-format seam** — the
agent does the language-specific part (configure the tool to emit a standard
report), oxplow does the generic part (parse it, attribute it, store it).

## Provenance is the spine

Every observation records whether oxplow **observed** it directly or the
agent **asserted** it. Developers reject self-reported agent numbers, so the
UI must never let an `asserted` figure pass for a measured one. Concretely:
oxplow parses coverage reports itself (→ `observed`); the agent never types
coverage numbers. See [data-model.md](./data-model.md)'s `effort_observation`
section for the column.

> **Storage retired (tsk215).** The `effort_observation` table + its store were
> **dropped** — the **metric substrate** ([metrics.md](./metrics.md)) is now the
> sole store for coverage/test/analysis facts. The hook ride-along records into
> `metric_sample` + `metric_finding` (the rich detail — test suite/case tree,
> coverage per-file uncovered lines, analysis payload — lives in verbatim
> `*-detail` findings). The `effort_evidence` asset computes each effort's
> observation rows from there (`CollectionService::effort_observations_from_metrics`)
> and stores them (`effort_observation_row`, read as `v_effort_observation`);
> the panel and MCP `list_effort_observations` read those stored rows
> (`SqliteEffortEvidenceStore::list_observations`, tsk862) — the evidence is
> computed once. `EffortObservation` is that row. Everything below about the **collector plugins**, the
> **hybrid ingestion** seam, and the **nudges** is unchanged — only the storage
> moved. (One micro-change: a *report-less* run — analyzer/tests ran but produced
> no parseable report — no longer leaves a "ran-record" row; the report-less
> nudge is what surfaces it.)

## Pieces

- **Effort-review rows** are reconstructed from the metric substrate
  (`effort_observations_from_metrics`), not a dedicated table. The
  `EffortObservation` wire type (`kind` ∈ `test-run`/`diff-coverage`/
  `static-analysis`, `metric_value`, `payload_json`, freshness pin) is the
  read/IPC shape only — see [metrics.md](./metrics.md) for the substrate.
- **Report collectors** (tsk863) — reading a report is a **collector**:
  a `collectors:` entry in `.oxplow/project.yaml` with `records: tests |
  coverage | analysis`, the `report: { path }` it reads, a parser in
  `entry`, and `trigger: { on_run: test | analysis }` (or none: by hand).
  One declaration, runtime, consent and run log for "read a report file",
  shared with every other collector (`.context/semantic-layer.md`
  "Collectors"). Validation (`crates/oxplow-config/src/collectors.rs`,
  `validate_report_collector`): the project's only (not an extension's);
  no `input`, `env`, `credentials`, `network`, `after` or `sync`; the
  trigger is `on_run` or `manual`, and an `on_run` matches what it records
  — `tests` / `coverage` on `test`, `analysis` on `analysis` (tsk892: each
  run's leg reads only its kinds, so another pairing would be parsed and
  dropped); a `records:` collector writes no facts or entities. Example:

  ```yaml
  collectors:
    - { id: tests.rust_coverage, records: coverage, entry: "oxplow:lcov", report: { path: target/coverage/lcov.info }, trigger: { on_run: test } }
    - { id: lint.clippy, records: analysis, entry: "oxplow:clippy", report: { path: target/clippy.json }, trigger: { on_run: analysis } }
  ```
- **Parsers — `oxplow-collect-plugin`** (`crates/oxplow-collect-plugin/`).
  A parser turns report text into a **typed output** for its kind
  (coverage = per-file `{ instrumented, covered }` line-sets; tests =
  `TestReport { suites → cases }`; analysis = `AnalysisReport { findings }`,
  each finding `{ path, line?, column?, severity, rule?, message }`). The
  typed shapes live in `oxplow-coverage`. `entry: "oxplow:<name>"` names a
  **bundled** parser: `junit`, `lcov`, `cobertura`, `jacoco`, `clippy`,
  `eslint`. **One table** holds them (`oxplow_config::collectors::
  BUNDLED_PARSERS`, tsk935): each one's name, what it records, how its
  report is pre-parsed and its jq program (`oxplow-config/src/parsers/
  *.jq`). The config check and the runtime (`Collector::bundled`, which
  reads it) can't disagree. Any other `entry` is the project's own `jaq` / `starlark` /
  `exec` parser (`runtime:`, `report.format` = the host pre-parse). There
  is no format registry: a collector names its parser. A parser writes
  paths as the report has them (`cargo llvm-cov` and eslint write absolute
  ones); `read_report` maps every coverage file and finding path to
  repo-relative against the checkout the report came from
  (`CollectorOutput::relative_to`, `oxplow_coverage::repo_relative`; tsk884)
  and drops files outside it, so diffs and facts always name repo files.
  That checkout is the **thread's own worktree** (`WorktreeRouter`, tsk890):
  a worktree stream's report paths, parser entries (and their approval
  hashes), branch and commits are read there, never in the primary
  checkout.
  The UI builds the test tree from `classname`+`name`.
- **`testing:` block** (`TestingConfig`, `crates/oxplow-config/src/lib.rs`):
  `command`, `fastCommand`, `runPatterns`, `analysisPatterns`,
  `agentHint` — how the project's tests run, read by the detector and the
  agent prompt. Human-only (its hint steers every agent). The old
  `collection:` block (with `reports:` and `plugins:`) is gone: the file
  rejects it as an unknown key (`a_collection_block_is_an_unknown_key`).

  **`fastCommand` (tsk171)** is the coverage-free counterpart to
  `command`, for the red/green loop. It must still emit a test report, but
  skips instrumentation and should accept a filter. It exists because
  `command` in a coverage-instrumented repo is far too slow to run every
  cycle (here: ~11s for the full suite vs 0.007s for one filtered test), so
  "route every invocation through it" was unfollowable — and an unfollowable
  rule doesn't degrade gracefully, it gets dropped entirely and NONE of the
  red→green runs get recorded. A weaker rule that is followed beats a stricter
  one that isn't.

  Both commands are also treated as implicit `runPatterns` by
  `on_post_tool_use`, so a fast command whose script name contains no
  built-in pattern (`bun run test:fast`) is still detected as a test run
  without having to be restated. All optional. Edits hot-reload via the
  config watcher (`ConfigWatcher`, see `git-integration.md`), so
  `/oxplow:configure` takes effect without a restart.
- **`/oxplow:configure` command** + **`oxplow-collection` skill** (assets in
  `crates/oxplow-plugin/`). `/configure` does two durable things: instruments
  the project's test tooling to emit a standard-format report at a stable
  path, and records the `testing:` block and the report collectors in
  `.oxplow/project.yaml`. The standing skill keeps
  coverage flowing after configure (run tests before closing a task; never
  type the numbers) so instrumentation doesn't bit-rot.

## Ingestion (hybrid)

Two paths feed the store (see [agent-model.md](./agent-model.md) for the
hook + MCP wiring):

- **Passive** — the PostToolUse Bash hook detects a test run (built-in
  patterns + `testing.runPatterns` + the two commands) and/or a
  static-analysis run (built-in patterns + `testing.analysisPatterns`, via
  `detect_analysis_run`) and records the matching observation(s) against
  the open effort. It then runs **every** report collector whose `on_run`
  is that kind of run and whose report the run wrote (its mtime inside the
  run's freshness window) — `read_run_reports` in `collection.rs`, each
  through `read_report`: a disabled collector, an unapproved program or a
  report that isn't there runs nothing; anything that ran is the
  collector's run (`collector_run` + `collector.synced`, through the
  collector runner's `RunLog`, deduped per detected run) and counts toward
  its health — the third failed parse in a row disables it (P7.C2,
  `plugin_health`); an `exec` parser nobody approved records
  `needs_approval` and doesn't run. A test run only reads `on_run: test`
  collectors, an analyzer run only `on_run: analysis` ones. What they
  parsed merges by kind (`RunReports`): JUnit reports into one
  suite/case tree embedded in the `test-run` payload (`suites`) — the
  `junit.jq` plugin takes each `<testcase>` from its IMMEDIATE parent
  `<testsuite>`'s direct children (NOT a recursive descent), so a NESTED
  testsuite (bun emits file-suite → describe-suite → testcase) doesn't
  double-count a case under both levels (tsk361); coverage
  reports into one coverage capture (line sets union, branch/function
  counters sum, tsk160) — the effort's diff coverage is derived from it;
  analysis reports into one `static-analysis` observation (findings +
  per-severity counts). All `observed`, no agent step.
  **Attribution (tsk347):** the run is pinned to its effort via the `"run"`
  ledger. An agent forces EXACT attribution by prefixing the command with
  `OXPLOW_TASK=<task id>` — `parse_task_token` reads it and
  `CollectionService::record_test_run` claims the run for that task's open effort (`find_open_for_work_item`), correct even
  under concurrent efforts. Without the token, resolution is, in order:
  **single open effort** → **target overlap** (tsk169: score each open effort by
  what the command names — `-p <crate>`, path args — against the files it has
  claimed **union the paths its task's own text names**, and take a STRICT
  unique winner) → **unattributed**.

  The task-text half is not a nicety (tsk185). Claimed files can legitimately be
  EMPTY, for two reasons that have nothing to do with timing — file claiming is
  SYNCHRONOUS on the PostToolUse hook, not a race (a first diagnosis said
  otherwise and was wrong):
  1. a brand-new effort has claimed nothing yet, and
  2. a file written through **Bash** (codegen, a formatter, an agent using a
     heredoc) is deliberately never auto-claimed — only Edit / Write /
     MultiEdit / NotebookEdit are, so those writes stay for snapshot
     reconciliation and will always surface at close.

  Files alone therefore cannot resolve the run you most want recorded.
  `task_target_paths` reads only `[[wikilinks]]` and backticked spans that look
  like paths (trimming a `:42` line suffix); scraping free prose would invent
  matches. Union rather than fallback: widening the pool can only create ties,
  which the strict-winner rule declines on. Ties and
  whole-suite runs that name nothing decline on purpose: a mis-attributed run is
  worse than an unattributed one, because the agent can still claim the latter at
  close. An unattributed test run with 2+ efforts open fires the
  `unattributed-run` nudge immediately (tsk170) rather than waiting for the
  closing EFFORT REVIEW.

  **The same decision governs the per-file auto-claim** (tsk186).
  `claim_open_effort_file` used to return early whenever more than one effort was
  open — "we can't know which one edited the file" — which switched claiming off
  precisely when attribution is hardest, and compounded: run scoring reads
  claimed files, so an unclaimed file also meant unattributed runs and a
  close-time reconcile by hand. It now calls the same
  `attribution::resolve_by_targets`, scoring the EDITED PATH instead of a
  command's targets. One implementation for both call sites on purpose — a file
  claim and a run claim must never disagree about which effort owns the work.

  **The filing discipline and attribution pull in opposite directions, and
  nothing else warns you.** "One user-visible concern per row" encourages many
  small tasks; batching several in one session means several efforts open at
  once, which is exactly when auto-attribution has to decline. Either
  **serialize** (close each task before starting the next) or **prefix every run
  with `OXPLOW_TASK=`**. Doing neither is what produces a closing audit full of
  unattributed runs to hand-reconcile — the failure mode tsk169/tsk170 exist to
  shrink, not to eliminate.
  **Detection is run-aware (tsk347):** `detect_test_run`/`detect_analysis_run`
  split the command on shell operators (`&&`/`||`/`;`/`|`) and ignore
  sub-commands whose leading executable only *reads* (grep/echo/cat/sed/…), so a
  command that merely MENTIONS a pattern (`grep test:collect .oxplow/project.yaml`) is no
  longer a phantom run (and fires no report-less nudge); leading `VAR=val` env
  assignments are skipped so the `OXPLOW_TASK=` prefix doesn't mask the real
  exec. **Background caveat:** the PostToolUse hook fires when the Bash call
  *returns*; a **backgrounded** `test:collect` returns at launch (before its
  reports regenerate), so nothing fresh is ingested — run it in the FOREGROUND.
  **The recording is a pump reactor, not part of the hook (P3.6, tsk476).**
  The ingest logs `agent.tool.finished` (the command and its output in
  `event_content`); the `collection` async consumer
  (`crates/oxplow-app/src/post_tool_reactors.rs`) rebuilds the Bash payload
  from it and runs `CollectionService::on_post_tool_use`. A run's
  recording can outlive the hook's 5 s budget (a debug-build junit ingest +
  a multi-MB lcov parse) and always completes; a crash re-delivers the
  event. **Redelivery records nothing twice:** the test-run capture's
  `idempotency_key` is `test-run:<event id>` (coverage and analysis captures
  were already keyed by their report's content, a coverage failure by
  `coverage-failure:<event id>`), and nudges are unique by `(cause, kind)`.
  **The effort the command ran in owns the run** — the event's effort
  anchor ranks after an `OXPLOW_TASK=` token and before the thread's open
  efforts (`resolve_owner`) — so a run the reactor records after `Stop` +
  the close (`work_item.transition` → done) closed the effort still lands on it.
  A test, coverage or analysis run resolves its owner **once**: the same
  effort stamps its capture, receives its ledger claim and pins its take
  (tsk926). Each run capture logs
  `test.run.recorded` (subject `run:<capture>`, anchored to the tool's turn,
  caused by the tool event) and each coverage capture `test.coverage.recorded`
  in the capture's transaction (`SqliteFactStore::record_facts_logged`); the
  `test.record_run` command and a by-hand `collector.sync` of a report
  collector log them too, anchored
  to the thread (in the collector's transaction: they're `External`, so their
  events aren't caused by the run's `command.executed`). The hook waits ≤2.5 s for the `collection` and
  `advisories.post_tool` consumers (`EventPump::settle`; the advisories
  consumer declares `after: [collection]`, so it sees a run only once
  collection has recorded it — an advisory reading `v_effort_observation`
  judges the run's own coverage, tsk506) and then returns the thread's
  **undelivered** nudges; one that
  lands later goes out on the thread's next tool call.
  **Landed commits also feed the wasted-token leg (tsk77):** any detected
  commit — including `git revert`, which needs its own `detect_git_revert`
  since the command never says "commit" — has HEAD's `This reverts commit`
  trailers read; a reverted commit attributable to exactly one CLOSED effort
  emits that effort's spend onto `oxplow.token_waste` (see metrics.md).
  **The legs are isolated and coverage retries (tsk79):** the analysis,
  test-run, and coverage legs each catch their own error (one leg's transient
  failure can't kill the legs after it), and the coverage leg retries once
  after `COVERAGE_RETRY_DELAY` — right after a test run, DB contention or a
  snapshot-lookup hiccup is transient, and without the retry a single
  swallowed error meant that run's coverage never existed. When both attempts
  lose — or a FRESH report exists but fails to parse — the miss is durable: a
  facts-empty `status = failed` coverage capture carrying the error (the
  fact-collector failure convention), queryable in the substrate instead of living
  only in a tty warn.
  **Clippy needs `bun run lint:collect`:** nothing else writes
  `target/clippy.json` (plain `cargo clippy` prints human output), so the
  `oxplow.analysis.*` metrics only populate when clippy runs via the
  `lint:collect` script (JSON to the configured report path; also in
  `testing.analysisPatterns` so the `bun run` command string is detected).
  **Freshness is the router:** a run only regenerates its own stack's/tool's
  report(s), so the mtime window (`FreshWindow::of_run`: written after the
  run's own start, less a second of mtime granularity, and by a minute
  after it ended — tsk888) naturally excludes the other stacks' reports — a `bun test` run
  picks up the frontend reports, a `cargo cov` run the Rust ones, a
  `cargo clippy` run the clippy findings, and all accrue within one effort.
  The UI builds a tech-natural tree by splitting each case's
  `classname`+`name` on `::`/`.`. A `static-analysis` observation doubles as
  the analyzer-ran record: when an analyzer is detected but regenerated no
  parseable report, it's stored command-only (no findings, no metric), the
  same way a `test-run` records command-only when no JUnit report is fresh.
- **Active (commands)** — `collector.sync { owner: "project", id,
  thread? }` runs a report collector by hand
  (`CollectionService::sync_report_collector`, tsk863): it reads the
  report as it is now, whenever it was written, records what it parsed in
  the thread (an agent's own; a person names one) like a detected run's —
  a test run, a coverage capture or a static-analysis capture, with
  `collector.sync project/<id>` as the run's command — and answers
  `{ recorded: { status, records, run } }` (`run:<capture>`, what
  `claim_runs` takes — the real capture for every kind, tsk891).
  `status` is `stored`, or why nothing landed: `no_stream`, `no_cases`
  (tests), `no_coverage` (nothing instrumented, or no coverage measure),
  `metric_off` (no enabled metric reads analysis). A failed write is an
  error, never `stored`. The run is the collector's (`trigger: manual`). It
  replaced `test.ingest_coverage` / `test.ingest_analysis` (P8.A8). **No
  baseline gate** for analysis: findings are *absolute* (current-file),
  so they store even when the effort has no start snapshot (tsk86).
  `test.record_run` is the one `asserted` writer, for richer
  pass/fail counts the exit code alone can't give; its counts also become
  status-sliced `oxplow.test_case` facts (no case identity) so the
  `oxplow.tests.*` specs read them, and it returns the capture id (the run
  identity `claim_runs` refs use). A report-less, count-less run records its
  capture under the `test-run` producer so it never reads as "found 0 tests"
  (see [metrics.md](./metrics.md)). The run capture also stamps
  **`closest_vcs_rev`/`vcs_rev_exact`** (tsk95) — the commit it tested,
  resolved via `file_ref_version::resolve` off the stream's latest snapshot,
  falling back to HEAD with `exact = false` when the tree is dirty (the normal
  case). This is the fold's only ancestry material and is **not backfillable**;
  see the stamping note in [metrics.md](./metrics.md).
  **A run reported any other way has its coverage read from its event**
  (tsk1015): the `collection.run_reports` pump consumer reacts to
  `test.run.recorded` and runs `CollectionService::on_test_run_recorded`
  — the project's `on_run: test` **coverage** collectors whose report was
  written around the run's time (`FreshWindow::around`), recorded as a
  detected run's coverage is, owned by the run's effort (the event's
  anchors; `RunOrigin::Event` over the run event, which also dedupes the
  collector's run per reported run). One routine for every way a run is
  reported: a run logged **with a cause** is one the collection reactor
  detected in a tool call and read inline — the hook's same-call
  advisories and report-less nudge depend on it — so its event reads
  nothing more; `test.record_run` and a by-hand sync of a test-report
  collector have theirs read here. A run's own **test** report isn't
  read from the event: it *is* the run, and reading it after the run was
  recorded would split one run across two rows (`v_test_run`'s grain).

**Observe-always (tsk269/tsk270).** Tests, analysis, **and coverage** are recorded
**regardless of how many efforts are open** — attribution is deferred to the
unified `"run"` ledger, never a precondition for recording. `on_post_tool_use`
resolves a single open effort only for the effort-RELATIVE *advisories*
(the report-less / coverage-target nudges), which legitimately
no-op under 0/N efforts; every OBSERVE call runs unconditionally. Report freshness
is a **window around the run** (`FreshWindow::of_run`): a report counts when its
mtime is after the run **started** and at most a minute after it ended (tsk888)
— a report written before the run began is an earlier run's, however recent.
The start is the call's `agent.tool.requested`, which hook ingest makes the
`agent.tool.finished` event's `cause` (one tool use id); `RunCause.started`
carries its time. With no start known (no tool use id), the window reaches
10 minutes back from the run's event. It is
judged at the event's own time (`RunCause.at`), not at delivery, so a redelivery
(a crash before the checkpoint, a retried dead letter, a pump backlog) sees what
the first delivery saw; a run delivered more than 10 minutes late is recorded
but gets no nudges (tsk505). A by-hand `collector.sync` reads the report
whenever it was written. Neither needs an open effort. **A run's effort is the one its event was anchored
to** (`run_effort`, tsk507) — for the effort-relative advisories, the
static-analysis snapshot pin and the `oxplow.nudge` fact — so a late delivery
keeps the effort it ran in; only a live call with no event resolves the
thread's single open effort now. **Coverage** is
effort-relative (diff vs the effort's start snapshot), so it can't store the diff
at record: `observe_coverage` stores the **absolute** whole-report coverage
(per-file coverage facts + the instrumented/covered line-sets in the capture's
`coverage-detail` detail envelope — `metric_capture.detail_json`, T-E1), and the
effort-relative diff is DERIVED with the effort's evidence
(`diff_coverage_for_effort`) — so a coverage run claimed *after* the effort closed
still produces a diff. The diff is between two **snapshots**, never a working
tree (tsk862): the effort's start snapshot and the one the coverage capture is
pinned to (`metric_capture.snapshot_id`). Recording a run's coverage takes that
snapshot (`measured_snapshot`, trigger `run_measured`; tsk883 — an analysis run's capture is pinned the same way, tsk937): the stream's
worktree as the report is read — the code the run measured, even when the
agent edited and ran in one turn, which no other take sees (turn-end and quiet
takes never run mid-turn). A run delivered after an edit landed (a file in the
take written after the run's event time) is recorded with **no pin**, so no
diff rather than one against code it never ran. So it holds for a worktree
stream, and an edit after the run moves nothing. A path new since the start counts all its
lines as changed; a side whose bytes were collected (blob GC) gives **no row**,
not "every line changed"; an oversize file (no bytes kept) contributes nothing. The earlier `find_single_open_for_thread` *drop-gates* on
the producers are gone — the helper stays only as the Class-A auto-attribute
optimization.

**Sub-agent runs + cross-agent attribution (tsk265).** The passive PostToolUse
path only sees the **parent agent's** tool calls — Claude/Codex sub-agent (Task
tool) tool calls don't fire the parent's hook, so a dispatched sub-agent's
`cargo test` is **invisible to passive collection**. oxplow deliberately does
NOT try to recover it by reading sub-agent transcripts / `SubagentStop` /
`agent_id` (all agent-specific and version-fragile). Instead, attribution rides
the two cross-agent-stable surfaces oxplow owns: the filesystem snapshot (which
runs don't touch) and the **MCP contract**. So a sub-agent records its runs
through `test.record_run`, passing `work_item` so the run attributes EXACTLY to
its effort even under concurrency (resolved via `find_open_for_work_item`); the
`dispatch_task` brief instructs this. Without a named task, a run attributes
automatically when one effort is open, else is left unclaimed for the close
reconcile + window-dominance + the agent's claim — never guessed onto one.
`claim_runs`/`disclaim_runs` on `effort.report` (the close's second call) let
the agent fix attribution at the close boundary; `effort.amend` does it after the fact. See
[agent-model.md](./agent-model.md) for the full claim→reconcile loop.

Both paths classify by the collector's `records:` (its parser's kind), not a
format-name heuristic. Trust tier rides in `source`: in-process tiers
(jaq/Starlark) are deterministic and do no I/O → `post-tool-bash` /
`coverage-report` / `analysis-report`; an `exec` parser can do I/O, so its
output is tagged `plugin-exec:<collector ids>` (`trust`) so the UI can mark it
lower-trust. The `provenance` column stays `observed` vs `asserted`.

The `static-analysis` payload is `{ command?, analyzer?, findings:[…],
errorCount, warningCount, infoCount, noteCount }`; its `metric_value` is the
error+warning count (**lower is better**, unlike coverage where higher is
better). The effort review shows the latest run's findings in the
oxplow-analytics *Static Analysis* lens (part of its `effort-tests` grid,
mounted in the `effort.review.details` slot), each row opening the file at the line.
The analysis ride-along has **no nudge** — the report-less nudge is
test-specific.

**The task-page effort section never shows Coverage & tests.** On the task
page's Activity timeline (`TaskDetail.tsx` → `ActivityTimeline`):
- An **in-progress** effort (`ended_at`/`end_snapshot_id` null) renders a
  minimal `ActiveEffortSection` — just an "In progress" header band
  (`tasks-effort-in-progress` testid), no changed-files tree, no summary, no
  fetches.
- A **completed** effort (`ActivityEffortSection`) shows the summary, the
  **Modified Files** tree (token usage is the task-level `task-tokens`
  lens in the `work_item.detail.body` slot) — but
  **not** coverage, test runs or static analysis. Those live only on the
  effort **diff view** (`DiffViewPage`, the effort-review surface), as the
  oxplow-analytics `effort-tests` lens grid in its `effort.review.details` slot:
  diff coverage, most-untested files, test runs, tests that failed (with
  their latest status, so a red→green loop reads plainly) and analyzer
  findings, all SQL over `v_effort_observation`'s payloads.

## Report-less-run nudge (PostToolUse)

When the PostToolUse hook detects a test run but no configured report
was refreshed by it (the agent ran `bun test` instead of the
report-emitting `bun run test:collect`, for example), the `collection`
reactor persists a one-shot nudge, which the thread's next tool-hook
response delivers (`take_undelivered`; see Nudge persistence). The nudge names the project's
own `testing.command` when set — and, when a `fastCommand` is declared,
offers that for iterating and `command` for the closing run (this repo's
`agentHint` says the same) — says the run wrote none of the reports the
project's report collectors read when there are some but no `command`, or
routes to `/oxplow:configure` when the project reads no reports at all.
When a collector **should** have read a report but didn't count — its
parse failed, it is disabled, or its program awaits approval — the nudge
says that instead, collector by collector (`RunReports::unread`,
`unread_reports_message`; tsk893): running the tests again wouldn't help,
and the reasons name what a person must do. A coverage collector that
didn't count also leaves the lost coverage's `failed` capture.

**Tool-agnostic design:** the hook never encodes tool→command knowledge.
It keys only on (1) "was this a test run?" (substring match against
built-in patterns + `runPatterns`) and (2) "did a report collector read a
report this run wrote?" (its mtime inside the run's freshness window). The tool-specific command it
names comes entirely from the project's config, so it works for any
test tool, current or future.

**Anti-nag:** the nudge fires at most once per effort — a durable one-shot
mark (`effort_once_mark`, `claim_once`), so a daemon restart doesn't re-arm
it. The *fired* nudge itself is persisted for review and delivery — see
Nudge persistence below.

## Commits get no nudge of their own

`on_post_tool_use` detects a `git commit` (`detect_git_commit` — token-aware,
so `git -c user.email=… commit` and `git commit --amend` match, `git add` /
`git log --grep commit` don't) only to drive the revert/token-waste leg and to
return early. There is **no commit-hygiene guard** (removed in tsk250): a
post-commit "these files aren't part of this effort's changed set" advisory
used to fire here.

The reasoning for removing it, worth keeping: attribution nudges earn their
keep by disentangling **concurrent** efforts — which changes belong to which
piece of work while several are in flight. A commit has one actor that has
already decided what to include, so there is nothing left to disentangle and
the nudge is just oxplow getting in the way. It also carried a hardcoded
`docs/` check that warned about *this* repo's `.github/workflows/docs.yml`
auto-deploy — the kind of assumption about a project's layout that a
general-purpose tool must not make (see also [[tsk251]]).

## Nudge persistence

PostToolUse nudges (report-less-run, and post-tool-use advisories such as
oxplow-analytics' `coverage-target`) are **persisted**
as well as returned to the agent, so a reviewer can see "what oxplow told the
agent this effort" after the fact, and the persisted row is what delivers it
(the next tool-hook response takes the thread's undelivered nudges). When `on_post_tool_use` decides to
return a nudge, it also calls `persist_nudge` (best-effort — a write error is
logged via `tracing::warn!` and swallowed, never failing the hook), which
records a row in the `agent_nudge` table tagged with `kind`
(`report-less-run` / `coverage-target`), the message, and the trigger (the
bash command); a view of `v_agent_nudge` re-runs on `ModelsChanged`. Persistence sits **after** the
durable dedup gate (`mark_nudged`), so a deduped/non-fired nudge is never
stored. The store
(`SqliteAgentNudgeStore`), IPC (`list_nudges_for_thread`), the
`v_agent_nudge` view and the oxplow-analytics `effort-nudges` lens are
covered in [data-model.md](./data-model.md),
[ipc-and-stores.md](./ipc-and-stores.md), and
[agent-model.md](./agent-model.md) (Nudge persistence).

## Adding a new observation kind

1. Pick a `kind` string and a `payload_json` shape (parsed in TS / by the
   agent — opaque to Rust, so no migration to enrich it).
2. Record the run as a capture whose `detail_json` envelope carries that
   kind, with the right `provenance` and a snapshot pin, and map the envelope
   kind in `CollectionService::effort_observations_from_metrics` — the
   `effort_evidence` asset then stores it with the effort's other rows.
3. Surface it on the effort-review UI (it reads `v_effort_observation`).

Prefer `observed` over `asserted` wherever oxplow can compute or parse the
fact itself — that's the difference between an understanding surface and a
dashboard of numbers nobody trusts.

## Pluggable parsers (collector plugins)

Report parsing is a **two-layer** design so a new format is a report
collector + a small script, never a Rust change (`crates/oxplow-collect-plugin/`):

1. **Container parse (host-owned).** The host reads the report file(s) and, per
   the collector's declared `input`, normalizes the bytes into a generic JSON
   value via shipped helpers. Scripts never touch the filesystem — that's what
   keeps an in-process parse deterministic and `observed`-eligible.
2. **Field mapping (plugin-owned).** A *collector* maps that value into its
   kind's typed output. There is **never a formless observation** — every
   collector declares a `kind` (`coverage` | `test` | `analysis`) with
   a fixed output schema. The genericity is in this uniform definition mechanism
   over typed kinds, so a future kind (perf, structure-map, …) is a new
   `CollectorKind` plus plugins that target it — not a new subsystem.
   (`analysis` was added exactly this way: a new `CollectorKind`, the
   `AnalysisReport` typed output, and bundled clippy/eslint jaq plugins — no
   new store, IPC, or subsystem.) The metric substrate's author-able
   producers are not a `CollectorKind`: they are **fact collectors** (a
   `collectors:` entry with `facts:`, P7.B3 — they were a `gauge` kind
   until then), run by the fact engine through the same jaq/starlark/exec
   tiers; see [metrics.md](./metrics.md).

**Transform tiers** (trust/preference order): `jaq` (jq, pure Rust — primary,
JSON→JSON reshaping), `starlark` (general/imperative; note: standard Starlark
forbids recursion + `while`, so deep tree-walks are impractical — jaq suits XML
better), `exec` (external process, JSON stdin→stdout — the escape hatch; can do
I/O, so it's tagged lower-trust, and **runs only once a person approved it on
this machine** — a project collector's program (`ProgramKind::Collector`),
approved in Settings → Data → Programs like any other; until then its run is
recorded `needs_approval` and nothing is read (see architecture.md → "A
repo's config never runs a program without consent"). All three tiers run under a `SandboxBudget`
(wall-clock timeout) so a runaway/malformed script is surfaced as an error, not
a hang.

**`exec` is the one tier whose budget actually STOPS the work** (tsk161): a
child process can be killed, so `run_exec` enforces the deadline with
`try_wait` + `kill` rather than detaching. It also drains stdout and stderr on
their own threads while writing stdin from a third. Doing that in sequence —
write the whole report, *then* `wait_with_output` — deadlocks any streaming
filter the moment a pipe buffer (~64 KB) fills: the child blocks writing
stdout, so it stops reading stdin, so the parent blocks writing. Reproduced
with multiple MB through `cat`, and it had no budget to break out of it.

> ### ⚠️ The sandbox timeout bounds the CALLER'S WAIT, not the WORK (tsk88)
>
> `run_sandboxed` runs the script on a worker thread and `recv_timeout`s. Rust
> can't kill a thread, so on overrun the worker is **detached and keeps burning a
> core** until it finishes on its own. Two consequences:
>
> - **Tightening the budget costs CPU instead of saving it.** The caller gives up
>   and is free to retry — and the coverage ride-along *does* retry (tsk79) — so a
>   marginal parse means *two* workers on the same input, not one.
> - **It is not containment** *for the in-process tiers*. A hostile/infinite jaq
>   or Starlark script detaches and spins forever; the budget only hides it from
>   the caller. Real containment needs a step-limited interpreter or a killable
>   child process — which is exactly what `exec` now has, so this caveat applies
>   to `jaq`/`starlark` only. Until then the number is
>   a **diagnostic ceiling for honest-but-slow scripts** (120s, matching
>   `FACT_COLLECTOR_TIMEOUT`) and must be set generously enough that honest ones never trip.
>
> **This same shape of bug has now bitten three times** — a fixed timeout sized
> for a small input silently killing whole-workspace work, and reporting it as
> nothing rather than as a failure:
> 1. **tsk47** — tree scans (then "gauges") timed out at 5s on every full-tree scan and wrote nothing
>    (`oxplow.ts.console_calls` read 0 against 137 real calls).
> 2. **tsk62** — the 5s *hook response* budget cancelled the coverage step after
>    the junit ingest on EVERY run, naming "a multi-MB lcov parse" as the cause.
>    Fixed by detaching the recording from the hook response…
> 3. **tsk88** — …at which point the same multi-MB lcov parse died at the 5s
>    *sandbox* budget instead: one layer down, and **intermittent** rather than
>    total. The real parse takes ~2.8s against a 5s budget, so it failed only
>    under load — `metric_capture` showed **11 `done` (196 facts each) against 7
>    `failed`**, i.e. ~39% of runs silently lost their coverage. The lcov plugin
>    was also quadratic per file (`+= [$n]` in a `reduce` copies the growing array
>    — one 4783-line file cost ~11M element copies); it's `map`-based and linear
>    now, pinned by `lcov_plugin_cost_stays_linear_in_lines_per_file`.
>
> The lesson for any new budget: **size it against a whole-workspace report in a
> DEBUG build** (the interpreter runs ~6x slower there, and that's what developers
> actually run), and remember that a timeout here is a diagnostic, not a limit.
>
> **And a lesson about reading the evidence:** tsk88 was first written up as
> "coverage never got a fact", borrowing tsk62's wording. One `GROUP BY status`
> over `metric_capture` disproved it. *Intermittent* was the stronger clue anyway
> — a marginal budget fails under load, which is exactly what a 60/40 split looks
> like, whereas "never" would have pointed somewhere else entirely. When a
> producer looks broken, **query `metric_capture` for its `status` mix before
> describing the failure**; "always" and "sometimes" have different causes.

**Container `input` kinds** — how the host pre-parses the report before the
transform (all yield a JSON value): `text` (raw string), `json`, `xml`
(explicit ordered tree `{tag, attrs, text?, children}`), `lcov` (array of
records, each key→array), `lines` (array of strings). `exec` always receives
raw content on stdin (ignores `input`).

**Starlark host builtins.** Beyond the pre-parsed `input`, a Starlark plugin
can call the layer-1 helpers directly as globals —
`parse_xml`/`parse_json`/`lcov_records`/`lines`/`regex_find`/`xpath` — so it can
self-parse raw text (set `input: text` and parse inside `transform`). These are
Starlark-only: **jaq can't call host functions**, which is why the bundled jaq
parsers pre-parse via `input` instead. (Standard Starlark forbids recursion +
`while`, so deep tree-walks are still awkward there — for XML, jaq remains the
easier fit.)

More globals back **fact collectors** (the metric substrate's author-able
producers — see [metrics.md](./metrics.md)); a fact collector can't call
the `ai_*` builtins, and an entity collector has no `files()`:
- `ast_query(text, language, sexpr)` → a flat `[{capture, text, start_row,
  start_col, end_row, end_col}]` list. Parses `text` with the named tree-sitter
  grammar (`rust`/`typescript`/`tsx`/`javascript`/`python`/`go`/`java`/`c`/`cpp`/
  `clojure`) and runs the S-expression `sexpr`. Pure (text inline →
  deterministic → `observed`); backed by `oxplow-code-metrics` (`ast`/`parse`/
  `query`). Flat by design so no Starlark recursion is needed.
- `files(glob)` → `[{path, text}]` of the **snapshot** files matching `glob`,
  from an in-memory map the host injects per run via `Evaluator::extra` (a
  `TreeHost`). Empty when no file matches. The snapshot is
  content-addressed/immutable → determinism + `observed` trust hold. A fact
  collector's script runs with a host via `run_fact_starlark(script, input,
  TreeHost::new(map), …)`; its `report:` is pre-parsed by `parse_report`.
- `code_metrics(text, language)` → per-function `[{name, complexity, length,
  parameter_count, start_line, end_line, visibility, has_doc}]` via
  `oxplow-code-metrics`. `has_doc` (tsk125) is per-language doc detection
  (`LanguageSpec::doc`): a doc comment immediately preceding the item —
  `///`/`/**`/… by prefix, NOT a plain `//` (except Go, where any preceding
  comment is the doc) — or a Python/Clojure docstring. Backs `oxplow.doc_coverage`.
- **The language-agnostic capability layer** (tsk314) — for metrics that are the
  *same concept across languages* (TODOs, complexity, …), a metric shouldn't
  name a language. Two globals make that possible:
  - `source_files()` → `[{path, text, language}]` — every **recognized** source
    file from the host (filtered by `oxplow-code-metrics::is_supported_path`),
    each tagged with its detected `language`. This is the reader: a script sweeps
    it and never writes a glob or names a language.
  - `markers(text, language)` → `[{line, kind, text}]` — TODO/FIXME/HACK/XXX/BUG
    comment markers, comment-aware via the grammar.
  Per-language knowledge (grammars, extensions, comment scanning) lives in
  `oxplow-code-metrics`; metrics are defined once on these capabilities (the
  `plugins/metrics/code/*.star` set). Adding a language → no metric changes.

**Output schemas** the transform must produce:
- coverage: `{ "files": { "<path>": { "instrumented": [<line>…], "covered": [<line>…], "branchesFound"?: <n>, "branchesHit"?: <n>, "functionsFound"?: <n>, "functionsHit"?: <n> } } }` — branch/function are optional **counts** (a line holds several branches; functions are named), default 0 = "no such data for this file" (tsk123). lcov emits them from `BRF`/`BRH`/`FNF`/`FNH`; jacoco from the sourcefile `<counter type="BRANCH"/"METHOD">`; cobertura branch from per-line `condition-coverage="H% (a/b)"` (direct `<lines>` only, so method `<lines>` don't double-count) and function from `<method>` `line-rate`.
- test: `{ "suites": [ { "name", "cases": [ { "classname", "name", "status": "passed|failed|skipped", "timeMs"? } ] } ] }`
- analysis: `{ "findings": [ { "path", "line"?, "column"?, "severity": "error|warning|info|note", "rule"?, "message" } ] }`
- fact collector: `{ "facts": [ { "measure", "value", "subject"?, "path"?, "line"?, "rule"?, "num"?, "den"?, "dims"? } ] }` and nothing else — `facts_of` refuses any other output (the old `samples`/`findings` channels are gone, P7.B3). See [metrics.md](./metrics.md). Facts are the durable atomic grain of the inverted substrate (epic tsk12): each is bound to a defined `measure` (which must be in the collector's `facts:` allow-list) and re-aggregated by a metric *spec* at read time. `num`/`den` are optional ratio components (a `ratio` spec re-derives Σnum/Σden). `rule` populates the fact's `rule` column (the `oxplow.rule` dimension — the per-language idiom collectors tag each `oxplow.ast_hit` fact with the idiom slug there). A fact on an **undefined** measure — or one outside the collector's `facts:` — is a declare-to-collect violation (dropped with a warn).

The two bundled analysis plugins are the canonical templates: `clippy.jq`
(`input: lines`; `fromjson?` per line tolerates non-JSON lines, keeps
`reason=="compiler-message"`, picks the primary span, maps `level` →
severity) and `eslint.jq` (`input: json`; severity `2`→error / `1`→warning,
null `ruleId` → no rule).

### Authoring a parser

Declare a report collector with its own `entry` — no recompile. The
**script lives in its own file** (project-relative; absolute paths and `..`
are rejected), not inline in the yaml. The entry and the report `path` are
also resolved through every symlink before anything is read, and one that
lands outside the checkout is refused (`in_checkout`, tsk927): a repo can't
point a collector at a file of the person's. Example: a Clover (XML) coverage
parser in jaq:

```yaml
collectors:
  - id: tests.clover
    records: coverage            # tests | coverage | analysis
    runtime: jaq                 # jaq | starlark | exec
    entry: oxplow/parsers/clover.jq
    report: { path: target/clover.xml, format: xml }   # text | json | xml | lcov | lines
    trigger: { on_run: test }
```

```jq
# oxplow/parsers/clover.jq — input value (.) → coverage output schema
{ files: reduce ([.. | select((type=="object") and (.tag=="file"))][]) as $f
    ({}; . + { ($f.attrs.path): {
        instrumented: [ $f | .. | select(.tag?=="line") | (.attrs.num|tonumber?) ],
        covered:      [ $f | .. | select((.tag?=="line") and ((.attrs.count//"0")|tonumber? // 0)>0) | (.attrs.num|tonumber?) ] } }) }
```

The host reads the script (it still does no I/O, so determinism holds). For
`starlark`, point `entry` at a `.star` file defining `def transform(input): …
return {…}` (the host appends the `json.encode(transform(...))` call; the
`json` stdlib **and** the `parse_xml`/`parse_json`/`lcov_records`/`lines`/
`regex_find`/`xpath` host builtins are available, so a Starlark parser can
self-parse raw `format: text`). For `exec`, `entry` is the program to spawn
(executable, with a shebang); it gets the raw report on stdin (so it takes no
`format`) and must print the kind's JSON to stdout.

The bundled parsers in `src/plugins/*.jq` are the canonical templates; each
is pinned by tests of its output over a real report
(`crates/oxplow-collect-plugin/src/lib.rs`).

### Report-derived RATIO metrics (a fact collector, not the ride-along)

A tool that emits a whole-project **ratio** (not line-sets/findings) — e.g. TS
`type-coverage`'s `--json-output` (`{correctCount, totalCount, percent}`) — is a
**fact collector** with `report: { path, format }`, NOT a report collector
(`records:` reads only the `tests`/`coverage`/`analysis` kinds).
The fact engine reads the report, parses it per `format` (`parse_report`) and
hands it to the jaq/starlark script as `input.report`; the script returns a
`{facts:[{measure, value, num, den}]}` ratio fact. tsk126 dogfoods this as
the `repo.scan_type_coverage` collector → `repo.type_coverage`
(`oxplow/plugins/type_coverage.jq`, `format: text` + `try fromjson catch null`
so a missing report emits nothing rather than failing the collector; report at `target/type-coverage.json`, regenerated by the
`type:coverage` package script). **Two TypeScripts on purpose:** `apps/desktop`
typechecks with TS 7 (the native compiler, no JS API), but `type-coverage` is
built on the JS compiler API, so it's a pinned root devDependency whose
`typescript` peer resolves to the root's `typescript@6` (the last JS release).
Don't run it via `bunx type-coverage@latest` — bunx auto-installs the newest
`typescript` as the peer and it crashes on TS 7 (`ts.SyntaxKind` undefined).
`trigger: { on: [snapshot.taken] }` re-reads the report each snapshot — there
is no "report written" event, so a snapshot trigger (or `collector.sync`
by hand) is how this fact collector runs.
