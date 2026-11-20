---
name: csharp-setup
description: Make a C# project write the reports oxplow reads - JUnit test results and cobertura coverage from dotnet test. Use when you set up oxplow collection for a .NET project, or when its test runs show no report.
---

# C#: reports oxplow reads

oxplow reads reports. You never type the numbers yourself.
Make each report a default of the normal run. Keep each change small.

## Tests (JUnit)

Add the `JunitXml.TestLogger` package to each test project. Then run:

```
dotnet test --logger "junit;LogFilePath=target/test-report.xml"
```

Declare it in `.oxplow/extensions/project/extension.yaml`:

```yaml
- { id: tests.dotnet_junit, records: tests, entry: "oxplow:junit", report: { path: target/test-report.xml }, on: { run: test } }
```

## Coverage (cobertura)

Add the `coverlet.collector` package to each test project. Then run:

```
dotnet test --collect:"XPlat Code Coverage" --results-directory target/coverage
```

Coverlet writes `coverage.cobertura.xml` in a folder named by a GUID.
To get a fixed path, set the `--results-directory` to a clean folder and
add `-- DataCollectionRunSettings.DataCollectors.DataCollector.Configuration.Format=cobertura`.
If you can not fix the path, copy the file to `target/coverage/cobertura.xml` in the test script.

Declare it:

```yaml
- { id: tests.dotnet_coverage, records: coverage, entry: "oxplow:cobertura", report: { path: target/coverage/cobertura.xml }, on: { run: test } }
```

## Commands

Set the test command with `oxplow.testing.command`. Use the run that
also writes coverage. Set `oxplow.testing.fastCommand` for a run with
no coverage that takes a filter.
