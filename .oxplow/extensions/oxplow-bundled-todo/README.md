# oxplow-bundled-todo

What comes with oxplow, built as an ordinary extension:

- the effort review: what a person checks in an agent's work, shown on
  each effort's diff (the `oxplow/changes.sections` slot);
- the analytics pages: tests and coverage, metric deltas, churn,
  duplication, co-change, token use and page visits;
- advisories that steer the agent toward tests and metric thresholds
  while an effort is open.

## Settings

The review policy's constants are this extension's `configuration:`, with
today's values as the defaults, so a project tunes them without forking:

| Key | Default | Reads |
|---|---|---|
| `largeUncommitted.files` | 15 | files an open effort holds before the agent is told to commit |
| `sharedFiles.windowMinutes` | 60 | how recent an edit is for two threads' open efforts changing the same file to count as overlapping work |

`oxplow.advisory_policy.read` returns the effective values and where each came
from; `oxplow.advisory_policy.write` sets the project's or the person's layer
(a compare-and-swap on the stamps `read` returns). A change takes effect
without a restart: the advisory reads it on its next run.
