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
`complete_task`**, so fresh test + coverage reports exist for oxplow to
attribute to the effort. The test command is recorded in the
`collection:` block of `oxplow.yaml` (`testCommand`).

Run it three specific ways:

- **Run EVERY test invocation through it** — including **red-phase / failing**
  runs and quick **single-test** runs, not just the final green one. Only
  report-emitting runs (`testCommand`) enter the effort's Tests panel; a bare
  `bun test <file>` / `cargo test <name>` is a report-less run and won't show
  there (so your red→green progression and any failures stay invisible).
- **Prefix `OXPLOW_TASK=<your task id>`** (e.g. `OXPLOW_TASK=tsk42 <testCommand>`).
  The collection hook reads the token and pins the run to **exactly** your
  task's effort via `find_open_for_task` — correct even when several efforts
  are open. Without it, a run is only auto-attributed when a single effort is
  open.
- **Run it in the FOREGROUND, never backgrounded.** The PostToolUse hook fires
  when the Bash call *returns*; a backgrounded run returns at launch (before
  the reports regenerate), so its reports are never ingested and the effort
  panel stays empty.

## How collection works (so you don't double-do it)

- **Test runs are observed automatically (foreground only).** When you run the
  tests via Bash, oxplow's PostToolUse hook records a `test-run` observation
  against the effort (command + exit code, + the parsed suite tree). You don't
  report it. Attribution uses the `OXPLOW_TASK=` token when present, else the
  single-open-effort rule.
- **Individual tests + coverage are parsed by oxplow, not you.** Each
  entry in `collection.reports` (JUnit → per-test tree; lcov / cobertura
  / jacoco-xml → diff coverage over the effort's changed lines) is parsed
  by oxplow when it's fresher than the effort start — so in a polyglot
  repo each stack's report lights up on its own run. **Never read a
  report and type the numbers/test names** — that would make them
  `asserted` and untrustworthy. Let oxplow do it (`observed`).

## When the data is missing

- **A stack isn't emitting a report** (its path isn't in
  `collection.reports`, or no `collection:` block exists yet) → run
  `/oxplow:configure`, which wires **every** test stack in the repo.
- **Report is at a non-standard location for this run** → call
  `mcp__oxplow__ingest_coverage` with the path/format to ingest it
  explicitly (it goes through the same deterministic parse path).

Do not file a follow-up to "add coverage later" — either it's
configured and automatic, or you run `/oxplow:configure` now.

## A report format oxplow doesn't parse yet

Parsers are **pluggable** — the built-ins (cobertura, lcov, jacoco,
junit) are jaq plugins, and you can add a new format with **no recompile**
by writing a `collection.plugins` entry in `oxplow.yaml`: a `jaq`
(JSON→JSON, primary), `starlark`, or `exec` transform that maps the
report into oxplow's coverage/test schema. The host pre-parses the
container for you (`input: xml | json | lcov | lines | text`) and the
transform emits oxplow's coverage/test JSON schema. Coverage stays
`observed` because the in-process tiers can't do I/O.
