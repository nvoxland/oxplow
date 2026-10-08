---
name: oxplow-codebase
description: Answering questions about the project's history and code from oxplow's own data — who changed what and when, which branches exist, where a symbol is defined, what a file declares, which files have errors. Loads on v_commit, v_commit_file, v_branch, v_tag, v_symbol, v_diagnostic, code_definition, code_references, code_symbols, and when the user asks "who changed X", "when did X change", "history of X", "what branches", "where is X defined", "what's in this file", "what calls X", or "what's broken".
---

# The project's history and code

oxplow keeps the repository's history and the language servers' view of
the code as SQL models. Read them with `query_sql` before shelling out to
`git log` or grepping: they're indexed, shared with lenses, and cover
every stream. `v_model_column` documents each column.

## History (`v_commit`, `v_commit_file`, `v_branch`, `v_tag`)

- **`v_commit`** — one row per commit (`sha`, `author`, `email`,
  `committed_at`, `subject`, `body`, `parents`), refreshed on boot and
  whenever refs move, for every stream's branch.
- **`v_commit_file`** — the files each commit changed (`sha`, `path`,
  `status`, `additions`, `deletions`). Join it to `v_commit` on `sha`:

  ```sql
  SELECT c.sha, c.subject, c.committed_at FROM v_commit c
  JOIN v_commit_file f ON f.sha = c.sha
  WHERE f.path = 'src/lib.rs' ORDER BY c.committed_at DESC LIMIT 20
  ```

  Who changes a file most: the same join, `GROUP BY c.author`.
- **`v_branch`** — local and remote-tracking branches (`name`, `kind`,
  `head_sha`, `is_default`, and the `stream_id` that has it checked
  out); **`v_tag`** — tags and their commits.

Changing history (commit, merge, rebase, push, checkout) is the
`oxplow.vcs.*` commands (`list_commands` shows them) or `git` in your
terminal.

## Code (`v_symbol`, `v_diagnostic`, the `code_*` tools)

- **`v_symbol`** — the functions, classes, methods … the running language
  servers report for each stream's files (`path`, `name`, `kind`,
  `container`, `line`, `col`), restated at each snapshot. Where
  something is defined: `SELECT path, line, kind, container FROM
  v_symbol WHERE name = 'main'`; what a file declares: `WHERE path = …
  ORDER BY line`. Only files whose language has a running server are
  covered; `v_symbol_capture` says what each snapshot's collection
  covered.
- **`v_diagnostic`** — what the language servers report now (`path`,
  `severity`, `message`, `line`). Which files are broken:
  `SELECT path, count(*) FROM v_diagnostic WHERE severity = 'error'
  GROUP BY path`.
- **Live questions** go to the language server itself through the MCP
  tools: `code_definition`, `code_references`, `code_hover`,
  `code_symbols` (a file), `code_workspace_symbols`,
  `code_call_hierarchy` (who calls / what it calls) and
  `code_diagnostics`. Use them for "what calls X" and anything the
  stored snapshot view can't know.
