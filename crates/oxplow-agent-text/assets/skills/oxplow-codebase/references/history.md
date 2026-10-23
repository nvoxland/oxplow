# History tools

Quick answers about the repository's history, one tool each:

- `oxplow_file_history { path, limit }` — the commits touching one file.
- `oxplow_recent_commits { limit }` — the latest of the indexed log.
- `oxplow_snapshots { stream?, limit }` — snapshots with parent, trigger and
  whether they went over their time budget (every stream's when none is named).

History past the index (any revision, one path, paged) is `oxplow_vcs_log`;
`v_commit` is for joins.

One commit past the index needs no tool: a query that names it by its full
sha (`v_commit` or `v_commit_file` `WHERE sha = '<sha>'`) has it indexed
first.

## Two revisions compared

Both models are read from the repository when a query names them, and
narrowed by what it pins: `WHERE base = 'main' AND head = 'feature'` is
one pair; pin one side and the other ranges over every branch (local and
remote-tracking, `origin/x`); pin neither and each local branch is
compared with the default branch. A side is any revision git resolves
(`origin/main`, a sha, `HEAD`); one it can't gives no row.

- `v_branch_divergence` — `ahead`, `behind`, `merge_base`,
  `overlapping_files` (JSON array of files changed on both sides) and
  `readiness` (`already_integrated`, `clean`, `conflict`): how far `head`
  has moved from `base`, and whether it would merge cleanly. A branch
  against its own upstream is `v_branch.ahead` / `behind`.
- `v_commit_between` — the commits on `head` that `base` lacks (`sha`,
  `short_sha`, `author`, `email`, `committed_at`, `subject`), newest
  first, 100 per pair at most: what is unpushed or unmerged. Whole
  histories are `v_commit`.
