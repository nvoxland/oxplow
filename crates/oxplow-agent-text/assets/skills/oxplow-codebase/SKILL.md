---
name: oxplow-codebase
description: Answering questions about the project's history and code from oxplow's own data — who changed what and when, which branches exist and how far apart, where a symbol is defined, what a file declares, which files have errors, and the errors the person saw in the app. Loads on v_op_error, v_commit, v_commit_file, v_branch, v_branch_divergence, v_commit_between, v_tag, v_symbol, v_diagnostic, oxplow_code_definition, oxplow_code_references, oxplow_code_symbols, and when the user asks "who changed X", "when did X change", "history of X", "what branches", "where is X defined", "what's in this file", "what calls X", "what's broken", or "can you see that error" / "the red error toast".
---

# The project's history and code

oxplow keeps the repository's history and the code as SQL datasets.
Read them with `oxplow_query_sql` before `git log` or grep. `v_field`
documents each column.

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
  Quick answers have a tool each (`references/history.md`); history past
  the index (any revision, one path, paged) is `oxplow_vcs_log`.
- **`v_branch`** — branches (`name`, `kind`, `head_sha`, `is_default`,
  the `stream_id` that has it checked out, and for a local one its
  `upstream`, `ahead`, `behind`); **`v_tag`** — tags. A `v_stream` row's
  `head_sha` is where its workspace is. Uncommitted files: `v_change_file`
  of the `v_change` with `kind = 'working'` (`staged`, `untracked`).
- **`v_file`** — a folder's entries (`references/code.md`).
- **`v_branch_divergence`**, **`v_commit_between`** — two revisions
  compared, the commits between (`references/history.md`).

Changing history (commit, merge, rebase, push, checkout): the
`oxplow.vcs.*` commands (`list_commands`) or `git` in your terminal. One
commit's worth of change, from the commit page: `oxplow.git.cherry_pick {
stream, rev }` applies a commit the branch doesn't carry, `oxplow.git.revert
{ stream, rev }` undoes one it does.

## Code (`v_symbol`, `v_diagnostic`, `v_lsp_server`, the `code_*` tools)

`references/code.md` (oxplow_get_skill reference `code`) has columns, symbol
kinds, examples and the live-server tools. In short: `v_symbol` is
where something is defined and what a file declares (from oxplow's own
parsing, no language server needed); `v_diagnostic` is what the language
servers report now (`SELECT path, count(*) FROM v_diagnostic WHERE
severity = 'error' GROUP BY path`); `v_lsp_server` says which servers exist
and run; "what calls X" and anything the stored view can't know go to the
`code_*` MCP tools.

## Errors the person saw (`v_op_error`)

When an operation fails in front of the person (a merge, a push, a save)
they get a red toast and a row on the Alerts page. Each is recorded, so
"can you see that error?" is answerable:

- **`v_op_error`** — one row per failure (`at`, `label` — what they were
  doing — `command`, `message`, `exit_code`, `signal`, `duration_ms`,
  `thread`, `stream_id`, `output_size` (the output's length), `seen` and
  `dismissed` (the person opened / cleared it)). The latest:
  `SELECT event_id, at, label, command, message FROM v_op_error ORDER BY
  at DESC LIMIT 5`.
- **The output itself** is `v_op_error_output` (`event_id`, `stderr`,
  `stdout`). A row with `output_size` NULL captured nothing; its
  `message` is all.
- Rows keep 30 days, their output 14; an older row keeps its label, the
  details NULL.
- Commands *you* ran fail back to you (`run_command`); this is the
  person's side.
