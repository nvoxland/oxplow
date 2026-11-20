# Language support — a language is an extension's

Core names no language. Everything oxplow knows about one lives in the
extension that declares it: a `languages:` entry of its manifest and its
data files. oxplow's own are its **language packs**, shipped extensions that
are off by default (`extensions/oxplow-rust`, `oxplow-typescript` (TypeScript,
TSX, JavaScript), `oxplow-python`, `oxplow-go`, `oxplow-java`, `oxplow-c` (C,
C++), `oxplow-csharp`, `oxplow-clojure`); a project turns on the ones it uses
(`oxplow.extensions.oxplow-rust.enabled: true`). Foundation declares the
languages oxplow knows only by their language servers (Lua, JSON, YAML,
HTML, CSS, Bash, Ruby, Zig): no grammar, so nothing parses them.

## A language's declaration

```yaml
languages:
  - id: rust                      # the stored name: symbol.language, the oxplow.language dimension
    title: Rust
    extensions: [rs]
    aliases: ["rs"]               # other names a script may pass (`ast_query(text, "rs", q)`)
    lsp_ids: [typescriptreact]    # LSP languageIds that are this language
    grammar: { native: rust }     # or { wasm: grammars/x.wasm }; leave it out for a language known only by its server
    analysis: languages/rust/analysis.yaml
    symbols: languages/rust/symbols.scm   # + symbols.yaml beside it
    lsp: { package: rust-analyzer }       # the Mason package to suggest
    units: [class, module]                # code units beyond functions
    starter:                              # how the metric scaffold writes a sample function
      function: "fn {name}(x: i32) -> i32 {"
      branch: ["    if x == {n} { return {n}; }"]
      tail: ["    0", "}"]
```

- **Parsed** by `extension/extensions/languages.rs` at load: the files are
  read, the language is built (grammar loaded, every node kind
  `analysis.yaml` names checked against the grammar, the symbol query
  compiled), and a wrong one is the extension's error at its line. An id is
  lowercase letters, digits and `_`.
- **`grammar: { native: <name> }`** is for the extensions that ship with
  oxplow only (`oxplow_code_metrics::native_grammar`, the one place in core
  that names grammar crates). A third party brings a WebAssembly grammar
  (`wasm:`; loading it is not built yet: such a language is refused, saying so).
