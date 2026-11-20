---
name: clojure-setup
description: Make a Clojure project write the reports oxplow reads - JUnit test results and cobertura coverage. Use when you set up oxplow collection for a Clojure project, or when its test runs show no report.
---

# Clojure: reports oxplow reads

oxplow reads reports. You never type the numbers yourself.
Make each report a default of the normal run. Keep each change small.

## Tests (JUnit)

With Kaocha, add the JUnit plugin in `tests.edn`:

```clojure
#kaocha/v1 {:plugins [:kaocha.plugin/junit-xml]
            :kaocha.plugin.junit-xml/target-file "target/test-report.xml"}
```

With `lein test`, use a JUnit reporter plugin that writes the same path.

Declare it in `.oxplow/extensions/project/extension.yaml`:

```yaml
- { id: tests.clj_junit, records: tests, entry: "oxplow:junit", report: { path: target/test-report.xml }, on: { run: test } }
```

## Coverage (cobertura)

Use cloverage with the cobertura output:

```
clojure -M:test:coverage --cobertura --output target/coverage
```

Or, with Leiningen, `lein cloverage --cobertura`. The file is
`target/coverage/coverage.xml`.

Declare it:

```yaml
- { id: tests.clj_coverage, records: coverage, entry: "oxplow:cobertura", report: { path: target/coverage/coverage.xml }, on: { run: test } }
```

## Commands

Set the test command with `oxplow.testing.command`. Use the run that
also writes coverage. Set `oxplow.testing.fastCommand` for a run with
no coverage that takes a filter.
