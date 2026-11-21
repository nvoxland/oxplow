# elixir-pack

A third-party language pack: it teaches oxplow Elixir, a language oxplow ships no pack for. It is the reference for "A language pack" in the oxplow-extension skill.

Its namespace is `acme`, not `oxplow` (that one is for oxplow's own packs). Its language id is `elixir`, with the files `.ex` and `.exs`. A language's id, aliases, file extensions and LSP ids belong to the first extension that claims them; a later claim holds the later extension (`language.conflict`). So a pack can't take a name foundation already declares (foundation holds Lua, JSON, YAML, HTML, CSS, Bash, Ruby and Zig for their language servers).

## What it holds

- `extension.yaml`: the `languages:` entry (`grammar.wasm`, and `grammar.source` saying where the wasm came from), and `test_conventions:` for ExUnit (`*_test.exs` files, `assert` and `refute`, `@tag :skip`).
- `grammars/elixir.wasm`: tree-sitter-elixir v0.3.5, compiled. Its scanner is C.
- `languages/elixir/analysis.yaml`: what code analysis reads. Elixir has no node kind for `def` or `if`: each is a `call` whose target is an identifier. So, as Clojure does, the file uses `forms:` and `form_heads:` (`def`, `defp`, `defmacro` as functions; `if`, `unless`, `with`, `for`, `try` as decisions). Each `->` clause (`stab_clause`) is also a decision.
- `languages/elixir/symbols.scm`: modules, functions (`def` and `defp`, with a body, `do:`, a guard or no arguments), macros, protocols and impls.
- `fixtures/`: the pack's tests. Two are `when: { language: elixir, path, body }` cases (symbols, complexity); one commits a `*_test.exs` file and reads its assertions and skips.

## What the shared form support does not cover

Form support was written for Clojure. For Elixir:

- **parameter count is 0**: it reads a `vec_lit`;
- **visibility is `unknown`**: `form_meta` knows only `defn-` and `^:private` (`defp` is private in Elixir);
- **doc is `none`**: `form_string` reads a `str_lit`, and an Elixir doc is an `@doc` attribute;
- **`and`, `or`, `&&`, `||`** are `binary_operator` nodes with no kind of their own, so they add nothing to complexity;
- **ExUnit `test "name" do` calls** are not functions, so the pack counts their assertions and skips, not the tests.

Moving any of these needs a core change (a new strategy).

## Test it

The pack needs a project that holds it under `.oxplow/extensions/`:

    bun run test:ext elixir-pack

(For a copy kept elsewhere, link the folder at `<root>/.oxplow/extensions/elixir-pack` and add `--project <root>`.)

## Rebuild the grammar

    oxplow extension grammar build examples/extensions/elixir-pack

What the author needs:

- `git` and network, to fetch the grammar at the `rev` of `grammar.source` (cached in `~/.cache/oxplow/grammars`);
- wasi-sdk's `clang`, which the build downloads on first use (or set `WASI_SDK_PATH`);
- the `tree-sitter` CLI, only when the repository doesn't commit `src/parser.c` (tree-sitter-elixir does, so it is not needed here).

To use a new grammar release, change `rev`, rebuild, then fix the node kinds in `analysis.yaml` that the load check refuses.
