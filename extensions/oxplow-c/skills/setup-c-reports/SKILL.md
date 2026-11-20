---
name: setup-c-reports
description: Make a C or C++ project's ctest run write the JUnit and lcov reports oxplow reads. Use when setting up collection for a C or C++ project, or when its test runs show no individual tests or no coverage.
---

# C and C++ reports for oxplow

oxplow reads a JUnit report (the individual tests) and an lcov report
(coverage).

1. JUnit: `ctest` can write it.

```sh
ctest --output-junit target/test-report.xml
```

   If your test framework writes JUnit itself (GoogleTest:
   `--gtest_output=xml:target/test-report.xml`), use that.
2. Coverage: build with coverage flags (`--coverage` for gcc or
   clang). Run the tests. Then make lcov output.

```sh
lcov --capture --directory . --output-file target/coverage/lcov.info
```

3. Put the steps in one script or Make target. Record it as
   `oxplow.testing.command`.
4. Declare both files as report collectors in the project's extension
   (`.oxplow/extensions/project/extension.yaml`). The test parser is
   `oxplow:junit`. The coverage parser is `oxplow:lcov`.
5. Make the smallest change that works. Leave the diff for the person
   to review.
