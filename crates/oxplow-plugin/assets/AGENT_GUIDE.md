# oxplow agent guide

Reference catalog the agent can read on demand — you shouldn't need to
quote this back, just use the right values when calling oxplow MCP tools.

## Tasks

Every unit of authored change is a `task`. There is **no `kind`
field** — tasks aren't typed as epic/task/subtask/bug/note. The shape
of the work falls out of the data:

- **Epic** — a task that has at least one child (`parent_id` points
  at it). Use `file_epic_with_children` when the change has ≥3
  sub-steps a reviewer would naturally inspect separately; otherwise
  file a single task.
- **Sub-task** — any task whose `parent_id` is set. Use sparingly:
  three siblings under a parent is a coordination win; one is just
  bookkeeping.

Bug/note categorization was intentionally dropped. If you need to
record an observation without queueing execution, use a wiki page or a
work-note attached to a thread.

## Link types (`run_command work_item.link`)

Link two tasks with `run_command` → `work_item.link { ref, target,
link_type }` (refs are `work_item:oxplow:tsk<n>`; the link is made in
your thread). Comment on a task with `work_item.comment { ref, body }`.


- **blocks** — from-item must finish before to-item can start. Use
  this for hard ordering (migration before feature that uses it).
- **discovered_from** — from-item was uncovered while working on
  to-item. Preferred escape hatch for scope creep: file the new
  thing separately, link it back, keep the original scoped.
- **relates_to** — general association with no enforced ordering.
  The catch-all when none of the stronger semantics fit.
- **duplicates** — from-item is the same work as to-item. Close or
  supersede the duplicate after linking.
- **supersedes** — from-item replaces to-item. The older target is
  stale and should not be worked on.
- **replies_to** — from-item is a threaded note/response to
  to-item. Useful for layered conversations about a proposal.