- **`analysis.yaml`** is the data the walkers read
  (`oxplow_code_metrics::LanguageSpec::parse`):
  `functions { kinds, param_list_fields, parameter_kinds, form_heads }`,
  `decisions { kinds, form_heads }`, `forms { list, symbol, name }` (a grammar
  of generic forms, Clojure's `list_lit`), `visibility { strategy }`,
  `doc { strategy, prefixes }`, and optional facets `imports { strategy }`,
  `duplication { skip_kinds, skip_form_heads }`,
  `merge { key, import_kinds, transparent_kinds, inner_decl_fields, name_fields, def_heads }`.
  A language without a facet lacks it (C# has no merge).
- **Strategies** are behavior data can't say, chosen by name, each named for
  what it looks at: visibility `visibility_modifier | accessibility_modifier
  | modifiers_keyword | access_specifier | capitalized_name |
  underscore_prefix | static_storage | form_meta | unknown`; doc
  `preceding_comment | first_string_statement | form_string | none`; imports
  `rust_use | es_import | python_import | go_import | java_import | c_include
  | cpp_include | clojure_ns | csharp_using` (`oxplow-code-deps`); merge key
  `named | c_declarator | form_head` (`oxplow-git`). A language needing a new
  one is a core change.

## The set

- **`oxplow_code_metrics::Languages`** is the set the enabled extensions
  declare; a `Language` is a cheap handle into it (equal by id). `for_path`
  gives a parseable language, `declared_for_path` any (a server-only one
  too), `by_name` resolves ids and aliases, `by_lsp_id` LSP ids. A
  language's symbol query compiles on first use; `tag()` (FNV-1a of its id)
  salts the duplicate detector's hashes, `fingerprint()` (of its
  declaration and files) keys the script host's parse memo, so a changed
  definition is never served stale.
- **Built** by `extension::language_set::of(extensions)` (foundation, then
  the shipped extensions in their order, then the rest by name; the first to
  claim a name, alias, LSP id or file extension keeps it, the later one is
  left out, its problem logged (it should hold the extension; not yet)) and
  **swapped** at each publication beside the
  model registry (`extension_activation`), into `svc.languages`
  (`LanguagesHandle`). Readers: change analysis, the symbol collector and
  resolver, the duplicate scan, the metrics service and collector runs, the
  LSP session manager (an installed server covers its language's files,
  read when files are matched; the no-server error names the language's
  `lsp.package`), and `GitProvider` (the structural merge).
- **The script worker** gets the set's sources in a `{"languages": …}`
  message before its run and builds its own (`oxplow-script` `ToWorker::Languages`);
  `ast_query`, `code_metrics`, `markers`, `source_files` and
  `duplicate_blocks` parse with it. Collector and fact-collector runs are
  given it (`Sources::languages`, not a permission); command, effect and
  provider scripts are not yet (there a parsing builtin says the language is
  unknown).
- **Every shipped language, on or off**: `language_set::shipped()` /
  `shipped_once()` read them from the embedded files. The metric scaffold
  offers them (online and in the SDK; the extension it writes `requires:`
  its language's pack), and an installed language server covers its
  language's files and keeps its Mason suggestion while the pack is off.
- **Tests** read the checkout's packs:
  `oxplow_code_metrics::for_tests::{shipped, lang}`; an oxplow-app test that
  parses code turns the packs on with
  `test_fixtures::enable_language_packs(dir)` (`services_with_thread` does).
  An extension fixture that parses code names its packs in its own
  `requires:`.

## Symbols

A symbol is a definition in a file. Oxplow extracts them itself with
tree-sitter, in every parseable language of the enabled packs; the language servers are not
involved (see "Language servers" below).

### The data files

Per language, files of the pack that declares it (`languages/<id>/`):

- `symbols.scm` — a tree-sitter query whose captures mark definitions.
- `symbols.yaml` (optional) — Rust: `trait_container` (how a trait impl's
  container is spelled, `<{type} as {trait}>`); Java, C++, C#: `signature:`
  knobs `name_fields` and `name_kinds` (where a parameter's name sits, so the
  signature keeps only types).

The loader (`SymbolSpec::compile`) refuses a
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

`oxplow_code_metrics::extract(languages, path, source) -> Vec<Definition>`; a
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
the snapshot). A deleted file drops out whatever its language.
`symbol_collector::backfill` keeps the index in line with the language set:
the set's fingerprint is kept in `symbol_index_state` (V57), and when the
index is empty or was filled with another set (a pack turned on or off,
while oxplow ran or not) it drops the symbols of languages no longer in
the set and restates every file from each live stream's latest snapshot.
The first publication at boot and each one that changes the set run it
(`extension_activation`). Columns: `.context/data-model.md`.

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

- **Tree-sitter (deterministic, offline)** — `oxplow_code_metrics::list_units(languages, path, source)`
  returns `Vec<CodeUnit>` (`kind` ∈ function/class/module/package, `name`,
  `container_path`, line span). It is a projection of `extract`: the
  function-like, class-like and module definitions, with the definition's
  container as `container_path`. The **package** is path-derived and only
  emitted when the language declares `UnitKind::Package` (Go, Java). Exposed
  to the agent as the fallback of the **`oxplow_code_symbols`** MCP tool (a file no
  language server serves is read live, uncommitted edits included, and the
  answer says `source: tree_sitter`); `v_symbol` holds the captured ones. Works
  for every parseable language of the enabled packs.
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

## Test conventions

What makes a test is data, not core's code: an extension's
`test_conventions:` (`{ id, languages?, paths?, containers?, functions?,
assertions?, skips? }`; `extension/extensions/test_conventions.rs`).
oxplow-foundation declares the rules that hold in any language (`*.test.*`
and `*.spec.*` files, `test/` and `tests/` folders, `Test*` / `*Test` /
`*-test` containers, `test_*` / `Test*` / `Benchmark*` / `Example*`
functions, `assert*` and `expect(` assertions, `.skip` / `.todo` / `xit(`
skips); each pack declares its own (Rust `test_*.rs` and `#[ignore`, Python
`test_*.py`, Go `*_test.go` with `t.Error*`, `t.Fatal*`, `require.<Name>`
and `t.Skip*`, Clojure `*_test.clj[c]`, Java `@Disabled` / `@Ignore`).

- `languages` limits a convention to files of those language ids (a file's
  language is the one `languages:` gives its path); absent, any file. An id
  nobody declares is a warning at the entry's line (the pack may be off).
- `paths` are globs, as `oxplow.testing.testPaths` (`*` stays in a folder,
  `**` crosses). `containers` and `functions` are name patterns matched
  against a whole name: `*`, `?` and `[A-Z]`, so `Test*`'s "followed by an
  uppercase letter, `_` or nothing" is `Test`, `Test_*`, `Test[A-Z]*`.
  `assertions` and `skips` are regular expressions; every match on a code
  line counts once (a span two conventions both match counts once), and line
  comments (`//`, `#` lines but not `#[attribute]`) are left out.
- A bad glob, name pattern or regex is an error at the entry's line.
- **The engine is core's** (`crate::test_conventions`): `TestConventions`
  is built from the enabled extensions at each publication, swapped into
  `svc.test_conventions` beside `svc.languages`, and read by change analysis
  (`TestPaths`, which adds `oxplow.testing.testPaths`); a manifest is never
  read per file. A pack that is off contributes nothing.

## Adding a language (the checklist)

1. **A pack**: an extension with a `languages:` entry (a shipped one in
   `extensions/oxplow-<pack>/`, added to `BUNDLED`, `build.rs` and the debug
   `generated` functions in `bundled_extensions.rs`, `namespace: oxplow`).
2. **Its grammar**: a shipped pack's is a workspace crate named in
   `native_grammar`; anyone else's is a `.wasm` (not loaded yet).
3. **`analysis.yaml`**: the node kinds and strategies above (the load check
   refuses a node kind the grammar doesn't have).
4. **`symbols.scm`** (and `symbols.yaml`): the captures in "Symbols" above.
5. **Its test conventions**: `test_conventions:` (a test-file name, the
   assertion and skip forms), with a fixture in the pack.
6. **Optional**: `starter:` (the metric scaffold's sample), `lsp:`, the merge
   and import facets, its idiom metrics (fact collectors), and the Monaco id /
   icon in `apps/desktop/src/editor-language.ts`.

## Status

Live: languages from extensions, the shipped packs with native grammars, the
LSP symbol surface, generic unit listing, and tree-sitter symbols with their
index. **Still open** (the change-review plan, milestones 2 to 5): the
`v_language` dataset and the pack suggestion, a fixture kind for a
language's analysis with each pack's cases moved into it, a guard that core
names no language, the language set for command, effect and provider
scripts, holding an extension whose language conflicts, one `Parsers` pool,
test conventions /
file kinds / run patterns / report collectors in the packs, and WebAssembly
grammars (Clojure first).
