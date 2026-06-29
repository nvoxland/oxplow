# Code quality: duplication + change-analysis

Native, in-process duplicate-block detection plus the Change-Analysis
function/zone/co-change tooling. Everything runs directly inside the Rust
process via tree-sitter — no subprocess, no Python or Node dependency, nothing
for the user to install.

> **Retired (tsk229):** the persisted **per-function metrics scan** (tool name
> `"metrics"` → `complexity` / `function-length` / `parameter-count` findings),
> the standalone **Code-quality page**, and the `run_code_quality_scan` /
> `list_code_quality_scans` IPC+MCP commands are **gone**. Those signals now live
> in the **metric substrate** as bundled, **language-agnostic** gauges
> (`oxplow.high_complexity_fns`, `oxplow.long_functions`, `oxplow.fn_count` —
> computed via the `code_metrics()` host builtin across all languages, tsk314 —
> see [metrics.md](./metrics.md)). What remains here: the **duplication** scan
> (`"duplication"` tool — no plugin equivalent, so it stays inherent) and the
> live **Change-Analysis** pipeline (`analyze_functions_at_refs`, import deltas,
> co-change), which call `oxplow-code-metrics` directly and never touched the
> scan store. The `code_quality_scan` / `code_quality_finding` tables persist —
> they now hold only `duplicate-block` findings.

## Per-function metrics (Change-Analysis, not a persisted scan)

`oxplow-code-metrics` computes complexity / length / parameter-count /
visibility / container-path per function. These are no longer fanned into a
persisted code-quality scan; they're consumed live by the Change-Analysis
`analyze_functions_at_refs` command (below) and projected into the metric
substrate by the bundled gauges.

`FunctionMetrics.visibility` (`Public`/`Private`/`Unknown`, surfaced
on the IPC as `"public"`/`"private"`/`"unknown"`) is a heuristic
public-or-private classification per language: Rust looks for a
`visibility_modifier` child; TS/JS uses `accessibility_modifier`,
`#`-prefixed names, or the enclosing class/`export_statement` for
top-level functions; Java reads the `modifiers` child; C++ tracks the
preceding `access_specifier` within the enclosing class/struct (class
default = private, struct default = public); Go uses identifier
capitalization; Python uses the leading-underscore convention; C
treats `static` storage class as private. The Change Analysis
Semantic view drives a "Show private" toggle from this field
(default on) and colors the function glyph by visibility.

`FunctionMetrics.container_path` (and `AnalyzedFunction.container_path`
on the IPC surface) carries the outer-to-inner names of the named-
declaration ancestors a function lives inside (class / impl / trait /
mod / namespace / interface / enum / record). The Change Analysis
Functions card uses it to render a `path > container > … > function`
tree so the user can scan high-level constructs first and drill in.
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

`run_duplication_scan` (in `crates/oxplow-app/src/code_quality_runner.rs`)
produces this shape directly. The duplication card surfaces it via the
`list_code_quality_findings` / `run_duplication_scan_at` /
`find_latest_code_quality_scan` IPC.

## Scope: codebase vs diff

Scans run in one of two scopes:

- `codebase` — the runner walks every supported file under the
  project root (skipping `.git`, `target`, `node_modules`, `dist`,
  `build`, and dotdirs).
- `diff` — caller passes a file list (typically from
  `listBranchChanges`); the runner only reads those.

Both scopes are persisted independently per `(stream, tool)`, so
the panel can show "what's complex / duplicated in the whole repo"
and "in just my branch's changes" at the same time without one
overwriting the other.

## `analyze_functions_at_refs` — before/after metrics for Change Analysis

The Change Analysis Dashboard
(`apps/desktop/src/pages/ChangeAnalysisPage.tsx`) needs per-function
metadata at *both* the base and head sides of a diff to bucket
functions into added / deleted / signature-changed / body-changed.

The IPC command `analyze_functions_at_refs`
(`crates/oxplow-tauri-ipc/src/commands/code_quality.rs`) takes a
list of `{ path, base_content, head_content }` specs and calls
`oxplow_code_metrics::analyze_file` directly per side. No tempdir,
no subprocess, no install dependency.

This is **not** persisted — every call re-analyses the provided
contents. It's also **separate from the scan store**: results do
not appear in the Code Quality panel or share scan IDs. Callers
that want persistent rollups should use `runCodeQualityScan`
instead.

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
explainable "edited both ways" signal.

The diff itself is computed inside the IPC via
`similar::TextDiff::from_lines` (no separate `git diff` invocation
needed). Source: `crates/oxplow-tauri-ipc/src/commands/churn.rs`.

## Change Analysis: interestingness scoring

The dashboard's `LookHereFirstCard` ranks files by a CRAP-flavored
multiplicative score so a single hot factor dominates:

```
sizeFactor      = log2(1 + additions + deletions)
complexitySpike = sum(complexityDelta where >0) across this file's modifiedBody
paramSpike      = sum(after-before where >0) across modifiedSignature
longNewFn       = max(0, max(added.length where length>60) - 60) / 40
untestedMul     = hasMatchingTest ? 1.0 : 1.5

base    = 1 + sizeFactor
spike   = (1 + 0.6 * complexitySpike) * (1 + 0.4 * paramSpike) * (1 + longNewFn)
score   = base * spike * untestedMul
```

