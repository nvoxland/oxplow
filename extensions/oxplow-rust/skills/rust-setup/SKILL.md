---
name: rust-setup
description: Make a Rust project write the reports oxplow reads - JUnit test results, lcov coverage and the clippy report. Use when you set up oxplow collection for a Rust project, or when its test or analysis runs show no report.
---

# Rust: reports oxplow reads

oxplow reads reports. You never type the numbers yourself.
Make each report a default of the normal run. Keep each change small.

## Tests (JUnit)

The stock test runner cannot write JUnit. Use cargo-nextest.

In `.config/nextest.toml`:

```toml
[profile.default.junit]
path = "junit.xml"
```

The file lands at `target/nextest/default/junit.xml`.
Keep the tool's own `classname`. oxplow builds the test tree from it.

Declare it in `.oxplow/extensions/project/extension.yaml`:

```yaml
- { id: tests.rust_junit, records: tests, entry: "oxplow:junit", report: { path: target/nextest/default/junit.xml }, on: { run: test } }
```

## Coverage (lcov)

Use `cargo-llvm-cov`:

```
cargo llvm-cov --lcov --output-path target/coverage/lcov.info
```

Declare it:

```yaml
- { id: tests.rust_coverage, records: coverage, entry: "oxplow:lcov", report: { path: target/coverage/lcov.info }, on: { run: test } }
```

## Clippy

This pack already reads the clippy report. Make clippy write it:

```
cargo clippy --message-format=json > target/clippy.json
```

The pack reads `target/clippy.json` by default after an analysis run
(`cargo clippy`). To use another path, set `rust.clippyReport` in the
pack's settings.

## Commands

Set the test command with `oxplow.testing.command`. Use the nextest run
that also writes coverage. Set `oxplow.testing.fastCommand` for a run
with no coverage that takes a filter.
