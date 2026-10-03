# Change Analysis

Change Analysis answers "which files in this change should I look at
first, and why?" It shows up on three pages:

- a **commit** page (the commit vs. its first parent),
- **Uncommitted Changes** (the working tree vs. `HEAD`),
- an **effort's diff** (what an agent effort changed, start snapshot to
  end, or to the working tree while it's still open).

Each of those pages shows the change's files. Below that sits the
**Change Analysis** section, a grid of lenses from the bundled
`oxplow-analytics` extension. Turn the extension off and the pages
keep just their file lists (see [Lenses](lenses.md)).

## What's in it

- **Summary.** Files added / modified / deleted, total +/-, how many
  test files changed, and the test-to-code line ratio.
- **Look Here First.** The files ranked by a review-priority score,
  each with its reasons ("complexity +14 across 3 fns", "212 lines
  touched"). The score multiplies its factors, so one hot signal
  (a long new function, a complexity spike) puts a file at the top.
- **Churn.** A treemap of changed lines per file, grouped by
  architectural zone. A 40-file branch reads as "mostly the store
  layer, one config edit" instead of a wall of paths. Zones come from
  the `zones:` block in `.oxplow/project.yaml`; ask your agent to set
  them up (it sets them with the `config.set` command on `zones`).
- **Function Changes.** Functions added, deleted, or modified outside
  tests, with the complexity and length deltas and how many lines
  changed inside each. Click one to open the diff at that function.
- **Test Changes.** The test files the change touched.
- **Co-change Surprises.** Files that usually change together with
  others that didn't change here, and files that had been untouched
  for a long time. Both are worth a second look before review.
- **Duplication.** Blocks in the changed files that duplicate code
  elsewhere. Click one to see the two copies side by side. This scan
  runs in the background, so it can show up a few seconds after the
  rest.
- **New Cross-zone Imports.** Imports the change added from one zone
  into another -- the "is this reaching into the wrong layer?" check.

Every file and function links to its diff between the change's two
sides.

## Where the numbers come from

oxplow analyzes the change once and stores the result: tree-sitter
metrics on both sides of every changed file, line churn per function,
import changes, co-change history from `git log`, and a duplicate-block
scan. Commits and closed efforts are computed once. The working tree
and open efforts recompute when files change.

The results live in the `v_change*` views, so you and your agent can
query them directly (`ensure_change`, then `query_sql`), or build your
own lens on them.

## When to use it

- **Before a self-review.** Read Look Here First before you read your
  own diff.
- **Before a code review.** Open the commit, read the top three files,
  then read the rest with that ranking in mind.
- **After an agent effort.** The effort's diff shows whether the
  touched surface matches what the task asked for.
