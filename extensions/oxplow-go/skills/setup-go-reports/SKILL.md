---
name: setup-go-reports
description: Make a Go project's go test run write the JUnit and coverage reports oxplow reads. Use when setting up collection for a Go project, or when its test runs show no individual tests or no coverage.
---

# Go reports for oxplow

oxplow reads a JUnit report (the individual tests) and an lcov report
(coverage). `go test` writes neither by itself. Add two small steps.

1. JUnit: install `go-junit-report` and pipe the test output to it.

```sh
go test -v -coverprofile=target/coverage.out ./... 2>&1 \
  | go-junit-report > target/test-report.xml
```

2. Coverage: `go test -coverprofile` writes the Go format. Convert it
   to lcov with a tool such as `gcov2lcov`.

```sh
gcov2lcov -infile=target/coverage.out -outfile=target/coverage/lcov.info
```

3. Put both steps in one script or Make target. Record it as
   `oxplow.testing.command`.
4. Declare both files as report collectors in the project's extension
   (`.oxplow/extensions/project/extension.yaml`). The test parser is
   `oxplow:junit`. The coverage parser is `oxplow:lcov`.
5. Make the smallest change that works. Leave the diff for the person
   to review.
