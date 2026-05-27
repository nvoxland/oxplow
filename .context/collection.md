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
  parsing of **cobertura**, **lcov**, and **jacoco-xml** into a uniform
  per-file `{ instrumented, covered }` line-set map. This is the *one place
  coverage numbers originate*. Paths are report-relative; the caller maps
  them to repo-relative paths.
- **Collection profile** (`collection:` block in `oxplow.yaml`, parsed by
  `crates/oxplow-config/src/lib.rs`): `testCommand`, `coverageReportPath`,
  `coverageFormat`, `testRunPatterns`. All optional — an unconfigured
  project collects nothing extra. `coverageFormat` is validated against the
  parser's known set. Edits to the block are hot-reloaded by the config
  watcher (`ConfigWatcher`, see `git-integration.md`), so `/oxplow:configure`
  takes effect without an app restart.
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
  observation against the open effort; if a `coverageReportPath` is
  configured it reads + parses that report and records a `diff-coverage`
  observation over the effort's changed lines. Both `observed`, no agent
  step. **Staleness guard:** the ride-along only ingests when the report's
  mtime is newer than the effort's start — a run that didn't regenerate the
  report (e.g. `cargo test` when coverage comes from a separate `cargo cov`)
  leaves a report from a prior effort, and attributing it here would be
  misleading (`CoverageIngest::StaleReport`).
- **Active (MCP)** — `ingest_coverage` is a thin explicit entry point (same
  deterministic parse path) for on-demand or non-standard-location reports.
  It passes `skip_if_stale = false`, so it ingests regardless of mtime — the
  caller explicitly asked for it. `record_test_run` is the one `asserted`
  writer, for richer pass/fail counts the exit code alone can't give.

## Adding a new observation kind

1. Pick a `kind` string and a `payload_json` shape (parsed in TS / by the
   agent — opaque to Rust, so no migration to enrich it).
2. Write it via `SqliteEffortObservationStore::record` with the right
   `provenance` and (where applicable) a freshness pin.
3. Surface it on the effort-review UI.

Prefer `observed` over `asserted` wherever oxplow can compute or parse the
fact itself — that's the difference between an understanding surface and a
dashboard of numbers nobody trusts.
