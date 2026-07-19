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
> `*-detail` findings); the effort-review panel reconstructs its observation rows
> from there via `CollectionService::effort_observations_from_metrics` (the
> `list_effort_observations` IPC/MCP). The `EffortObservation` type survives only
> as the read/IPC shape. Everything below about the **collector plugins**, the
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
- **Pluggable parsers — `oxplow-collect-plugin`** (`crates/oxplow-collect-plugin/`).
  Report parsing is **not baked in**: a `CollectorRegistry` maps a `format`
  string → a *collector* that turns report text into a **typed output** for its
  kind (coverage = per-file `{ instrumented, covered }` line-sets; test =
  `TestReport { suites → cases }`; analysis = `AnalysisReport { findings }`,
  each finding `{ path, line?, column?, severity, rule?, message }`). The typed
  shapes live in `oxplow-coverage`, which is now just a pure-types crate
  (+ legacy Rust parsers kept only as the golden-test oracle). The six
  first-party parsers (cobertura, lcov, jacoco coverage; junit tests; clippy,
  eslint analysis) ship as **bundled jaq plugins** (`src/plugins/*.jq`)
  registered by `CollectorRegistry::with_builtins()` — same behavior as before,
  just no longer a closed `match`. Projects add formats via `collection.plugins` (below) with
  no change to oxplow. See **Pluggable parsers** below for the model + how to
  author one. Paths/classnames are verbatim from the report; the caller maps
  paths to repo-relative and the UI builds the test tree from `classname`+`name`.
- **Collection profile** (`collection:` block in `.oxplow/project.yaml`, parsed by
  `crates/oxplow-config/src/lib.rs`): `testCommand`, `fastTestCommand`,
  `reports: [{ path,
  format }]`, `testRunPatterns`, `analysisRunPatterns`, and `plugins: [...]`
  (project-defined parsers — see below). `format` is no longer gate-kept against a hardcoded
  list; it's resolved against the collector registry at collection time, so a
  plugin-provided format works and an unknown one is a *warning*, not a config
  error. The `reports` list is what makes a **polyglot repo** work — list every
  stack's report(s); the ride-along parses each that's fresher than the effort
  start, so each stack lights up on its own run.

  **`fastTestCommand` (tsk171)** is the coverage-free counterpart to
  `testCommand`, for the red/green loop. It must still emit a test report, but
  skips instrumentation and should accept a filter. It exists because
  `testCommand` in a coverage-instrumented repo is far too slow to run every
  cycle (here: ~11s for the full suite vs 0.007s for one filtered test), so
  "route every invocation through it" was unfollowable — and an unfollowable
  rule doesn't degrade gracefully, it gets dropped entirely and NONE of the
  red→green runs get recorded. A weaker rule that is followed beats a stricter
  one that isn't.

  Both configured commands are also treated as implicit `testRunPatterns` by
  `on_post_tool_use`, so a fast command whose script name contains no built-in
  pattern (`bun run test:fast`) is still detected as a test run without having
  to be restated. (The pre-`reports` singular
  fields `coverageReportPath`/`coverageFormat`/`testReportPath`/`testReportFormat`
  are still read for back-compat and folded into `reports`.) All optional.
  Edits hot-reload via the config watcher (`ConfigWatcher`, see
  `git-integration.md`), so `/oxplow:configure` takes effect without a
  restart.
- **`/oxplow:configure` command** + **`oxplow-collection` skill** (assets in
  `crates/oxplow-plugin/`). `/configure` does two durable things: instruments
  the project's test tooling to emit a standard-format report at a stable
  path, and records the profile in `.oxplow/project.yaml`. The standing skill keeps
  coverage flowing after configure (run tests before closing a task; never
  type the numbers) so instrumentation doesn't bit-rot.

## Ingestion (hybrid)

Two paths feed the store (see [agent-model.md](./agent-model.md) for the
hook + MCP wiring):

