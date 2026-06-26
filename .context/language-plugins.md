# Language plugins — one cohesive bundle per language

Oxplow's understanding of source code is **language-aware**, and everything
it knows about a language is meant to live in **one cohesive, well-isolated
place** rather than scattered across crates. This doc is the map of that
architecture and the rules for changing it.

## The registry (source of truth for identity-level facets)

`crates/oxplow-code-metrics/src/plugin.rs` holds a `LanguagePlugin` per
language and a `registry()` over them. A `LanguagePlugin` answers *"what does
oxplow know about language X?"*:

| Field | Meaning |
|---|---|
| `language` | the canonical, workspace-wide `Language` (the single identity — see below) |
| `display_name` | human-facing name (`"C++"`, `"C#"`) |
| `extensions` | file extensions it claims — **the single source of truth** for `plugin::language_for_path` |
| `lsp_mason_package` | curated Mason LSP server suggestion, if oxplow ships one |
| `unit_kinds` | the code units it exposes (function / class / module / package) for navigation + metric roll-ups |
| `analysis_spec()` | → its `LanguageSpec` node tables in `spec.rs` |

`for_language(lang)` looks up the bundle; `mason_suggestion(language_id)`
resolves an LSP id through the registry (analysis languages) then a small
LSP-only table (lua/json/yaml/…); `language_for_path(path)` scans the
registry's `extensions`.

## One `Language`, several spec tables (the key design rule)

There is **exactly one** `Language` enum in the workspace —
`oxplow_code_metrics::Language` (tsk321). Every subsystem keys off it. The
per-language *spec tables* are intentionally **not** unified into one struct,
because each subsystem needs genuinely different node-kind knowledge; they
just share the one identity:

| Facet | Where it lives | Why there |
|---|---|---|
| Identity (`Language`, `language_from_name`, `language_from_lsp_id`, `name`, `tag`) | `oxplow-code-metrics/src/spec.rs` | the one source of truth; re-exported widely |
| Registry bundle (extensions, LSP hint, unit kinds, display name) | `oxplow-code-metrics/src/plugin.rs` | the cross-cutting facets, co-located |
| Static-analysis tables (`LanguageSpec`: function/decision/container kinds, visibility) | `oxplow-code-metrics/src/spec.rs` | drives complexity/markers/container metrics |
| AST-merge tables (`MergeSpec`, `KeyStrategy`) | `oxplow-git/src/ast_merge.rs` (`merge_spec(lang) -> Option<&MergeSpec>`) | merge-specific identity keys; `None` ⇒ language analysed but not merged (e.g. C#) |
| Import/zone extraction | `oxplow-code-deps` | depends on the code-metrics grammar table |
| Idiom metrics (`oxplow.<lang>.*` gauges) | `oxplow-collect-plugin/src/plugins/metrics/<lang>/*.star` | need the Starlark runtime; key off `Language` |
| Editor language id, file icons | `apps/desktop/src/editor-language.ts` (Monaco ids) | renderer-only; Monaco's own id space |
| LSP suggestion mirror | `apps/desktop/src/lspSuggestions.ts` | hand-mirrored from `plugin::mason_suggestion` |

The rule: **a language's identity flows through the one `Language` enum; each
subsystem keeps its own spec table keyed off it.** New cross-cutting facets
belong on `LanguagePlugin`, not in a new per-subsystem map.

## Decision: compiled-in Rust + tree-sitter (tsk323)

Language support **ships with oxplow, compiled in, using tree-sitter**. There
is deliberately **no dynamic grammar loading** and **no user-authored
*language* plugins**. The grammars are workspace Cargo deps; the specs are
static tables. This keeps analysis fast, safe (no native-code loading
surface), and each language's behavior reviewable in-tree.

### The user-extensibility boundary

Users extend oxplow **without recompiling** along two axes that are *not*
new languages:

- **Metrics** — `oxplow.yaml` `metrics:` + a Starlark/jaq script over the
  `files()`/`ast_query()`/`code_metrics()`/`source_files()`/`markers()`
  host builtins (see `.context/metrics.md`).
- **LSP servers** — `oxplow.yaml` `lsp.servers[]` (any `languageId`), or
  `lsp_install_server` (see `.context/lsp.md`).

Adding a brand-new language **with static analysis** (complexity, containers,
markers, merge) requires a recompile — that's the deliberate trade.

## Listing units (the generic structure surface)

"List the functions / classes / modules / packages that make sense for a
language" is served two ways, both keyed off the registry's `unit_kinds`:

- **Tree-sitter (deterministic, offline)** — `oxplow_code_metrics::list_units(path, source)`
  returns `Vec<CodeUnit>` (`kind` ∈ function/class/module/package, `name`,
  `container_path`, line span). Functions + class-like containers + modules
  come from the AST (`LanguageSpec`); the **package** is path-derived and only
  emitted when the language declares `UnitKind::Package` (Go, Java). Exposed to
  the agent as the **`list_code_units`** MCP tool. Works for the 11 bundled
  languages.
- **LSP (any server-backed language)** — the `lsp_document_symbols` /
  `lsp_workspace_symbols` MCP tools (tsk324) return the server's symbol tree
  for *any* configured LSP language, not just the tree-sitter set. See
  `.context/lsp.md`.

These are the substrate the Metrics package roll-up builds on: the
`metric_by_package` MCP tool (tsk327, `SqliteMetricStore::package_rollup_for_metric`)
sums a per-file metric's latest values by package (directory) — the
`metric_subject` package grain made concrete. The Metrics **Explorer** UI
group-by-package (a charting-semantics change) is the remaining follow-up.

## Adding a language (the checklist)

1. **Grammar dep** — add `tree-sitter-<lang>` to the workspace `Cargo.toml`
   and `crates/oxplow-code-metrics/Cargo.toml`.
2. **Identity** — add a `Language` variant + arms in `spec.rs`
   (`spec()`, `name()`, `tag()`, `language_from_name()`); add LSP aliases to
   `language_from_lsp_id()` if the server uses non-obvious ids.
3. **Analysis spec** — add a `LanguageSpec` static in `spec.rs` (function /
   decision / container / parameter node kinds + a `VisibilityStrategy`).
4. **Registry bundle** — add a `LanguagePlugin` entry in `plugin.rs`
   (display name, extensions, LSP suggestion, unit kinds). The
   `registry_has_exactly_one_entry_per_language` test enforces coverage.
5. **Merge (optional)** — if the language should structurally auto-merge, add
   a `MergeSpec` + `merge_spec` arm in `oxplow-git/src/ast_merge.rs`; omit it
   and the merge tier simply skips the language (`merge_spec` ⇒ `None`).
6. **Idiom metrics (optional)** — add `oxplow.<lang>.*` gauge scripts under
   `oxplow-collect-plugin/src/plugins/metrics/<lang>/`.
7. **Renderer (optional)** — Monaco id / icon in `editor-language.ts`;
   LSP suggestion mirror in `lspSuggestions.ts`.

## Status

Part of the unified language-plugin epic (tsk320). Live: the single
`Language` enum (tsk321), the registry bundle (tsk322), the LSP symbol surface
(tsk324), and generic unit listing — `list_units` + the `list_code_units` MCP
tool (tsk325). **Still open:** exercising `unit_kinds` against the
`metric_subject` roll-up hierarchy so metrics can be grouped by package/module
in the Metrics Explorer (filed as a follow-up), and LSP `callHierarchy`
(tsk326).
