---
name: oxplow-collection
description: Standing rules for oxplow's effort-scoped collection (which tests ran + diff coverage). Loads when finishing/closing a task (complete_task), when the user asks about test coverage or "what tests ran", and on /oxplow:configure. Keeps coverage flowing without bit-rot after a one-time configure.
---

# Collection — keep coverage flowing per effort

oxplow attaches **observations** to each task effort: which tests ran,
and diff coverage on the lines the effort changed. Collection is mostly
automatic; your job is small and is about making sure the data exists,
**not** producing numbers.

## The one rule

When you finish work on a task, **run the project's tests before you
`complete_task`**, so a fresh coverage report exists for oxplow to
attribute to the effort. The test command is recorded in the
`collection:` block of `oxplow.yaml` (`testCommand`).

## How collection works (so you don't double-do it)

- **Test runs are observed automatically.** When you run the tests via
  Bash, oxplow's PostToolUse hook records a `test-run` observation
  against the open effort (command + exit code). You don't report it.
- **Coverage is parsed by oxplow, not you.** If `collection.coverageReportPath`
  is configured, oxplow reads + parses that report (cobertura / lcov /
  jacoco-xml) after a detected test run and stores a `diff-coverage`
  observation over the effort's changed lines. **Never read the report
  and type the numbers** — that would make the figure `asserted` and
  untrustworthy. Let oxplow do it (`observed`).

## When the data is missing

- **Tooling isn't emitting a report** at `coverageReportPath` (or no
  `collection:` block exists yet) → run `/oxplow:configure` to wire the
  test tool to emit a standard-format report and record the profile.
- **Report is at a non-standard location for this run** → call
  `mcp__oxplow__ingest_coverage` with the path/format to ingest it
  explicitly (it goes through the same deterministic parse path).

Do not file a follow-up to "add coverage later" — either it's
configured and automatic, or you run `/oxplow:configure` now.
