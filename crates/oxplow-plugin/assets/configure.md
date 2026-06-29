---
description: Set up oxplow collection — wire EVERY test stack in the project to emit standard-format coverage + test reports and record them in oxplow.yaml.
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
to emit reports and list **all** their reports in the profile —
oxplow merges whatever's freshest per effort, so every stack lights
up.

## 1. Make each stack emit a coverage report

For every stack, make a coverage report a **default of its normal test
run**, at a stable repo-relative path in a standard format oxplow
parses: `cobertura` (XML), `lcov` (`.info`), or `jacoco-xml`.

- **Rust** — `cargo llvm-cov --lcov --output-path target/coverage/lcov.info`. Format `lcov`.
- **Python (pytest)** — `--cov --cov-report=xml:coverage.xml` in `addopts`. Format `cobertura`.
- **JS/TS (jest / vitest / bun)** — enable the `cobertura`/`lcov` coverage reporter to a fixed path.
- **Java / Kotlin** — JaCoCo plugin + XML report goal in `pom.xml`/`build.gradle`. Format `jacoco-xml`.

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

## 3. Record every report in oxplow.yaml

List **all** reports across **all** stacks under `collection.reports`:

```yaml
collection:
  testCommand: "<command that runs the tests>"   # informational
  reports:
    # Rust
    - { path: target/coverage/lcov.info, format: lcov }
    - { path: target/nextest/default/junit.xml, format: junit }
    # Frontend
    - { path: apps/desktop/coverage/cobertura-coverage.xml, format: cobertura }
    - { path: apps/desktop/test-report.xml, format: junit }
  # Extra command substrings that count as a test run, on top of the
  # built-in defaults (pytest, cargo test, jest, go test, …):
  testRunPatterns: [bun test]
```

`format` ∈ `lcov` | `cobertura` | `jacoco-xml` (coverage) | `junit`
(test results), **plus any format a project plugin registers** (next
step). Once set, oxplow collects **automatically**: on each
test run it sees, it parses every report **fresher than the effort
start** (so a frontend run uses the frontend reports, a Rust run the
Rust reports), merging JUnit into the per-test tree and coverage into
diff coverage. You never parse or report any of these numbers yourself
— oxplow does, so they stay trustworthy (`observed`, not `asserted`).

## 4. (Advanced) A stack whose report oxplow can't parse

If a stack only emits a format that isn't one of the built-ins, don't
fall back to asserting numbers — register a **plugin** instead, under
`collection.plugins`. A plugin maps the report into oxplow's
coverage/test shape and runs in-process (no recompile):

```yaml
collection:
  reports:
    - { path: target/clover.xml, format: clover }
  plugins:
    - name: acme.clover   # namespaced; oxplow. is reserved
      kind: coverage        # coverage | test
      formats: [clover]     # format name(s) this plugin claims
      runtime: jaq          # jaq (jq) | starlark | exec
      input: xml            # host pre-parse: text | json | xml | lcov | lines
      entryFile: oxplow/plugins/clover.jq   # the script file (a jq program here)
```

The script goes in its own file (`entryFile`, project-relative), not
inline in the yaml. Prefer `jaq` (jq) — the host pre-parses the
container (`input`) so the script just reshapes JSON. Use `starlark`
for logic jq can't express, or `exec` (`entryFile` is the executable;
raw report on stdin → JSON on stdout) as
a last resort. The host pre-parses the report (per `input:`); the
parser's job is to emit oxplow's coverage/test JSON schema.
