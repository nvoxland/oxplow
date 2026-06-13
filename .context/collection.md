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

## Pieces

- **`effort_observation` store** (`crates/oxplow-db/src/observation_store.rs`,
  migration `V26`). Generic effort-scoped fact: `kind` + `metric_value` +
  `payload_json` + `provenance` + a `page_ref`-style freshness pin. Three
  `kind`s today: `test-run`, `diff-coverage`, `static-analysis`. Effort-scoped and
  CASCADE-deleted with its effort (an observation is meaningless outside its
  effort's snapshot bracket). Full schema in
  [data-model.md](./data-model.md).
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
- **Collection profile** (`collection:` block in `oxplow.yaml`, parsed by
  `crates/oxplow-config/src/lib.rs`): `testCommand`, `reports: [{ path,
  format }]`, `testRunPatterns`, `analysisRunPatterns`, and `plugins: [...]`
  (project-defined parsers — see below). `format` is no longer gate-kept against a hardcoded
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
  patterns + the profile's `testRunPatterns`) and/or a static-analysis run
  (built-in patterns + `analysisRunPatterns`, via `detect_analysis_run`) and
  records the matching observation(s) against the open effort. It then walks
  **every** entry in `collection.reports` and ingests the ones fresher than
  the effort start (`merge_fresh_test_reports` / `merge_fresh_coverage` /
  `merge_fresh_analysis` in `collection.rs`): JUnit reports merge into one
  suite/case tree embedded in the `test-run` payload (`suites`); coverage
  reports merge into one `diff-coverage` observation over the effort's changed
  lines; analysis reports merge into one `static-analysis` observation
  (findings + per-severity counts). All `observed`, no agent step.
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
- **Active (MCP)** — `ingest_coverage` is a thin explicit entry point (same
  registry parse path) for on-demand or non-standard-location reports.
  It passes `skip_if_stale = false`, so it ingests regardless of mtime — the
  caller explicitly asked for it. `record_test_run` is the one `asserted`
  writer, for richer pass/fail counts the exit code alone can't give.

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
   collector declares a `kind` (`coverage` | `test` | `analysis`) with a fixed
   output schema. The genericity is in this uniform definition mechanism over
   typed kinds, so a future kind (perf, structure-map, …) is a new
   `CollectorKind` plus plugins that target it — not a new subsystem.
   (`analysis` was added exactly this way: a new `CollectorKind`, the
   `AnalysisReport` typed output, and bundled clippy/eslint jaq plugins — no
   new store, IPC, or subsystem.)

**Transform tiers** (trust/preference order): `jaq` (jq, pure Rust — primary,
JSON→JSON reshaping), `starlark` (general/imperative; note: standard Starlark
forbids recursion + `while`, so deep tree-walks are impractical — jaq suits XML
better), `exec` (external process, JSON stdin→stdout — the escape hatch; can do
I/O, so it's tagged lower-trust). In-process tiers run under a `SandboxBudget`
(wall-clock timeout) so a runaway/malformed script is surfaced as an error, not
a hang.

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

**Output schemas** the transform must produce:
- coverage: `{ "files": { "<path>": { "instrumented": [<line>…], "covered": [<line>…] } } }`
- test: `{ "suites": [ { "name", "cases": [ { "classname", "name", "status": "passed|failed|skipped", "timeMs"? } ] } ] }`
- analysis: `{ "findings": [ { "path", "line"?, "column"?, "severity": "error|warning|info|note", "rule"?, "message" } ] }`

The two bundled analysis plugins are the canonical templates: `clippy.jq`
(`input: lines`; `fromjson?` per line tolerates non-JSON lines, keeps
`reason=="compiler-message"`, picks the primary span, maps `level` →
severity) and `eslint.jq` (`input: json`; severity `2`→error / `1`→warning,
null `ruleId` → no rule).

### Authoring a parser plugin

Register it in `oxplow.yaml` — no recompile. The **script lives in its own
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
