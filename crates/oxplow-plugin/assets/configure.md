---
description: Set up oxplow collection — wire the project's test tooling to emit standard-format coverage + test reports and record the profile in oxplow.yaml.
---

Set up oxplow's **collection** so it can track which tests ran (the
individual tests, as a tree) and the diff coverage on each effort's
changed lines. See the `oxplow-collection` skill for the standing
rules. File this as a task first (the normal filing rule applies —
you'll be editing project files), then do three things:

## 1. Instrument the test tooling to always emit a coverage report

Inspect the project's build/test configuration and make a coverage
report a **default of the normal test run**, written to a stable
repo-relative path in a standard format. oxplow parses three formats
deterministically — pick whichever the tooling emits naturally:
`cobertura` (XML), `lcov` (`.info`), or `jacoco-xml`.

Representative wiring:

- **Rust** — `cargo llvm-cov --lcov --output-path target/coverage/lcov.info`
  (add a cargo alias / CI step). Format `lcov`.
- **Python (pytest)** — add `--cov --cov-report=xml:coverage.xml` to
  `addopts` in `pyproject.toml` / `pytest.ini` / `setup.cfg`. Format
  `cobertura`.
- **JS/TS (jest / vitest)** — enable the `cobertura` or `lcov` coverage
  reporter to a fixed `coverage/` path.
- **Java / Kotlin** — add the JaCoCo plugin + an XML report goal to
  `pom.xml` / `build.gradle`. Format `jacoco-xml`.

Make the **smallest** change that makes coverage automatic, and leave
the diff for the user to review — these are committed project files.

## 2. Instrument the test tooling to emit a JUnit report

To show the **individual tests that ran** (grouped into a tree), make
the test run also emit a **JUnit XML** report at a stable path — the
cross-language format oxplow parses. Representative wiring:

- **Python (pytest)** — add `--junit-xml=target/test-report.xml` to
  `addopts`.
- **JS/TS (jest)** — add the `jest-junit` reporter to a fixed path;
  vitest has `--reporter=junit --outputFile`.
- **Go** — pipe through `go-junit-report > target/test-report.xml`.
- **Rust** — `cargo test` can't emit JUnit; use **cargo-nextest** with
  a `[profile.<name>.junit] path = "junit.xml"` in `.config/nextest.toml`
  (lands at `target/nextest/<profile>/junit.xml`), and run
  `cargo nextest run`.

Use the same `classname` grouping the tool emits — oxplow builds the
tree by splitting it on `::` / `.`.

## 3. Record the profile in oxplow.yaml

Add (or update) a `collection:` block describing what you wired up:

```yaml
collection:
  testCommand: "<the command that runs the tests>"
  coverageReportPath: "<repo-relative path the coverage report lands at>"
  coverageFormat: cobertura      # | lcov | jacoco-xml
  testReportPath: "<repo-relative path the JUnit report lands at>"
  testReportFormat: junit
  # Optional — extra command substrings that count as a test run, on
  # top of the built-in defaults (pytest, cargo test, jest, go test, …):
  testRunPatterns: []
```

Every field is optional — wire coverage, test results, or both. Once
set, oxplow collects **automatically**: it records each test run it
sees via the Bash hook, parses the JUnit report into the individual-
test tree, and parses the coverage report into diff coverage over the
effort's changed lines. You never parse or report any of these numbers
yourself — oxplow does, so they stay trustworthy (`observed`, not
`asserted`).
