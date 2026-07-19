# Change Analysis

Change Analysis is the page you go to when you want to
*understand* a diff, not just look at it. Open it against any
pair of refs — a feature branch vs. its parent, two arbitrary
commits, working tree vs. `HEAD` — and it answers the question
**"which files should I look at first, and why?"**

## Opening it

From the launcher (++cmd+p++) → **Change Analysis**, or by
following a Change Analysis link from the Git dashboard / a work
item / a commit page. The page chrome carries Parent vs … /
Refresh / Open commit so you can rebase the comparison without
leaving the tab.

## The dashboard view

The default view (no scope applied) is a single-screen summary
of the whole change:

- **Look here first.** A ranked list of files by *interestingness*
  — a CRAP-flavored multiplicative score that combines churn,
  complexity, tests-missing signal, and duplication. Multiplicative
  on purpose: a single hot factor (e.g. a 200-line churn delta on
  a previously untested file) dominates the ranking, so the top
  three are genuinely the three you should open first.
- **Summary cards.** Files added / modified / deleted, total +/-,
  net complexity delta, duplication added, tests touched.
- **Change treemap.** Cells are scaled by churn and grouped by
  architectural zone (backend / frontend / config / docs / tests
  / build) — a 40-file branch reads as "mostly frontend, one
  config edit" instead of a wall of paths.
- **Import deltas.** Per-file added / removed import edges,
  classified as resolved (target exists in the tree) or
  unresolved (dangling).
- **Co-change surprise.** Files that historically changed
  together but didn't here, and files that changed here but
  historically don't co-change with the rest of the diff. Both
  rank in a "surprise" view so you can spot a missed peer
  before review catches it.
- **Pivots.** Click a row in the file-extension, directory, or
  status pivot tables to drill down into just that slice.

## Drilldown view

Click a pivot value (e.g. `.ts` files, or just the `crates/`
directory, or only the `added` files) and the dashboard
re-renders as a focused drilldown over that slice — same hook,
scope applied. The drilldown carries:

- A **Semantic / File-list view toggle** for switching between a
  function-level and a file-level breakdown.
- A status filter (added / modified / deleted / all).
- Duplication and tests cards relocated from the main dashboard
  so they're closer to the file list.
- The shared header so you can re-pivot without going back.

## Per-function metrics

The Function Churn card uses the IPC command
`analyze_functions_at_refs` to walk *both sides* of the diff and
bucket every function into:

- **Added** — present at head, not at base.
- **Deleted** — present at base, not at head.
- **Signature changed** — same name + same file, different
  arguments or return type.
- **Body changed** — same signature, different cyclomatic
  complexity or line count.

Each row shows the cyclomatic complexity at base and head, so a
function that went from 4 → 19 stands out next to one that went
from 4 → 5. Tiebreaks use a per-function variant of the same
interestingness score.

## Sibling-aware diff opens

Clicking through to the actual unified diff brings the *sibling
page list* with it — every other file in the same change, plus a
jump-to dropdown — so reviewing a 40-file branch isn't a
back-button pilgrimage. The diff page also carries capped
navigation (next / prev) for the current change.

## What feeds the cards

All numbers come from oxplow's in-process code-quality scanners:
complexity and duplication run as Rust tree-sitter scanners
against the worktree, with findings persisted in the project's
SQLite store. The Change Analysis cards read from those findings
directly — no external `lizard` / `jscpd` invocation, no parallel
state to keep in sync.

## When to use it

- **Before a self-review.** Glance at *Look here first* before
  you read your own diff. The score is honest about which file
  is the riskiest part of the change.
- **Before a code review.** Open Change Analysis against the PR's
  base. Read the top three files. Then read the diff with the
  ranking in mind.
- **After an agent effort.** When the agent says it shipped
  something, Change Analysis from before-effort to head shows
  whether the touched surface matches the task's intent.

The pitch is small but specific: the first screen of a review
shouldn't be a flat file list. It should tell you which three
files to look at first and why.