- **Passive** — the PostToolUse Bash hook detects a test run (built-in
  patterns + the profile's `testRunPatterns`) and/or a static-analysis run
  (built-in patterns + `analysisRunPatterns`, via `detect_analysis_run`) and
  records the matching observation(s) against the open effort. It then walks
  **every** entry in `collection.reports` and ingests the ones fresher than
  the effort start (`merge_fresh_test_reports` / `merge_fresh_coverage` /
  `merge_fresh_analysis` in `collection.rs`): JUnit reports merge into one
  suite/case tree embedded in the `test-run` payload (`suites`) — the
  `junit.jq` plugin takes each `<testcase>` from its IMMEDIATE parent
  `<testsuite>`'s direct children (NOT a recursive descent), so a NESTED
  testsuite (bun emits file-suite → describe-suite → testcase) doesn't
  double-count a case under both levels (tsk361); coverage
  reports merge into one `diff-coverage` observation over the effort's changed
  lines; analysis reports merge into one `static-analysis` observation
  (findings + per-severity counts). All `observed`, no agent step.
  **Attribution (tsk347):** the run is pinned to its effort via the `"run"`
  ledger. An agent forces EXACT attribution by prefixing the command with
  `OXPLOW_TASK=<task id>` — `parse_task_token` reads it and `record_test_run`
  claims the run for that task's open effort (`find_open_for_task`), correct even
  under concurrent efforts. Without the token, resolution is, in order:
  **single open effort** → **target overlap** (tsk169: score each open effort by
  what the command names — `-p <crate>`, path args — against the files it has
  claimed, and take a STRICT unique winner) → **unattributed**. Ties and
  whole-suite runs that name nothing decline on purpose: a mis-attributed run is
  worse than an unattributed one, because the agent can still claim the latter at
  close. An unattributed test run with 2+ efforts open fires the
  `unattributed-run` nudge immediately (tsk170) rather than waiting for the
  closing EFFORT REVIEW.

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
  **The recording runs DETACHED from the hook response (tsk62):**
  `bounded_hook_response` drops the handler future at its 5s budget, and a
  test-run's recording can legitimately outlive it (a debug-build junit ingest
  + a multi-MB lcov parse) — run inline, the coverage step after the junit was
  silently cancelled on EVERY run, so `oxplow.coverage` never got a fact. The
  control plane now spawns `on_post_tool_use` on its own task (always
  completes) and waits ≤2.5s for the nudge message; a slow run's nudge is
  still persisted (`persist_nudge`), only the immediate injection is skipped.
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
  gauge-failure convention), queryable in the substrate instead of living
  only in a tty warn.
  **Clippy needs `bun run lint:collect`:** nothing else writes
  `target/clippy.json` (plain `cargo clippy` prints human output), so the
  `oxplow.analysis.*` metrics only populate when clippy runs via the
  `lint:collect` script (JSON to the configured report path; also in
  `analysisRunPatterns` so the `bun run` command string is detected).
  **Staleness is the router:** a run only regenerates its own stack's/tool's
  report(s), so the mtime guard (`report_is_stale`, floor = effort start)
  naturally excludes the other stacks' stale reports — a `bun test` run
  picks up the frontend reports, a `cargo cov` run the Rust ones, a
  `cargo clippy` run the clippy findings, and all accrue within one effort.
  The UI builds a tech-natural tree by splitting each case's
  `classname`+`name` on `::`/`.`. A `static-analysis` observation doubles as
  the analyzer-ran record: when an analyzer is detected but regenerated no
  parseable report, it's stored command-only (no findings, no metric), the
  same way a `test-run` records command-only when no JUnit report is fresh.
