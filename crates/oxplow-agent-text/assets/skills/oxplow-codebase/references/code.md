# Code: symbols, diagnostics, language servers

- **`v_symbol`** — what each stream's files define, as oxplow's own
  tree-sitter parsing reads them, in every supported language (`ref`, `path`,
  `name`, `kind`, `container`, `signature`, `line`, `col`), restated at each
  snapshot. `kind` is one of function, method, constructor, class, struct,
  enum, enum_member, interface, trait, type, field, constant, variable,
  module, namespace, macro. `signature` holds the parameter types in
  languages that overload by them (Java, C#, C++), else NULL. Where
  something is defined: `SELECT path, line, kind, container FROM
  v_symbol WHERE name = 'main'`; what a file declares: `WHERE path = …
  ORDER BY line`. `ref` is `symbol:<path>/<id>` (no revision); no language
  server is needed. `v_symbol_capture` says what each snapshot's collection
  covered (`files_collected`, `files_failed`).
- **`v_diagnostic`** — what the language servers report now (`path`,
  `severity`, `message`, `line`). Which files are broken:
  `SELECT path, count(*) FROM v_diagnostic WHERE severity = 'error'
  GROUP BY path`.
- **`v_lsp_server`** — which language servers exist (`language_id`, `command`,
  `source`: `yaml` or `installed`), whether the binary is on this machine
  (`binary_exists`) and how many streams run it (`running`). Read it before
  relying on a language's `code_*` tools, and after `oxplow.lsp.install_server`.
- **Live questions** go to the language server itself through the MCP
  tools: `oxplow_code_definition`, `oxplow_code_references`, `oxplow_code_hover`,
  `oxplow_code_call_hierarchy` (who calls / what it calls). Use them for "what calls X" and anything the
  stored snapshot view can't know. `oxplow_code_symbols { path }` lists a file's symbols from its server
  and, when none runs, from oxplow's own tree-sitter reading of the file as it is now (it says
  which: `source`); `oxplow_code_workspace_symbols { language, query }` finds symbols by name across the
  workspace and needs a running server (`v_lsp_server`). `v_symbol` holds the stored symbols of
  files a snapshot captured (not a new uncommitted file).
