---
description: Set up oxplow collection — wire EVERY test stack in the project to emit standard-format coverage + test reports and record them in .oxplow/project.yaml.
---

Set up oxplow's **collection** so it can track which tests ran (the
individual tests, as a tree) and the diff coverage on each effort's
changed lines. See the `oxplow-collection` skill for the standing
rules. File this as a task first (the normal filing rule applies —
you'll be editing project files), then:

## 0. Inventory EVERY test stack in the repo

A repo often has more than one — e.g. a Rust workspace **and** a
JS/TS frontend, or a backend + a separate e2e suite. Find them all
(look for `Cargo.toml`, `package.json`, `pyproject.toml`/`pytest.ini`,
`go.mod`, `pom.xml`/`build.gradle`, etc.). You will wire **each** one
to emit reports and declare **each** report as a report collector in
`.oxplow/project.yaml` — oxplow reads every collector a run's kind
triggers and keeps what that run wrote, so every stack lights up.

## 1. Make each stack emit a coverage report

For every stack, make a coverage report a **default of its normal test
run**, at a stable repo-relative path in a standard format oxplow
parses with a bundled parser: `cobertura` (XML), `lcov` (`.info`), or
`jacoco` (XML) — named `entry: oxplow:<name>`.

- **Rust** — `cargo llvm-cov --lcov --output-path target/coverage/lcov.info`. Parser `oxplow:lcov`.
- **Python (pytest)** — `--cov --cov-report=xml:coverage.xml` in `addopts`. Parser `oxplow:cobertura`.
- **JS/TS (jest / vitest / bun)** — enable the `cobertura`/`lcov` coverage reporter to a fixed path.
- **Java / Kotlin** — JaCoCo plugin + XML report goal in `pom.xml`/`build.gradle`. Parser `oxplow:jacoco`.

## 2. Make each stack emit a JUnit report

To show the **individual tests** (as a tree), make each stack's test
run also emit **JUnit XML** at a stable path:

- **Python (pytest)** — `--junit-xml=target/test-report.xml` in `addopts`.
- **JS/TS** — jest: `jest-junit` reporter; vitest: `--reporter=junit --outputFile`; bun: `--reporter=junit --reporter-outfile=…`.
- **Go** — `go-junit-report > target/test-report.xml`.
- **Rust** — `cargo test` can't emit JUnit; use **cargo-nextest** with `[profile.<name>.junit] path = "junit.xml"` in `.config/nextest.toml` (lands at `target/nextest/<profile>/junit.xml`).

Keep the tool's natural `classname` — oxplow builds the tree by
splitting `classname`+`name` on `::` / `.`.

Make the **smallest** change that makes each report automatic, and
leave the diffs for the user to review — these are committed files.

## 3. Record the test commands and every report in .oxplow/project.yaml

The commands go under `testing:`; each report is read by a **report
collector** under `collectors:` — one per report, across **all**
stacks:

```yaml
testing:
  command: "<command that runs the tests and writes the reports>"
  fastCommand: "<the same without coverage, taking a filter>"   # optional
  # Extra command substrings that count as a test (or analysis) run, on
  # top of the built-in defaults (pytest, cargo test, jest, go test, …):
  runPatterns: [bun test]
  analysisPatterns: [lint:collect]
  agentHint: "Run tests with <command>."   # injected into every agent prompt

collectors:
  # Rust
  - { id: tests.rust_coverage, records: coverage, entry: "oxplow:lcov", report: { path: target/coverage/lcov.info }, trigger: { on_run: test } }
  - { id: tests.rust_junit, records: tests, entry: "oxplow:junit", report: { path: target/nextest/default/junit.xml }, trigger: { on_run: test } }
  # Frontend
  - { id: tests.desktop_coverage, records: coverage, entry: "oxplow:cobertura", report: { path: apps/desktop/coverage/cobertura-coverage.xml }, trigger: { on_run: test } }
  - { id: tests.desktop_junit, records: tests, entry: "oxplow:junit", report: { path: apps/desktop/test-report.xml }, trigger: { on_run: test } }
```

`records` is what the report holds: `tests` (JUnit), `coverage` or
`analysis` (linter findings). `entry: "oxplow:<parser>"` names a bundled
parser: `junit`, `lcov`, `cobertura`, `jacoco`, `clippy`, `eslint`.
`trigger: { on_run: test }` reads the report after each test run oxplow
sees, when that run wrote it (an analyzer's report: `on_run: analysis`);
without a trigger it runs only by hand (`collector.sync`). So a frontend
run uses the frontend reports, a Rust run the Rust reports, JUnit
merging into the per-test tree and coverage into diff coverage. You
never parse or report any of these numbers yourself — oxplow does, so
they stay trustworthy (`observed`, not `asserted`).

Each run is recorded as the collector's (Settings → Data shows the last
one). A collector whose report fails to parse three times in a row is
turned off until a person turns it back on.

## 4. (Advanced) A stack whose report oxplow can't parse

If a stack only emits a format no bundled parser reads, don't fall back
to asserting numbers — give its report collector its **own parser**: a
script that maps the report into oxplow's coverage/test/analysis shape,
run in-process (no recompile):

```yaml
collectors:
  - id: tests.clover
    records: coverage
    runtime: jaq                        # jaq (jq) | starlark | exec
    entry: oxplow/parsers/clover.jq     # the script file
    report: { path: target/clover.xml, format: xml }   # host pre-parse: text | json | xml | lcov | lines
    trigger: { on_run: test }
```

The script goes in its own file (`entry`, project-relative), not inline
in the yaml. Prefer `jaq` (jq) — the host pre-parses the report
(`format`) so the script just reshapes JSON. Use `starlark` for logic jq
can't express, or `exec` (`entry` is the executable; the raw report on
stdin, JSON on stdout) as a last resort: a program runs only once a
person approves it on their machine (Settings → Data → Programs).
