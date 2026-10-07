# Code quality: duplication + change-analysis

Native, in-process duplicate-block detection plus the Change-Analysis
function/zone/co-change tooling. Everything runs directly inside the Rust
process via tree-sitter — no subprocess, no Python or Node dependency, nothing
for the user to install.

> **Retired (tsk229):** the persisted **per-function metrics scan** (tool name
> `"metrics"` → `complexity` / `function-length` / `parameter-count` findings),
> the standalone **Code-quality page**, and the `run_code_quality_scan` /
> `list_code_quality_scans` IPC+MCP commands are **gone**. Those signals now live
> in the **metric substrate** as bundled, **language-agnostic** fact collectors
> (`oxplow.high_complexity_fns`, `oxplow.long_functions`, `oxplow.fn_count` —
> computed via the `code_metrics()` host builtin across all languages, tsk314 —
> see [metrics.md](./metrics.md)). What remains here: the **duplication** scan
> (`"duplication"` tool — no plugin equivalent, so it stays inherent) and the
> building blocks of the **change-analysis producer** (per-function metrics,
> churn, import deltas, co-change), which call `oxplow-code-metrics` directly.
> The `code_quality_scan` / `code_quality_finding` tables persist — they hold
> only `duplicate-block` findings, written by the change analyzer's scans.
>
> **Moved (tsk308):** the Change Analysis UI (the `components/ChangeAnalysis/`
> cards, `useChangeAnalysis`, the scope drilldown) and its IPC
> (`analyze_functions_at_refs`, `analyze_co_change_surprise`,
> `run_duplication_scan_at`, `find_latest_code_quality_scan`,
> `list_code_quality_findings`, `read_endpoint_files_content`) are gone.
> `crates/oxplow-app/src/change_analysis.rs` computes and stores each change
> behind `v_change*` ([semantic-layer.md](./semantic-layer.md) → "Change
> analysis"), and the oxplow-bundled `change-review` lens grid shows it in
> the `vcs.commit.details`, `vcs.status.details` and `effort.review.details` slots.

## Per-function metrics (change analysis, not a persisted scan)

`oxplow-code-metrics` computes complexity / length / parameter-count /
visibility / container-path per function. These are no longer fanned into a
persisted code-quality scan; the change analyzer uses them (below) and the
bundled fact collectors project them into the metric substrate.

`FunctionMetrics.visibility` (`Public`/`Private`/`Unknown`, surfaced
on the IPC as `"public"`/`"private"`/`"unknown"`) is a heuristic
public-or-private classification per language: Rust looks for a
`visibility_modifier` child; TS/JS uses `accessibility_modifier`,
`#`-prefixed names, or the enclosing class/`export_statement` for
top-level functions; Java reads the `modifiers` child; C++ tracks the
preceding `access_specifier` within the enclosing class/struct (class
default = private, struct default = public); Go uses identifier
capitalization; Python uses the leading-underscore convention; C
treats `static` storage class as private. It lands in
`v_change_function.visibility`.

`FunctionMetrics.container_path` (and `AnalyzedFunction.container_path`
carries the outer-to-inner names of the named-declaration ancestors a
function lives inside (class / impl / trait / mod / namespace / interface /
enum / record); it's stored `::`-joined as `v_change_function.container`.
Top-level functions report an empty `container_path`. The set of
container kinds is per-language — `LanguageSpec.container_kinds` plus
`container_name_fields` in `crates/oxplow-code-metrics/src/spec.rs`.
Go and C have no class-like containers and use an empty list.

Languages: Rust, TypeScript (incl. TSX), JavaScript, Python, Go,
Java, C, C++. Adding a language is one entry in
`crates/oxplow-code-metrics/src/spec.rs` listing the function /
parameter / decision-point / container AST node names plus a grammar
loader. Files in unsupported languages are silently skipped.

**Duplicate blocks** (tool name `"duplication"`) — handled by
`oxplow-code-dup`. **Function-anchored AST subtree-hash detector
(Deckard-style).** Pipeline:

1. Walk the tree-sitter AST of each file, find every function-like
   node (per `Language::spec().function_kinds` — covers Rust
   `function_item` / `closure_expression`, JS/TS function /
   arrow-function / method, Python / Go / Java / C / C++
   equivalents). **Code outside any function body is not in the
   corpus.** This is deliberate — top-level `const` style objects,
   `enum` declarations with thiserror derives, JSX expression trees,
   schema literals, etc. share AST shape across unrelated files,
   and were the dominant false-positive class of the prior detector.
2. For each function node, hash the function body subtree AND every
   sub-subtree large enough to seed a meaningful match. Hash =
   64-bit fold of preorder-normalized kind sequence: identifiers,
   numeric literals, and strings fold to placeholders (`ID`, `NUM`,
   `STR`); imports / use / include / package declarations are
   skipped whole-subtree; comments are skipped; cross-language
   collisions are prevented by salting with `Language::tag()`.
3. Group records by hash. For each (function-A, function-B) pair
   that shares any matching subtree, emit ONE finding for the
   largest matching subtree between them — so a whole-function
   clone subsumes the inner-loop and inner-branch matches that
   would otherwise pile up.
4. Filter by `min_lines` (default 5) and `min_nodes` (default 30
   AST nodes). The line floor is aggressive on purpose — function-
   anchoring + the node-count floor already filter top-level
   boilerplate and trivial expression subtrees, so the line floor
   doesn't have to do that work too.

Output is two `duplicate-block` findings per pair (one per side)
with `extra.peerPath` / `extra.peerStartLine` / `extra.peerEndLine`
so the panel can render the cross-reference inline.

## Normalized finding shape

```ts
interface CodeQualityFinding {
  path: string;          // repo-relative
  startLine: number;
  endLine: number;
  kind: "duplicate-block";
  metricValue: number;
  extra: Record<string, unknown> | null;
}
```

`run_duplication_scan_scoped` (in
`crates/oxplow-app/src/code_quality_runner.rs`) produces this shape. The
whole tree at the scanned version is the corpus; the scope (a path list)
only decides which files findings are anchored to, so a copy of an
unchanged file is still found.

## Where scans come from

The change analyzer is the one producer: each analyzed change runs a
scan of its head revision (the working tree, a commit or a snapshot,
read through `Trees` — `.context/vcs.md`) scoped to its changed files,
in the background. `DuplicationRecorder::record`
(`crates/oxplow-app/src/duplication_scan.rs`) leaves the record:

- a `code_quality_scan` row (tool `duplication`, scope `change <id>`, the
  `revision` it read — `working`, `snap:<id>`, `git:<rev>` — and a
  path-list fingerprint) and its findings, read through
  `v_code_quality_scan` / `v_code_quality_finding` (the oxplow-bundled
  `findings` and `duplicate-blocks` lenses);
- **stored in one write** (`SqliteCodeQualityStore::finish_scan_with_findings`):
  the findings, their `finding → file` page refs and the scan's `done`
  status commit together, so a scan announces one `ModelsChanged`, not
  two per finding. That burst (`page_ref` feeds `v_knowledge_page` and
  `v_thread_work`) used to keep the webview re-reading the wiki and Work
  models for the whole store and made typing lag;
- **only the latest scan per scope is kept**: the same write deletes the
  older finished scans of its tool and scope (findings cascade; their page
  refs are deleted explicitly). Readers only ever want a scope's latest;
  migration V19 pruned what had piled up before this;
- **only parseable files are read** into the corpus
  (`oxplow_code_metrics::is_supported_path`), not every file in the tree;
- **no facts**: a change scan anchors only its changed files, so a capture
  from it would restate the whole tree from a slice and zero out every
  untouched file's duplicates (tsk365). `oxplow.duplicate_lines` is the
  built-in whole-tree collector's (P7.B5, closes tsk388): the
  `duplicate_blocks(min_lines)` Starlark builtin over the whole tree after
  ref moves (`snapshot.taken`, `trigger: git_refs`) — paced, so a burst
  of them (a commit, a rebase, a restart's catch-up) is one scan of the
  latest tree once they settle for 60 s, at most every 15 min — an empty capture
  clearing the metric after a refactor (tsk44); those restates are history,
  never baselines the dominated-capture prune acts on (tsk709) —
  [metrics.md](./metrics.md) → "built-in";
- a status-bar background task. Its rows' commits announce it
  (`ModelsChanged` on `v_code_quality_scan`); the `CodeQualityScanned`
  bus event is gone (P7.B4).

The change's own `v_change_duplicate` rows are the findings anchored in
its changed files. There is no manual "Scan now" any more.

**Scans are coalesced.**
- `change_analysis::DupQueue` runs one scan at a time across every
  change (a HEAD move or a turn's end take that recorded files analyzes
  the working tree and each open effort together); a newer request for a
  change replaces its queued one, so rapid agent edits don't pile up
  whole-tree parses.
- **An analysis whose inputs haven't moved isn't redone**, forced or
  not. `change.analyzed_from` (V12) records the build, each side's tree
  (a snapshot's `tree_hash`; a working tree's latest snapshot's) and an
  effort's own files; a rerun from the same keeps what's stored, and no
  duplicate scan is queued. A turn's end no longer forces a rerun: its
  take does, and only when it recorded files.
- **One detection at a time per process, whoever asks.** The detector
  (`oxplow_code_dup::detect_duplicates`) holds a process-wide lock while
  it runs, so a change's scan and the whole-tree `oxplow.duplicate_lines`
  collector (which runs in the collector runtime, outside `DupQueue`)
  wait for each other instead of parsing the tree side by side.
- A scan stores its rows only if its analysis generation is still the
  change's latest, and stamps `change.duplicates_events_to` (V13). A
  change analyzed since its last stored scan — one a stop cut off —
  awaits its duplicates (`awaiting_duplicates`); boot queues those
  (`change_analysis::resume_duplicates`), since an analysis whose inputs
  haven't moved isn't redone.
- A failure while storing marks the scan and its task failed rather than
  leaving them "running".
- An effort analyzed while open is recomputed once it closes: the change
  store reports a moved head (working tree → end snapshot) — unless the
  end snapshot holds the tree it was analyzed against.

## Function analysis for a change

`code_analysis::analyze_files` (`crates/oxplow-app/src/code_analysis.rs`)
takes `{ path, base_content, head_content }` specs and calls
`oxplow_code_metrics::analyze_file` per side (no tempdir, no subprocess).
The change analyzer buckets the result into added / deleted /
signature-changed / body-changed functions; a file's review priority is
oxplow-bundled's model `change_interest` over those rows (P7.B5); see
[semantic-layer.md](./semantic-layer.md) → "Change analysis".

The result also carries a `churn: Vec<AnalyzedFileChurn>` rollup
— one entry per file where both `base_content` and
`head_content` were supplied. Each rollup has `file_added` /
`file_deleted` totals and a `functions[]` breakdown attributing
added / deleted / modified line counts to the head-side function
whose `[start_line, end_line]` interval contains each line.
Deletions on the base side map to the corresponding head-side
function via qualified-name match
(`container::container::name`); base-only functions count toward
`file_deleted` but produce no per-function row. `modified_lines`
= `min(added_lines, deleted_lines)` per function — a cheap,
explainable "edited both ways" signal. The diff is
`similar::TextDiff::from_lines` (`crates/oxplow-app/src/churn.rs`).

## Architectural-change overlay: zones, import deltas, co-change surprise

A second axis of analysis sits on top of the function-level metrics:
"what does this change *mean* architecturally?" Three pieces compose
it.

### `oxplow-code-deps`

Tree-sitter-based import extractor + zone classifier. Same nine
languages as `oxplow-code-metrics` (it depends on that crate's
grammar table). Public API:

- `extract_imports(path, source) -> Vec<ImportEdge>` — one
  `ImportEdge { from_path, raw, module, kind, start_line, end_line }`
  per import declaration. Module strings are language-native and
  unresolved (`std::fs`, `./Foo`, `<stdio.h>`, `foo.bar`).
- `diff_edges(before, after) -> (added, removed)` — set diff keyed on
  `(kind, module)`.
- `ZoneRules` — the PROJECT's zone table, compiled from the `zones:`
  block in `.oxplow/project.yaml` (`oxplow_config::ZoneRuleConfig`).
  `classify(path) -> String` takes the first rule whose glob matches;
  no match (and no table at all) yields `ZONE_OTHER`.
- `zone_for_module(name) -> Option<String>` — resolves a Rust `use
  foo::*` to a zone by looking for `foo` (or its `-`-spelled form) as a
  path SEGMENT of some rule's pattern, so `oxplow_db` finds
  `crates/oxplow-db/**`. The one heuristic here; it is what lets the
  project's own table resolve first-party imports without oxplow
  reading Cargo/npm manifests. `None` ⇒ the caller reports
  `ZONE_EXTERNAL`.
- `ZonedImportEdge { edge, from_zone, to_zone }` with
  `is_cross_zone()` — true only when target is in-repo, known, and
  different from the source. `ZONE_EXTERNAL` targets never trip
  cross-zone (importing serde isn't a layer violation).

**Oxplow ships no rule table** (tsk251). What makes a file "the store
layer" follows from how a particular repo is laid out, and oxplow runs
on any repo — the old built-in table hardcoded ~25 `crates/oxplow-*` /
`apps/desktop/*` / `.context/` prefixes, so every other project's files
fell through to `other` and these surfaces were inert. A project with no
`zones:` block classifies everything as `other` and the zone surfaces
stay empty rather than guessing.

Zone labels are free-form strings; `other` and `external` are computed
sentinels and are rejected as declared labels. Config shape, ordering
semantics, and the MCP authoring tools are below.

The UI has its own matcher at
`apps/desktop/src/components/ChangedFiles/zones.ts` (so the changed-files
tree's zone badges need no backend roundtrip); it reads the same rules off
`get_config`.
The two glob implementations are pinned by the shared fixture
`fixtures/zone-globs.json`, which both test suites run — that is what
keeps them from drifting.

### Declaring zones (`zones:` in `.oxplow/project.yaml`)

```yaml
zones:
- match: ["**/Cargo.toml", "**/package.json"]   # one glob or a list
  zone: meta
- match: ["**/*_test.rs", "**/tests/**"]
  zone: test
- match: crates/oxplow-db/**
  zone: store
  color: "#ea580c"        # optional; else a palette entry by first use
- match: "**/*.toml"       # catch-all — deliberately last
  zone: meta
```

**Order is load-bearing: first match wins.** Role-by-filename rules
(tests, docs) go above the package rules so a test file inside a crate
reads as `test`; catch-alls go last. Globs match the full repo-relative
path with `*` stopping at `/` — `**` is the only way to span
directories. Bad globs, empty labels, reserved labels and malformed
colours are config errors at load (`validate_zone_rules`), so a broken
rule is visible rather than silently never matching.

This repo's own table lives in its `.oxplow/project.yaml` and is the
worked example.

### MCP: `list_zones`; writing the table is `config.set`

The agent owns this table. `list_zones` returns the rules plus what they
actually match — a file count per zone over the worktree and a sample of
paths that fell through to `other`, which is the signal that the table
has gone stale as the repo grew. Writing it is the `config.set { key:
"zones", value: [rules] }` command ([commands.md](./commands.md)) — the
WHOLE ordered table, validated by the loader's rules (reserved labels,
glob shapes), written to `.oxplow/project.yaml` (committed, so a team
shares one vocabulary), logged as `config.changed`, undoable. Read side:
`crates/oxplow-app/src/zones_service.rs`.

No IPC of its own — `zones` rides on `get_config`, and the config
watcher hot-reloads file edits, so a write repaints an open
changed-files tree without a restart.

### Co-change (oxplow-bundled, P7.B5)

No longer core. oxplow-bundled's `co_change_pair` model, materialized
over the commit index (`v_commit_file`, every stream's head; see
git-integration.md "Commit indexer"), holds each pair of files that
shared at least 3 commits of 50 files or fewer (mass renames and
formatter sweeps drown the signal) in the last 180 days; its
`change_co_change` model flags a change's files that are dormant (no
commit in 90 days; never touched counts as 90) — checked first — or whose
top three co-changers are all absent from the change. Measured on this
repo's history (2,351 commits, 19k file rows): the pairs compute in
0.14 s ([performance.md](./performance.md)).

### Import deltas

`analyze_files` also returns `import_deltas: Vec<ImportDelta>`
(`added` / `removed` / `cross_zone_added` zoned edges per file), stored as
`v_change_import` rows with `cross_zone` set for new boundary crossings.
The resolver is intentionally minimal:

- Rust `use crate::*` / `self` / `super` → importer's own zone.
- Rust `use foo::*` → `ZoneRules::zone_for_module`; missing → External.
- TS `./foo` / `../foo` → lexical relative-path normalization
  through `ZoneRules::classify`.
- Bare specifiers (`react`, `@scope/x`, `node:fs`) → External.
- Everything else → unresolved (`to_zone: null`); cross-zone logic
  ignores it.

Better to underflag than overflag — a missed cross-zone touch is a
quieter UI; a false-positive is a wrong "wrong layer" callout.

### Where it shows

The oxplow-bundled `change-review` grid (commit, uncommitted and
`effort.review.details` slots): summary, look-here-first, a churn treemap grouped
by zone, function changes, test changes, co-change surprises,
duplication (with compare links) and new cross-zone imports. Zone badges
also render in the core changed-files tree (`ChangedFilesTree`).

## Adding a new code/quality signal

Code/quality signals are now authored as **metrics** (bundled or project
`.oxplow/project.yaml` `metrics:` entries) over the `code_metrics()` / `ast_query()` host
builtins — see [metrics.md](./metrics.md), duplicated lines included (the
`duplicate_blocks` builtin). The change analyzer's scoped scan stays an
in-process scan here: it needs the change's files as its scope.

## Performance notes

The duplication runner punts its CPU-bound work to a
`tokio::task::spawn_blocking` pool so it doesn't stall the runtime on large
repos. Rough ballpark on the oxplow checkout (~2k source files): duplicate scan
~2s. Big jumps suggest a tunable (`DupOptions { k, w, min_lines }`) needs
adjusting.
