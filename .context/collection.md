# Collection — effort-scoped observations

What this doc covers: oxplow's **collection** subsystem — structured,
provenance-tagged facts attached to a task effort (which tests ran, diff
coverage on the effort's changed lines). The first vertical slice; the same
plumbing is meant to grow to perf deltas, structure maps, etc.

## Why it exists

Everything else oxplow knows is either **computed by oxplow** (snapshots,
blame, code-quality scans) or **free text the agent wrote** (wiki). Test
results and coverage are neither: they're *structured* but
language/framework-specific, so oxplow can't compute them generically. The
bet is to split at the **standard-format seam** — the agent does the
language-specific part (configure the test tool to emit a standard report),
oxplow does the generic part (parse it, attribute it, store it).

## Provenance is the spine

Every observation records whether oxplow **observed** it directly or the
agent **asserted** it. Developers reject self-reported agent numbers, so the
UI must never let an `asserted` figure pass for a measured one. Concretely:
oxplow parses coverage reports itself (→ `observed`); the agent never types
coverage numbers. See [data-model.md](./data-model.md)'s `effort_observation`
section for the column.

## Pieces

- **`effort_observation` store** (`crates/oxplow-db/src/observation_store.rs`,
  migration `V26`). Generic effort-scoped fact: `kind` + `metric_value` +
  `payload_json` + `provenance` + a `page_ref`-style freshness pin. Two
  `kind`s in the slice: `test-run`, `diff-coverage`. Effort-scoped and
  CASCADE-deleted with its effort (an observation is meaningless outside its
  effort's snapshot bracket). Full schema in
  [data-model.md](./data-model.md).
- **`oxplow-coverage` parser** (`crates/oxplow-coverage/`). Deterministic
  report parsing: **cobertura**, **lcov**, and **jacoco-xml** into a uniform
  per-file `{ instrumented, covered }` line-set map (`parse`), plus **JUnit**
  XML into a `TestReport { suites → cases }` tree (`parse_junit`). This is the
  *one place test/coverage numbers originate*. Paths/classnames are verbatim
  from the report; the caller maps paths to repo-relative and the UI builds
  the test tree from `classname`+`name`.
- **Collection profile** (`collection:` block in `oxplow.yaml`, parsed by
  `crates/oxplow-config/src/lib.rs`): `testCommand`, `reports: [{ path,
  format }]` (format ∈ `lcov`/`cobertura`/`jacoco-xml` = coverage, `junit`
  = test results), `testRunPatterns`. The `reports` list is what makes a
  **polyglot repo** work — list every stack's report(s); the ride-along
  parses each that's fresher than the effort start, so each stack lights up
  on its own run. (The pre-`reports` singular fields
  `coverageReportPath`/`coverageFormat`/`testReportPath`/`testReportFormat`
  are still read for back-compat and folded into `reports`.) All optional.
  Edits hot-reload via the config watcher (`ConfigWatcher`, see
  `git-integration.md`), so `/oxplow:configure` takes effect without a
  restart.
- **`/oxplow:configure` command** + **`oxplow-collection` skill** (assets in
  `crates/oxplow-plugin/`). `/configure` does two durable things: instruments
  the project's test tooling to emit a standard-format report at a stable
  path, and records the profile in `oxplow.yaml`. The standing skill keeps
  coverage flowing after configure (run tests before closing a task; never
  type the numbers) so instrumentation doesn't bit-rot.

## Ingestion (hybrid)

Two paths feed the store (see [agent-model.md](./agent-model.md) for the
hook + MCP wiring):

- **Passive** — the PostToolUse Bash hook detects a test run (built-in
  patterns + the profile's `testRunPatterns`) and records a `test-run`
  observation against the open effort. It then walks **every** entry in
  `collection.reports` and ingests the ones fresher than the effort start
  (`merge_fresh_test_reports` / `merge_fresh_coverage` in `collection.rs`):
  JUnit reports merge into one suite/case tree embedded in the `test-run`
  payload (`suites`); coverage reports merge into one `diff-coverage`
  observation over the effort's changed lines. All `observed`, no agent
  step. **Staleness is the router:** a run only regenerates its own stack's
  report(s), so the mtime guard (`report_is_stale`, floor = effort start)
  naturally excludes the other stacks' stale reports — a `bun test` run
  picks up the frontend reports, a `cargo cov` run the Rust ones, and both
  accrue within one effort. The UI builds a tech-natural tree by splitting
  each case's `classname`+`name` on `::`/`.`.
- **Active (MCP)** — `ingest_coverage` is a thin explicit entry point (same
  deterministic parse path) for on-demand or non-standard-location reports.
  It passes `skip_if_stale = false`, so it ingests regardless of mtime — the
  caller explicitly asked for it. `record_test_run` is the one `asserted`
  writer, for richer pass/fail counts the exit code alone can't give.

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
tracks nudged effort ids in an in-memory `HashSet` (not persisted — the
nudge is ephemeral guidance, not durable state). The dedup clears if the
daemon restarts, so the first run of a new session can nudge again.

## Adding a new observation kind

1. Pick a `kind` string and a `payload_json` shape (parsed in TS / by the
   agent — opaque to Rust, so no migration to enrich it).
2. Write it via `SqliteEffortObservationStore::record` with the right
   `provenance` and (where applicable) a freshness pin.
3. Surface it on the effort-review UI.

Prefer `observed` over `asserted` wherever oxplow can compute or parse the
fact itself — that's the difference between an understanding surface and a
dashboard of numbers nobody trusts.