- **Active (MCP)** — `ingest_coverage` and `ingest_analysis` are thin
  explicit entry points (same registry parse path) for on-demand or
  non-standard-location reports. Both pass `skip_if_stale = false`, so they
  ingest regardless of mtime — the caller explicitly asked. `ingest_analysis`
  is the on-demand counterpart to `ingest_coverage`: it resolves `format` via
  the registry, parses as `CollectorKind::Analysis`, and records a
  `static-analysis` observation against the open effort via the same private
  `record_static_analysis` the passive ride-along uses (provenance `observed`,
  source `analysis-report` / `plugin-exec:<name>`). `report_path`/`format`
  default to the first analysis report in `collection.reports`; it returns a
  status JSON — `stored` with per-severity counts, or a reason
  (`no_open_effort` / `not_configured` / `report_missing` / `stale_report` /
  `parse_error`). **No baseline gate**: unlike coverage (which intersects with
  the effort's changed lines and therefore *needs* a start snapshot),
  analysis findings are *absolute* — current-file findings, not diff-relative —
  so `ingest_analysis` stores even when the open effort has no start snapshot
  (pin = `None`). This keeps the active MCP path in agreement with the passive
  ride-along, which already records with no baseline (tsk86). It exists because
  analysis had no active
  path — only the passive PostToolUse hook — so the eslint/TS format could
  never be exercised end-to-end in a repo that runs no eslint; the active
  entry closes that symmetry gap (and serves on-demand / odd-location
  reports). `record_test_run` is the one `asserted` writer, for richer
  pass/fail counts the exit code alone can't give; its counts also become
  status-sliced `oxplow.test_case` facts (no case identity) so the
  `oxplow.tests.*` specs read them, and it returns the capture id (the run
  identity `claim_runs` refs use). A report-less, count-less run records its
  capture under the `test-run` producer so it never reads as "found 0 tests"
  (see [metrics.md](./metrics.md)). The run capture also stamps
  **`closest_git_version`/`git_version_exact`** (tsk95) — the commit it tested,
  resolved via `file_ref_version::resolve` off the stream's latest snapshot,
  falling back to HEAD with `exact = false` when the tree is dirty (the normal
  case). This is the fold's only ancestry material and is **not backfillable**;
  see the stamping note in [metrics.md](./metrics.md).

**Observe-always (tsk269/tsk270).** Tests, analysis, **and coverage** are recorded
**regardless of how many efforts are open** — attribution is deferred to the
unified `"run"` ledger, never a precondition for recording. `on_post_tool_use`
resolves a single open effort only for the effort-RELATIVE *advisories*
(commit-hygiene + the report-less / coverage-target nudges), which legitimately
no-op under 0/N efforts; every OBSERVE call runs unconditionally. Report freshness
is gated by a **time-window floor** (`report_fresh_floor`, ~10 min) instead of the
old effort-start floor, so it works with no open effort. **Coverage** is
effort-relative (diff vs the effort's start snapshot), so it can't store the diff
at record: `observe_coverage` stores the **absolute** whole-report coverage
(per-file coverage facts + the instrumented/covered line-sets in the capture's
`coverage-detail` detail envelope — `metric_capture.detail_json`, T-E1; the
legacy `coverage-detail` finding is still dual-written until T-E2), and the
effort-relative diff is DERIVED at read
(`diff_coverage_for_effort`) — so a coverage run claimed *after* the effort closed
still produces a diff. The earlier `find_single_open_for_thread` *drop-gates* on
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
through `record_test_run`, passing `task_id` so the run attributes EXACTLY to
its effort even under concurrency (resolved via `find_open_for_task`); the
`dispatch_task` brief instructs this. Without a named task, a run attributes
automatically when one effort is open, else is left unclaimed for the close
reconcile + window-dominance + the agent's claim — never guessed onto one.
`claim_runs`/`disclaim_runs` on `complete_task`/`update_task` let the agent fix
attribution at the close boundary; `amend_effort` does it after the fact. See
[agent-model.md](./agent-model.md) for the full claim→reconcile loop.

Both paths resolve `format` → collector via the registry and **classify by the
collector's kind** (coverage vs test vs analysis), not a format-name heuristic.
An unknown format is `tracing::warn!`-logged and skipped (not silently dropped).
Trust tier rides in `source`: in-process tiers (jaq/Starlark) are deterministic
and do no I/O → `observed` / `coverage-report` / `analysis-report`; the
external-exec escape hatch can do I/O, so its output is tagged
`plugin-exec:<name>` so the UI can mark it lower-trust. The `provenance` column
stays `observed` vs `asserted`.

The `static-analysis` payload is `{ command?, analyzer?, findings:[…],
errorCount, warningCount, infoCount, noteCount }`; its `metric_value` is the
error+warning count (**lower is better**, unlike coverage where higher is
better). The effort-review UI (`EffortObservations.tsx`) shows a *Static
analysis* section next to *Coverage & tests*: the analyzer label + a high-level
headline (e.g. `clippy: 0 errors, 3 warnings`, green clean / amber
warnings-only / rose on any error) with a findings drill-in grouped by file
(`path:line — rule — message`), each row opening the file. The analysis
ride-along has **no nudge** — the report-less nudge is test-specific.

**The task-page effort section never shows Coverage & tests.** On the task
page's Activity timeline (`TaskDetail.tsx` → `ActivityTimeline`):
- An **in-progress** effort (`ended_at`/`end_snapshot_id` null) renders a
  minimal `ActiveEffortSection` — just an "In progress" header band
  (`tasks-effort-in-progress` testid), no changed-files tree, no summary, no
  fetches.
- A **completed** effort (`ActivityEffortSection`) shows the summary, the
  **Modified Files** tree, and **token usage** (`EffortTokenUsageBlock`) — but
  **not** the `EffortObservationsBlock` (coverage + test-runs + static-analysis).
  That test/coverage/analysis breakdown was deliberately removed from the task
  page to keep it focused on *what changed*; it lives only on the standalone
  effort **diff view** (`DiffViewPage`), which is the effort-review surface.
  `EffortObservationsBlock` itself is unchanged — only the TaskDetail call site
  was dropped.

## Report-less-run nudge (PostToolUse)

When the PostToolUse hook detects a test run but no configured report
was refreshed by it (the agent ran `bun test` instead of the
report-emitting `bun run test:collect`, for example), `on_post_tool_use`
returns a one-shot nudge surfaced to the agent via
`hookSpecificOutput.additionalContext`. The nudge names the project's
own `collection.testCommand` when set, points at the configured `reports`
paths if a profile exists without a `testCommand`, or routes to
`/oxplow:configure` when no profile is present at all.

**Tool-agnostic design:** the hook never encodes tool→command knowledge.
It keys only on (1) "was this a test run?" (substring match against
built-in patterns + `testRunPatterns`) and (2) "did a configured report
get refreshed?" (mtime vs effort start). The tool-specific command it
names comes entirely from the project's config, so it works for any
test tool, current or future.

**Anti-nag:** the nudge fires at most once per effort. `CollectionService`
tracks nudged effort ids in an in-memory `HashSet` (the *dedup* is not
persisted — it clears on daemon restart, so the first run of a new session
can nudge again). The *fired* nudge itself is persisted for review — see
Nudge persistence below.

## Commit-hygiene nudge (PostToolUse)

The same `on_post_tool_use` hook also guards commits. When the Bash
command is a successful `git commit` (`detect_git_commit` — token-aware,
so `git -c user.email=… commit` and `git commit --amend` match, `git add`
/ `git log --grep commit` don't), `check_commit_hygiene` compares the
files in the new HEAD commit against the open effort's **changed set** and
returns a one-shot nudge naming any committed file that falls *outside*
it. Informational only — it never blocks the commit (a legitimate
cross-cutting commit is fine; the agent just gets a conscious heads-up).

This came out of the tsk80 incident: a deliberately-held blog post under
`docs/` was already `git add`ed from a prior session and rode along
silently into a feature commit; committing `docs/` to main auto-deploys
the site via `.github/workflows/docs.yml`, so a push would have published
held content. When any out-of-effort file sits under `docs/`, the nudge
appends a stronger auto-deploy warning.

**"In-effort" is claim-aware** (claim-first attribution, Child 3): when
the effort has CLAIMED files (`task_effort_file` — populated in real time
by the PostToolUse auto-claim and at completion by `touched_files`), the
guard prefers that set. A committed file the effort never claimed is
out-of-effort **even if it changed during the window**, and a claimed file
is never falsely flagged. Only when the effort is UNREVIEWED (no claims at
all — legacy or non-structured-edit efforts) does the guard fall back to
the raw snapshot diff: **"changed set" = start-snapshot vs working-tree**,
the same notion diff-coverage uses (`path_changed_in_effort`, the
path-granularity sibling of `changed_lines_for`): a path is in the changed
set when its working-tree content differs from its effort-start-snapshot
content (absent-side-as-empty, so adds and deletes both count). This
fallback works for an *open* effort, where `list_changed_paths_for_effort`
can't (it needs an end snapshot). HEAD sha + committed file list come from
`oxplow_git::head_commit_sha` / `get_commit_detail` via `spawn_blocking`.

**Skips cleanly** when the commit didn't succeed (non-zero exit), there's
no open effort, or the effort has no start snapshot yet. **Anti-nag:**
once per commit sha, tracked in a second in-memory `HashSet`
(`nudged_commits`) alongside `nudged_efforts`. See also
[git-integration.md](./git-integration.md) (commits are otherwise
user-driven; the Stop hook emits no commit directives).

## Nudge persistence

Both PostToolUse nudges (report-less-run + commit-hygiene) are **persisted**
as well as returned to the agent, so a reviewer can see "what oxplow told the
agent this effort" after the fact — previously the nudge string was forwarded
via `additionalContext` and then lost. When `on_post_tool_use` decides to
return a nudge, it also calls `persist_nudge` (best-effort — a write error is
logged via `tracing::warn!` and swallowed, never failing the hook), which
records a row in the `agent_nudge` table tagged with `kind`
(`report-less-run` / `commit-hygiene`), the message, and the trigger (the
bash command) and emits `AgentNudgesChanged`. Persistence sits **after** the
in-memory dedup gates (`mark_nudged` / `mark_commit_nudged`), so a
deduped/non-fired nudge is never stored. The store
(`SqliteAgentNudgeStore`), IPC (`list_nudges_for_effort` /
`list_nudges_for_thread`), and the collapsed "Agent nudges" task-page
sub-view are covered in [data-model.md](./data-model.md),
[ipc-and-stores.md](./ipc-and-stores.md), and
[agent-model.md](./agent-model.md) (Nudge persistence).

## Adding a new observation kind

1. Pick a `kind` string and a `payload_json` shape (parsed in TS / by the
   agent — opaque to Rust, so no migration to enrich it).
2. Write it via `SqliteEffortObservationStore::record` with the right
   `provenance` and (where applicable) a freshness pin.
3. Surface it on the effort-review UI.

Prefer `observed` over `asserted` wherever oxplow can compute or parse the
fact itself — that's the difference between an understanding surface and a
dashboard of numbers nobody trusts.

## Pluggable parsers (collector plugins)

Report parsing is a **two-layer** design so a new format is config + a small
script, never a Rust change (`crates/oxplow-collect-plugin/`):

1. **Container parse (host-owned).** The host reads the report file(s) and, per
   the collector's declared `input`, normalizes the bytes into a generic JSON
   value via shipped helpers. Scripts never touch the filesystem — that's what
   keeps an in-process parse deterministic and `observed`-eligible.
2. **Field mapping (plugin-owned).** A *collector* maps that value into its
   kind's typed output. There is **never a formless observation** — every
   collector declares a `kind` (`coverage` | `test` | `analysis` | `gauge`) with
   a fixed output schema. The genericity is in this uniform definition mechanism
   over typed kinds, so a future kind (perf, structure-map, …) is a new
   `CollectorKind` plus plugins that target it — not a new subsystem.
   (`analysis` was added exactly this way: a new `CollectorKind`, the
   `AnalysisReport` typed output, and bundled clippy/eslint jaq plugins — no
   new store, IPC, or subsystem. `gauge` — the author-able scalar kind feeding
   the metric substrate — was added the same way; see
   [metrics.md](./metrics.md).)

**Transform tiers** (trust/preference order): `jaq` (jq, pure Rust — primary,
JSON→JSON reshaping), `starlark` (general/imperative; note: standard Starlark
forbids recursion + `while`, so deep tree-walks are impractical — jaq suits XML
better), `exec` (external process, JSON stdin→stdout — the escape hatch; can do
I/O, so it's tagged lower-trust). All three tiers run under a `SandboxBudget`
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
>   `GAUGE_TIMEOUT`) and must be set generously enough that honest ones never trip.
>
> **This same shape of bug has now bitten three times** — a fixed timeout sized
> for a small input silently killing whole-workspace work, and reporting it as
> nothing rather than as a failure:
> 1. **tsk47** — gauges timed out at 5s on every full-tree scan and wrote nothing
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

Two more globals back the **`gauge`** kind (the metric substrate's author-able
capabilities — see [metrics.md](./metrics.md)):
- `ast_query(text, language, sexpr)` → a flat `[{capture, text, start_row,
  start_col, end_row, end_col}]` list. Parses `text` with the named tree-sitter
  grammar (`rust`/`typescript`/`tsx`/`javascript`/`python`/`go`/`java`/`c`/`cpp`/
  `clojure`) and runs the S-expression `sexpr`. Pure (text inline →
  deterministic → `observed`); backed by `oxplow-code-metrics` (`ast`/`parse`/
  `query`). Flat by design so no Starlark recursion is needed.
- `files(glob)` → `[{path, text}]` of the **snapshot** files matching `glob`,
  from an in-memory map the host injects per run via `Evaluator::extra` (a
  `GaugeHost`). Empty when no host is in scope (e.g. a report-derived run) or no
  file matches. The snapshot is content-addressed/immutable → determinism +
  `observed` trust hold. Run a gauge collector with a host via
  `Collector::run_gauge(content, GaugeHost::new(map))`.
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
- gauge: `{ "facts": [ { "measure", "value", "subject"?, "path"?, "line"?, "rule"?, "num"?, "den"?, "dims"? } ], "samples"?: [ { "value", "subject"? ("kind:ref"), "dims"? } ], "findings"?: [ … ] }` (see [metrics.md](./metrics.md)). The primary channel is now **`facts`** — the durable atomic grain of the inverted substrate (epic tsk12): each fact is bound to a defined `measure` (which must be in the gauge's `emits` allow-list) and re-aggregated by a metric *spec* at read time. `num`/`den` are optional ratio components (a `ratio` spec re-derives Σnum/Σden). `rule` populates the fact's `rule` column (the `oxplow.rule` dimension — the per-language idiom gauges tag each `oxplow.ast_hit` fact with the idiom slug there). Emitting a fact on an **undefined** measure — or one outside the gauge's `emits` — is a declare-to-collect violation (the fact is dropped with a warn). `facts` are now the **only** recorded channel (facts-only, T-C3b): `run_one_gauge` writes nothing but facts, and any legacy `samples`/`findings` a script still returns (the per-language idiom scripts, not yet unbaked) are computed-but-ignored.

The two bundled analysis plugins are the canonical templates: `clippy.jq`
(`input: lines`; `fromjson?` per line tolerates non-JSON lines, keeps
`reason=="compiler-message"`, picks the primary span, maps `level` →
severity) and `eslint.jq` (`input: json`; severity `2`→error / `1`→warning,
null `ruleId` → no rule).

### Authoring a parser plugin

Register it in `.oxplow/project.yaml` — no recompile. The **script lives in its own
file** (`entryFile`, project-relative; absolute paths and `..` are rejected),
not inline in the yaml. Example: a Clover (XML) coverage parser in jaq,
claiming the `clover` format that a `reports[]` entry then references:

```yaml
collection:
  reports:
    - { path: target/clover.xml, format: clover }
  plugins:
    - name: acme.clover     # namespaced; "oxplow." is reserved for built-ins
      kind: coverage          # coverage | test | analysis
      formats: [clover]       # format name(s) this plugin claims
      runtime: jaq            # jaq | starlark | exec
      input: xml              # text | json | xml | lcov | lines (jaq/starlark only)
      entryFile: oxplow/plugins/clover.jq
```

```jq
# oxplow/plugins/clover.jq — input value (.) → coverage output schema
{ files: reduce ([.. | select((type=="object") and (.tag=="file"))][]) as $f
    ({}; . + { ($f.attrs.path): {
        instrumented: [ $f | .. | select(.tag?=="line") | (.attrs.num|tonumber?) ],
        covered:      [ $f | .. | select((.tag?=="line") and ((.attrs.count//"0")|tonumber? // 0)>0) | (.attrs.num|tonumber?) ] } }) }
```

`entryFile` resolves relative to the project root; the host reads it (the
script still does no I/O, so determinism holds). For `starlark`, point
`entryFile` at a `.star` file defining `def transform(input): … return {…}`
(the host appends the `json.encode(transform(...))` call; the `json` stdlib
**and** the `parse_xml`/`parse_json`/`lcov_records`/`lines`/`regex_find`/`xpath`
host builtins are available, so a Starlark plugin can self-parse raw
`input: text`). For `exec`, `entryFile` is the program to spawn (executable,
with a shebang); optional `args: [...]`; it gets raw report bytes on stdin and
must print the kind's JSON to stdout.

The first-party parsers in `src/plugins/*.jq` are the canonical templates. New
formats are verified by a golden test that the plugin reproduces the reference
parser's output (`crates/oxplow-collect-plugin/src/lib.rs` tests).

### Report-derived RATIO metrics (a gauge, not the ride-along)

A tool that emits a whole-project **ratio** (not line-sets/findings) — e.g. TS
`type-coverage`'s `--json-output` (`{correctCount, totalCount, percent}`) — is a
**`gauge`** with `compute.report`, NOT a `reports[]` ride-along entry (the
ride-along only classifies the `coverage`/`test`/`analysis` kinds). The gauge
runner reads `compute.report` and feeds it to a jaq/starlark plugin that emits a
`{facts:[{measure, value, num, den}]}` ratio fact. tsk126 dogfoods this as
`repo.type_coverage` (`oxplow/plugins/type_coverage.jq`, `input: text` +
`try fromjson catch null` so a missing report emits nothing rather than failing
the gauge; report at `target/type-coverage.json`, regenerated by the
`type:coverage` package script). `trigger: on-snapshot` re-reads the report each
snapshot — there is no auto-firing `on-report` dispatcher, so `on-snapshot`
(or `manual` via `run_metric`) is how a report gauge runs.
