# Language support — one cohesive bundle per language

Oxplow's understanding of source code is **language-aware**, and everything
it knows about a language is meant to live in **one cohesive, well-isolated
place** rather than scattered across crates. This doc is the map of that
architecture and the rules for changing it.

## The registry (source of truth for identity-level facets)

`crates/oxplow-code-metrics/src/language.rs` holds a `LanguageSupport` per
language and a `registry()` over them. A `LanguageSupport` answers *"what does
oxplow know about language X?"*:

| Field | Meaning |
|---|---|
| `language` | the canonical, workspace-wide `Language` (the single identity — see below) |
| `display_name` | human-facing name (`"C++"`, `"C#"`) |
| `extensions` | file extensions it claims — **the single source of truth** for `language::language_for_path` |
| `lsp_mason_package` | curated Mason LSP server suggestion, if oxplow ships one |
| `unit_kinds` | the code units `list_units` exposes for it (function / class / module / package); all but package are projections of its extracted definitions |
| `analysis_spec()` | → its `LanguageSpec` node tables in `spec.rs` |

`for_language(lang)` looks up the bundle; `mason_suggestion(language_id)`
resolves an LSP id through the registry (analysis languages) then the
`LSP_ONLY` table (lua/json/yaml/…: ids, Mason package, extensions);
`lsp_extensions(language_id)` answers the files an LSP id covers the same
way — what an installed (Mason) server matches files by, since it has no
configured `extensions` ; `language_for_path(path)` scans the
registry's `extensions`.

## One `Language`, several spec tables (the key design rule)

There is **exactly one** `Language` enum in the workspace —
`oxplow_code_metrics::Language` (`Language`). Every subsystem keys off it. The
per-language *spec tables* are intentionally **not** unified into one struct,
because each subsystem needs genuinely different node-kind knowledge; they
just share the one identity:

| Facet | Where it lives | Why there |
|---|---|---|
| Identity (`Language`, `language_from_name`, `language_from_lsp_id`, `name`, `tag`) | `oxplow-code-metrics/src/spec.rs` | the one source of truth; re-exported widely |
| Registry bundle (extensions, LSP hint, unit kinds, display name) | `oxplow-code-metrics/src/language.rs` | the cross-cutting facets, co-located |
| Static-analysis tables (`LanguageSpec`: decision kinds, parameters, visibility; function-like syntax for the duplication detector) | `oxplow-code-metrics/src/spec.rs` | drives complexity/markers metrics and `oxplow-code-dup` |
| Symbol queries (`symbols.scm`, optional `symbols.yaml`) | `oxplow-code-metrics/languages/<lang>/` | data, not code: what a file defines and how each definition is named (see Symbols) |
| AST-merge tables (`MergeSpec`, `KeyStrategy`) | `oxplow-git/src/ast_merge.rs` (`merge_spec(lang) -> Option<&MergeSpec>`) | merge-specific identity keys; `None` ⇒ language analysed but not merged (e.g. C#) |
| Import/zone extraction | `oxplow-code-deps` | depends on the code-metrics grammar table |
| Idiom metrics (`oxplow.<lang>.*` fact collectors of oxplow-foundation) | `extensions/oxplow-foundation/collectors/<lang>/*.star` | need the Starlark runtime; key off `Language` |
| Editor language id, file icons | `apps/desktop/src/editor-language.ts` (Monaco ids) | renderer-only; Monaco's own id space |
| LSP suggestion mirror | `apps/desktop/src/lspSuggestions.ts` | hand-mirrored from `language::mason_suggestion` |

The rule: **a language's identity flows through the one `Language` enum; each
subsystem keeps its own spec table keyed off it.** New cross-cutting facets
belong on `LanguageSupport`, not in a new per-subsystem map.

## Decision: compiled-in grammars, symbols as data

Grammars are compiled in today: workspace Cargo deps, no dynamic grammar
loading, no user-authored language definitions. That keeps analysis fast and
safe (no native-code loading surface) and each language's behavior
reviewable in-tree.

Symbol extraction is data, not per-language code: a tree-sitter query
(`symbols.scm`) plus an optional `symbols.yaml`, the same form a language
contributed later would bring. Grammars from extensions are a planned
follow-up: tree-sitter grammars compiled to WASM and loaded in a wasmtime
store, shipped with their own `symbols.scm` / `symbols.yaml`. The
data-driven extraction is the path there; nothing in `symbols.rs` names a
language.

### The user-extensibility boundary

Users extend oxplow **without recompiling** along two axes that are *not*
new languages:

- **Metrics** — an extension's `metrics:` + collectors (the project's own:
  `.oxplow/extensions/project/`) + a Starlark/jaq script over the
  `files()`/`ast_query()`/`code_metrics()`/`source_files()`/`markers()`
  host builtins (see `.context/metrics.md`).
