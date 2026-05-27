---
description: Set up oxplow collection — wire the project's test tooling to emit a standard-format coverage report and record the profile in oxplow.yaml.
---

Set up oxplow's **collection** so it can track which tests ran and the
diff coverage on each effort's changed lines. See the
`oxplow-collection` skill for the standing rules. File this as a task
first (the normal filing rule applies — you'll be editing project
files), then do two things:

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

## 2. Record the profile in oxplow.yaml

Add (or update) a `collection:` block describing what you wired up:

```yaml
collection:
  testCommand: "<the command that runs the tests>"
  coverageReportPath: "<repo-relative path the report lands at>"
  coverageFormat: cobertura   # | lcov | jacoco-xml
  # Optional — extra command substrings that count as a test run, on
  # top of the built-in defaults (pytest, cargo test, jest, go test, …):
  testRunPatterns: []
```

Once this is set, oxplow collects **automatically**: it records each
test run it sees via the Bash hook, and parses the report after a run
to attribute diff coverage to the open effort. You never parse or
report coverage numbers yourself — oxplow does, so the numbers stay
trustworthy (`observed`, not `asserted`).
