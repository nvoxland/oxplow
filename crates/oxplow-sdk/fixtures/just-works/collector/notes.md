# Recorded: a fresh agent builds a collector extension (P7.C6)

Recorded 2026-10-02 with `scripts/record-just-works.sh collector`:
Claude Code 2.1.287, its default model, `--safe-mode` (no CLAUDE.md,
plugins, hooks, memory or MCP), the oxplow-extension skill appended to its
system prompt, and only file tools plus `oxplow extension …` in Bash.

- **Outcome:** success in one run — 64 turns, 3.5 minutes, $2.39.
  `oxplow extension check` and `oxplow extension test` on what it wrote: 0
  errors, 0 warnings (`check.txt`, `test.txt`); `test` ran both of its
  intent examples (a collector over fixture rows, the lens on an empty
  project).
- **What it built** (`produced/stale-work/`): a Starlark collector whose
  `input` query computes each open task's last touch (its edit, comment
  activity, efforts) and whole days idle in SQL; the script keeps those
  idle a week or more. A model over the entity with `not_null`/`unique`
  tests, and a lens over the model with an alert and a Sync action.
- **What the skill carried:** the sandbox has no clock, so the age is
  computed in the `input` query (the skill's Starlark rules); the
  collector example feeds `rows` instead of syncing ("don't sync just to
  make `check` pass"); the empty-project lens example expects `rows: 0`.
- **Not visible here:** `--output-format json` keeps the final message,
  not the transcript, so where it went wrong on the way isn't recorded.
  To see that, rerun with `--output-format stream-json` and read it.

- **Patched after recording (2026-10-07):** the lens format dropped the
  `task` link kind (a work item links as `page`, by its ref), so the
  title column's `link: { kind: task, from: id }` was removed to keep the
  recording checking clean. The run predates the work-item interface:
  it read `v_task`, which is gone, so the collector's `input` was
  rewritten over `v_work_item` (same output columns; the id is the
  number in an oxplow ref). Re-record it with the current skill.
