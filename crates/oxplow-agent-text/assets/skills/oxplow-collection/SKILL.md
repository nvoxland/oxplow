---
name: oxplow-collection
description: How Oxplow displays test reports and diff coverage when a project has configured collection.
---

# Test reports in Oxplow

Follow the project's testing workflow. Oxplow can attach a fresh report to
the request when it observes the run complete. If the Tests panel matters
for this task, use the project's report-emitting command and let it finish
in the foreground. `oxplow.testing.fastCommand` often provides a focused
run, while `oxplow.testing.command` may also produce coverage.

Missing reports leave ordinary test output and exit status available in the
harness. A report already on disk can be collected with
`oxplow.collector.sync`. Use the `configure` command when the person wants
to set up or change report collection.

Oxplow reads configured report files directly. See
`references/custom-parsers.md` for additional formats.
