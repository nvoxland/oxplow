# Metrics

A metric is a number oxplow records over time and charts: lines of
code, test duration, clippy warnings, tokens spent on a task. Metrics
are durable and branch-aware, so "did this get better or worse" has an
answer you can look at instead of a feeling.

Two kinds ship out of the box, and they answer different questions.

## Metrics about the code

The familiar kind. Complexity, duplication, file length, lint
suppressions, TODO count, doc coverage, test counts and duration.
These come from in-process tree-sitter scanners and from whatever
your test and coverage tooling already emits -- see
[Collection](#where-the-numbers-come-from) below.

## Metrics about the driving

The less familiar kind. If the agent does the typing and you do the
engineering, the open
question is whether your steering is any good. These measure that:

- **Steering per effort** -- average steering events per closed
  effort (your prompts, Stop-hook nudges, review comments). Lower
  means the agent needed less correction.
- **Efforts per task** -- how many attempts a task took. The
  redo-rate signal.
- **Time to green** -- wall-clock from an effort's first red test run
  to its first green.
- **Tokens per effort** -- the cost of a unit of work.
- **Wasted tokens** -- token spend in efforts whose commits were
  later reverted.
- **Cache hit ratio** -- prompt-side cache reads over total prompt
  tokens.

None of these tell you whether the code is good. They tell you
whether the collaboration is working: which tasks needed six attempts,
where you spent tokens on work you threw away, whether a change to how
you brief the agent actually reduced the back-and-forth.

## The pages

- **Metrics** -- the catalog, and the way in. Every metric oxplow knows
  about, grouped into sections, each with a sparkline and its latest
  value, colored against its target. Filter by name, Enabled / All,
  range, and branch.
- **Metric detail** -- one metric: the chart over time, the recordings
  table, what the metric is (unit, direction, thresholds), an Enabled
  toggle, and Add to dashboard.
- **Dashboards** -- your own arrangements of metric tiles. See
  [Dashboards](dashboards.md).

Open any of them from the launcher (++cmd+p++) -- they're under
**Activity** in the category tree, or just type the name.

## Setting a target

A metric with a target gets a direction (higher-better,
lower-better, or neutral) and a line on its chart, and the Metrics page
colors its value green, amber, or red against it. Targets live in
`.oxplow/project.yaml`, so they're shared with the project rather than
local to you; ask your agent to set one.

Breakdowns (a metric by package, language, or model) aren't on the
detail page. Ask your agent: it can read any metric in SQL, sliced any
way the data allows, and build a [lens](lenses.md) if you want to keep
the view. It can also
slice a metric's history by worktree, thread, effort, task, or commit,
and roll it up by day, week, or month, e.g. "tokens per task per week".

## Writing your own

Ask your agent for what you want counted:

> Track how many functions in the Rust crates are over a complexity of
> 10, by top-level folder, and link me to the worst one in each file.

The agent scaffolds a metric extension and changes it to fit. From a
terminal the same scaffold is:

```
oxplow extension new metric complex-fns --language rust --glob 'src/**/*.rs'
```

That writes `.oxplow/extensions/complex-fns/` with:

- a measure with one fact per file, so a change that touches three files
  rescans three files and keeps the rest;
- a collector that runs after each snapshot and counts the functions over
  a limit (`LIMIT` at the top of the script);
- an `area` dimension, the top folder of each path, so you can split the
  metric by folder;
- the metric itself, lower is better, split by area;
- a lens in the launcher (Code) listing the files, each row linking to the
  worst function's line;
- a fixture: a small set of files and the facts the collector must return.

The agent runs `oxplow extension check` and `oxplow extension test` on it,
so the fixture is what proves the collector is right. Both pass as
scaffolded and fail if an edit breaks the script or makes its facts differ
from the fixture. A real run fails, and says why, when a fact is on a
measure the extension doesn't define or doesn't list for that collector,
or carries a dimension key without the extension's namespace. The script
is Starlark and reads only the snapshot, so an extension's collector
doesn't need your approval.

Files over `snapshotMaxFileBytes` (5 MiB by default) and binary files
aren't given to collectors. A rebuild reports which ones were left out.
The metric reads such a file as clean, so check that list if a number
looks low.

The `/oxplow:new-metric` skill walks the same path with more structure.
A metric that has to run an external program, or parse a report a tool
writes, goes in the project's own extension,
`.oxplow/extensions/project/extension.yaml` (below); the external program
needs your approval first.

## Where the numbers come from

Collection and analysis also extend beyond these built-in examples.
Add sources that describe your architecture or external systems, derive
new entities and measures, and connect them to existing work and code
records. Dimensions let you ask the same question by package, service,
team-relevant category, or any other declared breakdown. The goal is
understanding the software and its development, not only measuring agent
collaboration. See [Lenses and extensions](lenses.md).

A metric extension, including the project's own
(`.oxplow/extensions/project/extension.yaml`), takes four blocks:

- `measures` -- what is being counted, and how it aggregates.
- `collectors` -- the script that produces the raw numbers, and when it
  runs (after each snapshot, when an effort finishes, on a schedule, or
  by hand). Starlark, jq, or an external program. An external program
  from the project's own extension runs only after you approve it in Settings →
  Data → Programs, so pulling a repo can't run something behind your
  back. A changed program needs approving again.
- `metrics` -- a chartable view over a measure: aggregation, filters,
  target, direction.
- `dimensions` -- the axes you can slice by (package, language,
  branch, and any you add).

A metric extension's metrics are on while the extension is enabled, and
your `.oxplow/project.yaml` can still turn one off or change its target
(`use:` and `enabled: false`; it takes no definitions).
An extension's collectors run Starlark or jq only, never an external
program.

A file with a `gauges` block doesn't load: write `collectors` instead.

Test results and coverage arrive separately, through
[collection](#collection) -- oxplow watches the test commands you
run and parses the reports they produce, per effort.

## Metrics over your data

Not every metric needs a scanner. A metric can count or average records
oxplow already keeps, such as tasks, commits and test runs, or data an
extension syncs in. Two ship built in:

- **Tasks completed**: tasks marked done, by the day they finished.
- **Open tasks**: how many tasks are ready, in progress or blocked,
  recorded over time.

Both can be split by priority. Ask your agent for more, for example
"median time from a PR opening to merging, per week" over an extension's
pull requests.

## Collection

Test and coverage numbers are scoped to the **effort** that produced
them, so you can see which tests ran and what the diff coverage was for
that specific piece of work. Configure it once with
`/oxplow:configure`, which wires each test stack in the project to
emit reports oxplow can parse.

An effort's tests, coverage, metric changes, and agent nudges show on
its diff page, from `oxplow-bundled`, the extension that ships with oxplow.

The catch worth knowing: only test runs oxplow can see get recorded.
If your project's configured command is `bun run test:collect`, a
bare `bun test` produces no reports and the effort shows nothing.
