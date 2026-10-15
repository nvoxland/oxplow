---
name: oxplow-codebase
description: Answering questions about the project's history and code from oxplow's own data — who changed what and when, which branches exist, where a symbol is defined, what a file declares, which files have errors, and the errors the person saw in the app. Loads on v_op_error, v_commit, v_commit_file, v_branch, v_tag, v_symbol, v_diagnostic, oxplow_code_definition, oxplow_code_references, oxplow_code_symbols, and when the user asks "who changed X", "when did X change", "history of X", "what branches", "where is X defined", "what's in this file", "what calls X", "what's broken", or "can you see that error" / "the red error toast".
---

# The project's history and code

oxplow keeps the repository's history and the language servers' view of
the code as SQL models. Read them with `oxplow_query_sql` before shelling out to
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

  Who changes a file most: the same join, `GROUP BY c.author`. Who last
  changed each line: the `oxplow_vcs_blame { path, revision? }` tool (`working`,
  `snap:<id>` or `git:<rev>`; an uncommitted line names no commit).
  History past the index (any revision, one path, paged) is the
  `oxplow.vcs.log` command; `v_commit` is for joins.
- **`v_branch`** — local and remote-tracking branches (`name`, `kind`,
  `head_sha`, `is_default`, and the `stream_id` that has it checked
  out); **`v_tag`** — tags and their commits.

Changing history (commit, merge, rebase, push, checkout) is the
`oxplow.vcs.*` commands (`list_commands` shows them) or `git` in your
terminal.

## Code (`v_symbol`, `v_diagnostic`, `v_lsp_server`, the `code_*` tools)

`references/code.md` (oxplow_get_skill reference `code`) has the columns, symbol
kinds, example queries and the live-server tools. In short: `v_symbol` is
where something is defined and what a file declares (from oxplow's own
parsing, no language server needed); `v_diagnostic` is what the language
servers report now (`SELECT path, count(*) FROM v_diagnostic WHERE
severity = 'error' GROUP BY path`); `v_lsp_server` says which servers exist
and run; "what calls X" and anything the stored view can't know go to the
`code_*` MCP tools.

## Errors the person saw (`v_op_error`)

When an operation fails in front of the person — a merge, a push, a save, a
query in the app — they get a red toast and an entry in the Op Errors list.
Each one is also recorded, so "can you see that error?" is answerable
without a screenshot:

- **`v_op_error`** — one row per failure (`at`, `label` — what they were
  doing — `command`, `message`, `exit_code`, `signal`, `duration_ms`,
  `thread`, `stream_id`, and `output_size`, the length of the captured
  stderr / stdout), newest last. The latest few:
  `SELECT event_id, at, label, command, message FROM v_op_error ORDER BY
  at DESC LIMIT 5`.
- **The output itself** is the event's `output` body: call
  `oxplow_read` with `event:<event_id>#output`. A row with
  `output_size` NULL had nothing captured; its `message` is all there is.
- Rows are kept 30 days and their output 14; an older row keeps its label
  with the details NULL.
- Failed *commands you ran* come back to you as the command's own error
  (`run_command`); this is the person's side.
