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
  **Attribution (tsk347):** the run is pinned to its effort via the `"run"`
  ledger. An agent forces EXACT attribution by prefixing the command with
  `OXPLOW_TASK=<task id>` — `parse_task_token` reads it and `record_test_run`
  claims the run for that task's open effort (`find_open_for_task`), correct even
  under concurrent efforts; without it the single-open auto rule applies.
  **Detection is run-aware (tsk347):** `detect_test_run`/`detect_analysis_run`
  split the command on shell operators (`&&`/`||`/`;`/`|`) and ignore
  sub-commands whose leading executable only *reads* (grep/echo/cat/sed/…), so a
  command that merely MENTIONS a pattern (`grep test:collect oxplow.yaml`) is no
  longer a phantom run (and fires no report-less nudge); leading `VAR=val` env
  assignments are skipped so the `OXPLOW_TASK=` prefix doesn't mask the real
  exec. **Background caveat:** the PostToolUse hook fires when the Bash call
  *returns*; a **backgrounded** `test:collect` returns at launch (before its
  reports regenerate), so nothing fresh is ingested — run it in the FOREGROUND.
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
  pass/fail counts the exit code alone can't give.

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
(`oxplow.coverage.abs_pct` + per-file instrumented/covered line-sets in the
`coverage-detail` finding), and the effort-relative diff is DERIVED at read
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
  parameter_count, start_line, end_line, visibility}]` via `oxplow-code-metrics`.
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
- coverage: `{ "files": { "<path>": { "instrumented": [<line>…], "covered": [<line>…] } } }`
- test: `{ "suites": [ { "name", "cases": [ { "classname", "name", "status": "passed|failed|skipped", "timeMs"? } ] } ] }`
- analysis: `{ "findings": [ { "path", "line"?, "column"?, "severity": "error|warning|info|note", "rule"?, "message" } ] }`
- gauge: `{ "samples": [ { "value", "subject"? ("kind:ref"), "dims"? } ] }` (→ `metric_sample`; see [metrics.md](./metrics.md)). A gauge may emit **many** samples with different subjects — the bundled code gauges return a `tree:.` repo total **plus** a sparse `file:<path>` sample per nonzero file, which is the per-file *attribution grain* the effort roll-up reads ([metrics.md](./metrics.md)).

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
