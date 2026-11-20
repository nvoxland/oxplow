# oxplow-starlark

What oxplow knows about Starlark (`.star`, `.bzl`, `.bazel`), as a language pack that ships with oxplow and is off by default. Turn it on in a project that has Starlark files (`oxplow.extensions.oxplow-starlark.enabled: true`); oxplow suggests it when it sees them. oxplow extensions write their collectors and commands in Starlark, so this pack lets oxplow analyse them.

- `languages:` in `extension.yaml` declares the language: its file extensions, other names and its grammar (compiled into oxplow). No language server is declared.
- `languages/starlark/analysis.yaml` is what code analysis reads: function and decision node kinds, the parameter list, and how visibility and doc comments are told. It has no import facet: `load(...)` is a call.
- `languages/starlark/symbols.scm` says what a file defines.
