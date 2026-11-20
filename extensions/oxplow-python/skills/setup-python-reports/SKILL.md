---
name: setup-python-reports
description: Make a Python project's pytest run write the JUnit and coverage reports oxplow reads. Use when setting up collection for a project with pytest, or when its test runs show no individual tests or no coverage.
---

# Python reports for oxplow

oxplow reads a JUnit report (the individual tests) and a Cobertura
report (coverage). Make pytest write both on every normal run.

1. Install `pytest-cov` if it is not there.
2. Add the flags to `addopts`, so each run writes the reports:

```toml
# pyproject.toml
[tool.pytest.ini_options]
addopts = "--junit-xml=target/test-report.xml --cov --cov-report=xml:coverage.xml"
```

3. Keep the tool's own `classname`. oxplow builds the test tree from
   `classname` and `name`.
4. Declare both files as report collectors in the project's extension
   (`.oxplow/extensions/project/extension.yaml`). The coverage parser is
   `oxplow:cobertura`. The test parser is `oxplow:junit`.
5. Make the smallest change that works. Leave the diff for the person
   to review.
