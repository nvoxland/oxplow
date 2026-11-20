---
name: typescript-setup
description: Make a TypeScript or JavaScript project write the reports oxplow reads - JUnit test results, lcov or cobertura coverage and the ESLint report. Use when you set up oxplow collection for a JS or TS project, or when its test or analysis runs show no report.
---

# TypeScript and JavaScript: reports oxplow reads

oxplow reads reports. You never type the numbers yourself.
Make each report a default of the normal run. Keep each change small.

## Tests (JUnit)

Pick the line for the test runner:

- jest: add the `jest-junit` reporter.
- vitest: `--reporter=junit --outputFile=target/test-report.xml`.
- bun: `--reporter=junit --reporter-outfile=target/test-report.xml`.

Declare it in `.oxplow/extensions/project/extension.yaml`:

```yaml
- { id: tests.web_junit, records: tests, entry: "oxplow:junit", report: { path: target/test-report.xml }, on: { run: test } }
```

## Coverage (lcov or cobertura)

Turn on the `lcov` or `cobertura` coverage reporter. Give it a fixed path.

- jest: `coverageReporters: ['lcov']`. The file is `coverage/lcov.info`.
- vitest: `coverage.reporter: ['lcov']`.
- bun: `bun test --coverage --coverage-reporter=lcov`.

Declare it:

```yaml
- { id: tests.web_coverage, records: coverage, entry: "oxplow:lcov", report: { path: coverage/lcov.info }, on: { run: test } }
```

Use `oxplow:cobertura` when the report is cobertura XML.

## ESLint

This pack already reads the ESLint report. Make ESLint write it:

```
eslint . --format json --output-file target/eslint.json
```

The pack reads `target/eslint.json` by default after an analysis run
(`eslint`, `tsc --noEmit`). To use another path, set `typescript.eslintReport` in
the pack's settings.

## Commands

Set the test command with `oxplow.testing.command`. Use the run that
also writes coverage. Set `oxplow.testing.fastCommand` for a run with
no coverage that takes a filter.
