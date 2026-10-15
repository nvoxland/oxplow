# History tools

Quick answers about the repository's history, one tool each:

- `oxplow_file_history { path, limit }` — the commits touching one file.
- `oxplow_recent_commits { limit }` — the latest of the indexed log.
- `oxplow_snapshots { stream?, limit }` — snapshots with parent, trigger and
  whether they went over their time budget (every stream's when none is named).

History past the index (any revision, one path, paged) is `oxplow_vcs_log`;
`v_commit` is for joins.
