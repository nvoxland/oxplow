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
- **Pluggable parsers — `oxplow-collect-plugin`** (`crates/oxplow-collect-plugin/`).
  Report parsing is **not baked in**: a `CollectorRegistry` maps a `format`
  string → a *collector* that turns report text into a **typed output** for its
  kind (coverage = per-file `{ instrumented, covered }` line-sets; test =
  `TestReport { suites → cases }`). The typed shapes live in `oxplow-coverage`,
  which is now just a pure-types crate (+ legacy Rust parsers kept only as the
  golden-test oracle). The four first-party parsers (cobertura, lcov, jacoco,
  junit) ship as **bundled jaq plugins** (`src/plugins/*.jq`) registered by
  `CollectorRegistry::with_builtins()` — same behavior as before, just no longer
  a closed `match`. Projects add formats via `collection.plugins` (below) with
  no change to oxplow. See **Pluggable parsers** below for the model + how to
  author one. Paths/classnames are verbatim from the report; the caller maps
  paths to repo-relative and the UI builds the test tree from `classname`+`name`.
- **Collection profile** (`collection:` block in `oxplow.yaml`, parsed by
  `crates/oxplow-config/src/lib.rs`): `testCommand`, `reports: [{ path,
  format }]`, `testRunPatterns`, and `plugins: [...]` (project-defined
  parsers — see below). `format` is no longer gate-kept against a hardcoded
  list; it's resolved against the collector registry at collection time, so a
  plugin-provided format works and an unknown one is a *warning*, not a config
  error. The `reports` list is what makes a **polyglot repo** work — list every
  stack's report(s); the ride-along parses each that's fresher than the effort
  start, so each stack lights up on its own run. (The pre-`reports` singular
  fields `coverageReportPath`/`coverageFormat`/`testReportPath`/`testReportFormat`
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
  registry parse path) for on-demand or non-standard-location reports.
  It passes `skip_if_stale = false`, so it ingests regardless of mtime — the
  caller explicitly asked for it. `record_test_run` is the one `asserted`
  writer, for richer pass/fail counts the exit code alone can't give.

Both paths resolve `format` → collector via the registry and **classify by the
collector's kind** (coverage vs test), not a `== "junit"` heuristic. An
unknown format is `tracing::warn!`-logged and skipped (not silently dropped).
Trust tier rides in `source`: in-process tiers (jaq/Starlark) are deterministic
and do no I/O → `observed` / `coverage-report`; the external-exec escape hatch
can do I/O, so its output is tagged `plugin-exec:<name>` so the UI can mark it
lower-trust. The `provenance` column stays `observed` vs `asserted`.

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

## Pluggable parsers (collector plugins)

Report parsing is a **two-layer** design so a new format is config + a small
script, never a Rust change (`crates/oxplow-collect-plugin/`):

1. **Container parse (host-owned).** The host reads the report file(s) and, per
   the collector's declared `input`, normalizes the bytes into a generic JSON
   value via shipped helpers. Scripts never touch the filesystem — that's what
   keeps an in-process parse deterministic and `observed`-eligible.
2. **Field mapping (plugin-owned).** A *collector* maps that value into its
   kind's typed output. There is **never a formless observation** — every
   collector declares a `kind` (`coverage` | `test`) with a fixed output
   schema. The genericity is in this uniform definition mechanism over typed
   kinds, so a future kind (perf, structure-map, …) is a new `CollectorKind`
   plus plugins that target it — not a new subsystem.

**Transform tiers** (trust/preference order): `jaq` (jq, pure Rust — primary,
JSON→JSON reshaping), `starlark` (general/imperative; note: standard Starlark
forbids recursion + `while`, so deep tree-walks are impractical — jaq suits XML
better), `exec` (external process, JSON stdin→stdout — the escape hatch; can do
I/O, so it's tagged lower-trust). In-process tiers run under a `SandboxBudget`
(wall-clock timeout) so a runaway/malformed script is surfaced as an error, not
a hang.

**Container `input` kinds** (host helpers; all yield a JSON value): `text` (raw
string), `json`, `xml` (explicit ordered tree `{tag, attrs, text?, children}`),
`lcov` (array of records, each key→array), `lines` (array of strings). Also
available to scripts: `regex`, `xpath`. `exec` always receives raw content on
stdin (ignores `input`).

**Output schemas** the transform must produce:
- coverage: `{ "files": { "<path>": { "instrumented": [<line>…], "covered": [<line>…] } } }`
- test: `{ "suites": [ { "name", "cases": [ { "classname", "name", "status": "passed|failed|skipped", "timeMs"? } ] } ] }`

### Authoring a parser plugin

Add it to `oxplow.yaml` — no recompile. Example: a Clover (XML) coverage parser
in jaq, claiming the `clover` format that a `reports[]` entry then references:

```yaml
collection:
  reports:
    - { path: target/clover.xml, format: clover }
  plugins:
    - name: clover
      kind: coverage          # coverage | test
      formats: [clover]       # format name(s) this plugin claims
      runtime: jaq            # jaq | starlark | exec
      input: xml              # text | json | xml | lcov | lines (jaq/starlark only)
      entry: |                # jaq program: input value (.) → output schema
        { files: reduce ([.. | select((type=="object") and (.tag=="file"))][]) as $f
            ({}; . + { ($f.attrs.path): {
                instrumented: [ $f | .. | select(.tag?=="line") | (.attrs.num|tonumber) ],
                covered:      [ $f | .. | select((.tag?=="line") and ((.attrs.count//"0")|tonumber)>0) | (.attrs.num|tonumber) ] } }) }
```

For `starlark`, set `runtime: starlark` and write `def transform(input): …
return {…}` (the host appends the `json.encode(transform(...))` call; the
`json` stdlib is available). For `exec`, set `runtime: exec`, `entry: <program>`,
optional `args: [...]`; it gets raw report bytes on stdin and must print the
kind's JSON to stdout.

The first-party parsers in `src/plugins/*.jq` are the canonical templates. New
formats are verified by a golden test that the plugin reproduces the reference
parser's output (`crates/oxplow-collect-plugin/src/lib.rs` tests).
