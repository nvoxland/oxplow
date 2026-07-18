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

The less familiar kind, and the reason metrics are in oxplow at all.
If the agent does the typing and you do the engineering, the open
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

- **Metrics** -- the explorer. Every metric with an active spec,
  charted, filterable by branch and date range.
- **Recorded Metrics** -- the catalog. Every metric oxplow knows
  about, grouped into sections, with an Enabled / All / Off-target
  filter. This is where you turn metrics on and set targets.
- **Metric detail** -- one metric: the chart, its breakdown by
  package / language / branch, the recordings table, and its
  settings.
- **Dashboards** -- your own arrangements of metric tiles. See
  [Dashboards](dashboards.md).

Open any of them from the launcher (++cmd+p++) -- they're under
**Activity** in the category tree, or just type the name.

## Setting a target

A metric with a target gets a direction (higher-better,
lower-better, or neutral) and a line on its chart. The Recorded
Metrics page can then filter to **Off target**, which is the short
list worth looking at. Targets live in `.oxplow/project.yaml`, so
they're shared with the project rather than local to you.

## Where the numbers come from

Four config blocks in `.oxplow/project.yaml`:

- `measures` -- what is being counted, and how it aggregates.
- `gauges` -- the script that produces the raw numbers. Starlark,
  jq, or an external program.
- `metrics` -- a chartable view over a measure: aggregation, filters,
  target, direction.
- `dimensions` -- the axes you can slice by (package, language,
  branch, and any you add).

Test results and coverage arrive separately, through
[collection](#collection) -- oxplow watches the test commands you
run and parses the reports they produce, per effort.

## Letting the agent write them

Metrics are authored by the agent, not through a form. Ask for what
you want counted:

> Track how many `unwrap()` calls are in the Rust crates, and set a
> target of zero.

The agent has a `scaffold_metric` MCP tool that writes the
measure/gauge/metric trio plus the gauge script, then runs it to
confirm it produces a number. The `/oxplow:new-metric` skill walks
the same path with more structure.

This is deliberate: a metric is a small program, and describing what
you want counted is faster than filling in four config blocks by
hand.

## Collection

Test and coverage numbers are scoped to the **effort** that produced
them, so a task's page shows which tests ran and what the diff
coverage was for that specific piece of work. Configure it once with
`/oxplow:configure`, which wires each test stack in the project to
emit reports oxplow can parse.

The catch worth knowing: only test runs oxplow can see get recorded.
If your project's configured command is `bun run test:collect`, a
bare `bun test` produces no reports and the effort shows nothing.