Each multiplier ≥ 1.2 contributes a hover-readable `reason` —
"complexity +14 across 3 fns", "no test in same dir", etc. All
weights live in `INTERESTINGNESS_WEIGHTS`
(`apps/desktop/src/components/ChangeAnalysis/interestingness.ts`)
so they're tuneable from one place.

Per-function variant `functionInterestingness` uses the same
shape but with churn lines + length on a single function. Used
by `FunctionChurnCard` for tiebreak ordering.

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
- `classify_zone(path) -> Zone` — path-prefix table mapping every
  repo file to one of ~22 architectural zones (`ui`, `shell`, `ipc`,
  `store`, `git`, `lsp`, `runtime`, `analysis`, `test`, `docs`, …).
  Project-meta basenames (Cargo.toml, package.json) and test paths
  override crate-zone classification.
- `zone_for_crate_name(name) -> Option<Zone>` — workspace-crate
  lookup for resolving Rust `use foo::*` to a zone via the synthetic
  path `crates/foo/src/lib.rs`.
- `ZonedImportEdge { edge, from_zone, to_zone }` with
  `is_cross_zone()` — true only when target is in-repo, known, and
  different from the source. `Zone::External` targets never trip
  cross-zone (importing serde isn't a layer violation).

Mirror TS table at
`apps/desktop/src/components/ChangeAnalysis/zones.ts` (kept in sync
by hand) so the UI can badge files without a backend roundtrip. The
Rust table is the source of truth for `ZonedImportEdge` records
crossing the IPC.

### `oxplow-git/co_change`

Walks `git log` (libgit2, time-sorted) within a configurable window
(default 180 days, 5k commit cap), drops mega-commits (>50 files —
mass renames / formatter sweeps drown the signal), builds two maps:

- `co_changers: file → Vec<(co_changer, count)>` filtered to pairs
  with ≥ 3 co-occurrences, sorted descending.
- `last_touched: file → seconds-since-epoch`.

`analyze_surprise(history, commit_files, dormant_days) -> Vec<FileSurprise>`
classifies each file as `Normal | UsualCoChangersAbsent { expected }
| Dormant { last_touched_days }`. Dormancy fires before the
co-changer check (cheaper, clearer signal); files never seen in the
window are treated as dormant. `SurpriseReason` is specta-derived.

The caller is expected to cache `CoChangeHistory` per `(repo,
window)` — the public API is pure once the history is built.

### IPC: `import_deltas` + `analyze_co_change_surprise`

`analyze_functions_at_refs` (the existing per-function metrics
command) now also returns `import_deltas: Vec<ImportDelta>`:

```ts
interface ImportDelta {
  path: string;
  added: ZonedImportEdge[];
  removed: ZonedImportEdge[];
  cross_zone_added: ZonedImportEdge[]; // subset of `added`
}
```

The resolver inside the IPC is intentionally minimal:

- Rust `use crate::*` / `self` / `super` → importer's own zone.
- Rust `use foo::*` → workspace crate lookup; missing → External.
- TS `./foo` / `../foo` → lexical relative-path normalization
  through `classify_zone`.
- Bare specifiers (`react`, `@scope/x`, `node:fs`) → External.
- Everything else → unresolved (`to_zone: null`); cross-zone logic
  ignores it.

Better to underflag than overflag — a missed cross-zone touch is a
quieter UI; a false-positive is a wrong "wrong layer" callout.

New command `analyze_co_change_surprise(file_paths) -> Vec<FileSurprise>`
runs the git-history pipeline above on a `spawn_blocking` worker.
History is rebuilt on every call (sub-second on oxplow-scale repos)
— runtime-level caching is a future optimization.

### UI cards (Change Analysis drilldown)

Three new cards in
`apps/desktop/src/components/ChangeAnalysis/`, inserted at the top
of `ChangeAnalysisDrilldown` above `FilesPanel`:

- **`ZoneBarCard`** — horizontal bar of touched zones sized by churn,
  with cross-zone-added-imports listed below. The headline "wrong
  layer" signal.
- **`ChangeTreemapCard`** — squarified treemap (inline algorithm, no
  d3 dep) sized by churn, coloured by zone. Visual gestalt for "where
  is this commit's mass."
- **`CoChangeSurpriseCard`** — only renders when the backend flags
  something. Lists files with `Dormant` (amber chip + day count) or
  `UsualCoChangersAbsent` (blue chip + top-3 expected co-changers).

Zone badges also render inline in `FileTreeView` rows via the muted
`detail` slot.

## Adding a new code/quality signal

Code/quality signals are now authored as **metrics** (bundled or project
`.oxplow/project.yaml` `metrics:` entries) over the `code_metrics()` / `ast_query()` host
builtins — see [metrics.md](./metrics.md). Duplication is the lone exception:
cross-file token matching has no Starlark equivalent, so it stays an inherent
in-process scan here.

## Performance notes

The duplication runner punts its CPU-bound work to a
`tokio::task::spawn_blocking` pool so it doesn't stall the runtime on large
repos. Rough ballpark on the oxplow checkout (~2k source files): duplicate scan
~2s. Big jumps suggest a tunable (`DupOptions { k, w, min_lines }`) needs
adjusting.