- **LSP servers** — `.oxplow/project.yaml` `lsp.servers[]` (any `languageId`), or
  the `oxplow.lsp.install_server` command (see `.context/lsp.md`).

Adding a brand-new language (grammar, complexity, symbols, merge) requires a
recompile today; that's the trade until grammars load from extensions.

## Symbols

A symbol is a definition in a file. Oxplow extracts them itself with
tree-sitter, in every bundled language; the language servers are not
involved (see "Language servers" below).

### The data files

Per language, under `crates/oxplow-code-metrics/languages/<lang>/`:

- `symbols.scm` — a tree-sitter query whose captures mark definitions.
- `symbols.yaml` (optional) — Rust: `trait_container` (how a trait impl's
  container is spelled, `<{type} as {trait}>`); Java, C++, C#: `signature:`
  knobs `name_fields` and `name_kinds` (where a parameter's name sits, so the
  signature keeps only types).

Both are `include_str!`'d. The loader (`SymbolSpec::compile`) refuses a
capture outside this vocabulary and an unknown yaml key.

| capture | meaning |
|---|---|
| `@definition.<kind>` | the definition (its extent; a callable's metrics run on it); `<kind>` is a `SymbolKind` |
| `@name` | its name (several spell one name in source order); a name spelled with `::` (C++ `geo::area`) puts the leading parts in its container |
| `@scope` | a container named outside the nesting (Go's receiver type) |
| `@parameters` | the parameter list whose types make the signature (overloading languages) |
| `@signature` | a node whose text is the signature as written (a Clojure dispatch value) |
| `@container` + `@container.name` (+ `@container.trait`) | a node that only nests definitions (a Rust `impl`) |
| `@_…` | used by predicates only |

Kinds (`SymbolKind`, one vocabulary for every language): function, method,
constructor, class, struct, enum, enum_member, interface, trait, type,
field, constant, variable, module, namespace, macro.

`oxplow_code_metrics::extract(path, source) -> Vec<Definition>`; a
`Definition` is `{ kind, name, container, signature, id, name_line/col,
start/end line/col }`. Per-language tests are under
`crates/oxplow-code-metrics/src/symbols/` (`tests_<lang>.rs`; Rust, Python and Java in `tests.rs`).

### Ids

A definition's id is its containers' names and its own joined by `::`; then
`(param types)` in languages that overload by parameters (Java, C#, C++:
always present on callables, defaults dropped) or a Clojure defmethod
dispatch value, `area(:circle)`; then `~n` for the nth definition with that
same id in the file, in position order. The URI is `oxplow:oxplow/symbol?path=<path>&symbol=<id>`
(resources.md); no revision is part of the identity.

- `crates/oxplow-app/src/lib.rs/Services::boot`
- `src/parse.rs/<Parser as Display>::fmt` (Rust trait impl)
- `src/stack.rs/Stack::push` (generics stripped from container types)
- `src/main/java/com/acme/Parser.java/Parser::parse(String,int)`
- `src/geo.cpp/geo::area(const Shape&)`
- `src/acme/core.clj/area(:circle)` (a Clojure `ns` is not a container)
- `scripts/build.py/main~2`

Anonymous closures and lambdas are not symbols. Function metrics
(`analyze_with_language`) and `list_units` are projections of the same
extraction: metrics exist for callable definitions (function, method,
constructor) only, and a closure's decisions count toward its enclosing
named function. `FunctionMetrics` carries the definition's `id`.

### The index

`v_symbol` / `v_symbol_capture` come from `crates/oxplow-app/src/symbol_collector.rs`,
the `symbols.collect` pump consumer on `snapshot.taken` (after
`search.index`). For each changed file of a supported language it reads the
file as the snapshot captured it (the working tree when snapshots keep no
contents, "Track changes only"), extracts, and restates the file; a deleted
file drops out. There is no server and no budget. A `symbol_capture` row per
snapshot records `files_collected` and `files_failed` (content unreadable at
the snapshot). At boot, `symbol_collector::backfill` fills an empty index
from each live stream's latest snapshot. Columns: `.context/data-model.md`.

### Resolving a ref

`crates/oxplow-app/src/symbol_resolve.rs`, RPC `resolve_symbol { stream_id,
symbol } -> Option<SymbolLocation { path, line, col, revision, versionGone }>`:

- A current ref is its `v_symbol` row.
- A pinned URI (`rev=snap:<id>`, `rev=git:<sha>`) re-extracts the file at that
  revision, reading `path` and the `symbol` id as the URI gives them.
- A pin whose version is gone, or no longer holds the symbol, falls back to
  the current symbol with `versionGone`.

The desktop's `openSymbol` (`App.tsx`) opens a current symbol in the editor at
its line, a pinned one in the file viewer at that revision (no line jump
there), and on `versionGone` shows an error notice and then the current one.
`ref_resolver` titles a pinned ref from its current row.

### Language servers

Live only: definition, references, hover, call hierarchy, document and
workspace symbols (the editor outline and the MCP `code_*` tools). None of
them read `v_symbol`. See `.context/lsp.md`.

## Listing units (the generic structure surface)

"List the functions / classes / modules / packages that make sense for a
language":

- **Tree-sitter (deterministic, offline)** — `oxplow_code_metrics::list_units(path, source)`
  returns `Vec<CodeUnit>` (`kind` ∈ function/class/module/package, `name`,
  `container_path`, line span). It is a projection of `extract`: the
  function-like, class-like and module definitions, with the definition's
  container as `container_path`. The **package** is path-derived and only
  emitted when the language declares `UnitKind::Package` (Go, Java). Exposed
  to the agent as the fallback of the **`oxplow_code_symbols`** MCP tool (a file no
  language server serves is read live, uncommitted edits included, and the
  answer says `source: tree_sitter`); `v_symbol` holds the captured ones. Works
  for the 11 bundled languages.
- **LSP (any server-backed language)** — the `oxplow_code_symbols` (a file, preferred
  over the tree-sitter fallback when a server runs) and `oxplow_code_workspace_symbols`
  MCP tools, and definition, references, hover and call hierarchy, answer for
  *any* configured LSP language, not just the tree-sitter set. See
  `.context/lsp.md`.

These are the substrate the Metrics package/dimension roll-up builds on:
`SqliteMetricStore::dimension_rollup_for_metric` sums a per-file metric's
latest values by **package** (directory) or any per-file `dims_json` key
(e.g. **language**) — the `metric_subject` package grain made concrete. A
slice by dimension is `metric_grid(…, dim)` in SQL; a per-package or
per-language view is page or agent work.

## Adding a language (the checklist)

1. **Grammar dep** — add `tree-sitter-<lang>` to the workspace `Cargo.toml`
   and `crates/oxplow-code-metrics/Cargo.toml`.
2. **Identity** — add a `Language` variant + arms in `spec.rs`
   (`spec()`, `name()`, `tag()`, `language_from_name()`); add LSP aliases to
   `language_from_lsp_id()` if the server uses non-obvious ids.
3. **Analysis spec** — add a `LanguageSpec` static in `spec.rs` (decision /
   parameter node kinds, function-like syntax for the duplication detector,
   a `VisibilityStrategy`).
4. **Symbols** — add `languages/<lang>/symbols.scm` (and `symbols.yaml` if
   the language needs it), register them where the built-ins are
   `include_str!`'d in `symbols.rs`, and add `src/symbols/tests_<lang>.rs`
   covering kinds, containers, ids, repeats and (for an overloading
   language) signatures.
5. **Registry bundle** — add a `LanguageSupport` entry in `language.rs`
   (display name, extensions, LSP suggestion, unit kinds). The
   `registry_has_exactly_one_entry_per_language` test enforces coverage.
6. **Merge (optional)** — if the language should structurally auto-merge, add
   a `MergeSpec` + `merge_spec` arm in `oxplow-git/src/ast_merge.rs`; omit it
   and the merge tier simply skips the language (`merge_spec` ⇒ `None`).
7. **Idiom metrics (optional)** — add `oxplow.<lang>.*` fact-collector scripts under
   `extensions/oxplow-foundation/collectors/<lang>/` and declare them in its `extension.yaml`.
8. **Renderer (optional)** — Monaco id / icon in `editor-language.ts`;
   LSP suggestion mirror in `lspSuggestions.ts`.

## Status

Live: the single `Language` enum, the registry bundle, the LSP symbol surface,
generic unit listing (`list_units`, behind `oxplow_code_symbols`' fallback), and tree-sitter
symbols with their index. **Still open:** exercising `unit_kinds` against the
`metric_subject` roll-up hierarchy so metrics can be grouped by package/module
in the Metrics Explorer; and grammars from extensions (WASM).
