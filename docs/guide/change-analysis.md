# Change Analysis

Change Analysis answers "which files in this change should I look at
first, and why?" It shows up on three pages:

- a **commit** page (the commit vs. its first parent),
- **Uncommitted Changes** (the working tree vs. `HEAD`),
- a **request's diff** (what an agent request changed, start snapshot to
  end, or to the working tree while it's still running).

Each of those pages shows the change's files. Below that sits a one-line
**Review** brief with a link to the **Review** page. oxplow-bundled
provides both (see [Pages](pages.md)).

## What's in it

The Review page lists what the change was asked to do, then:

- **Look Here.** The files with a high or medium signal, worst first,
  then by how much each changed. Open a row to see each signal and its
  line.
- **By Area.** How many signals each area gave (tests, verification,
  dependencies, code, scope, history), with links to deeper pages.
- **Signals.** Every signal of the change in one table. Narrow it by
  area, severity or source.
- **Tests.** What the change did to its tests, and what checked it.
  Read it to tell a weakened test from one that was only moved.
- **Code.** Which functions grew, which public API moved and which
  blocks were copied from elsewhere.

There are no architectural zones. A project that wants its own layers
can add them as a dataset in its own extension.

Every file and function links to its diff between the change's two
sides.

## Where the numbers come from

oxplow analyzes the change once and stores the result: tree-sitter
metrics on both sides of every changed file, line churn per function,
import changes, co-change history from `git log`, and a duplicate-block
scan. Commits and finished requests are computed once. The working tree
and running requests recompute when files change.

The results live in the `v_change*` views, so you and your agent can
query them directly (`oxplow_ensure_change`, then `oxplow_query_sql`), or build your
own page on them.

## When to use it

- **Before a self-review.** Read Look Here First before you read your
  own diff.
- **Before a code review.** Open the commit, read the top three files,
  then read the rest with that ranking in mind.
- **After an agent request.** The request's diff shows whether the
  touched surface matches what the task asked for.
