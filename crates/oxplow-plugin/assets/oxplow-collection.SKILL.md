---
name: oxplow-collection
description: Standing rules for oxplow's effort-scoped collection (which tests ran + diff coverage). Loads when finishing/closing a task (effort.report), when the user asks about test coverage or "what tests ran", and on /oxplow:configure. Keeps coverage flowing without bit-rot after a one-time configure.
---

# Collection — keep coverage flowing per effort

oxplow attaches **observations** to each task effort: which tests ran,
and diff coverage on the lines the effort changed. Collection is mostly
automatic; your job is small and is about making sure the data exists,
**not** producing numbers.

## The one rule

When you finish work on a task, **run the project's tests before you
close it** (`work_item.transition` + `effort.report`), so fresh test + coverage reports exist for oxplow to
attribute to the effort. The test command is recorded in the
`testing:` block of `.oxplow/project.yaml` (`command`).

Run it three specific ways:

- **Run EVERY test invocation through a report-emitting command** — including
  **red-phase / failing** runs and quick **single-test** runs, not just the
  final green one. A bare `bun test <file>` / `cargo test <name>` is a
  report-less run and won't reach the Tests panel, so your red→green
  progression and any failures stay invisible.

    Which command depends on what you're doing:

    - **Iterating (red/green, one test, one crate): `fastCommand`** when the
      project declares one. It emits the same test report but skips coverage
      instrumentation, and it takes a filter — so it's seconds, not minutes.
    - **Before closing the task: `command`.** The full run, with coverage.
      Diff coverage for the effort comes only from this one.

    If the project declares no `fastCommand`, use `command` throughout.
    If that turns out to be too slow to run every cycle, say so and offer to add
    one — do NOT quietly fall back to a bare `cargo test`, which records
    nothing. An unrecorded run is the failure mode this rule exists to prevent.
- **Run it in the FOREGROUND, never backgrounded.** The PostToolUse hook fires
  when the Bash call *returns*; a backgrounded run returns at launch (before
  the reports regenerate), so its reports are never ingested and the effort
  panel stays empty.

## How collection works (so you don't double-do it)

- **Test runs are observed automatically (foreground only).** When you run the
  tests via Bash, oxplow's PostToolUse hook records a `test-run` observation
  against the effort (command + exit code, + the parsed suite tree). You don't
  report it: the run is the effort whose tool call ran it.
- **Individual tests + coverage are parsed by oxplow, not you.** Each
  report collector (`collectors:` with `records:` — JUnit → per-test
  tree; lcov / cobertura / jacoco → diff coverage over the effort's
  changed lines) reads its report after a run that wrote it — so in a
  polyglot repo each stack's report lights up on its own run. **Never read a
  report and type the numbers/test names** — that would make them
  `asserted` and untrustworthy. Let oxplow do it (`observed`).

## When the data is missing

- **A stack isn't emitting a report** (no report collector reads it, or
  the project has none yet) → run `/oxplow:configure`, which wires
  **every** test stack in the repo.
- **A report was written outside a run oxplow saw** → run its collector
  by hand: `collector.sync { owner: "project", id }`
  (`mcp__oxplow__run_command`); it reads the report now and records it
  in your thread, through the same deterministic parse.

Do not file a follow-up to "add coverage later" — either it's
configured and automatic, or you run `/oxplow:configure` now.

## A report format oxplow doesn't parse yet

Parsers are **pluggable** — the bundled ones (`oxplow:junit`, `lcov`,
`cobertura`, `jacoco`, `clippy`, `eslint`) are jq programs, and a report
collector can name its **own** with no recompile: a `jaq` (JSON→JSON,
primary), `starlark`, or `exec` script in the project that maps the
report into oxplow's coverage/test/analysis schema. The host pre-parses
the report for you (`report: { format: xml | json | lcov | lines | text }`)
and the script emits the schema. Coverage stays
`observed` because the in-process tiers can't do I/O.
