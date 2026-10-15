# A report format oxplow doesn't parse yet

Parsers are **pluggable** — the bundled ones (`oxplow:junit`, `lcov`,
`cobertura`, `jacoco`, `clippy`, `eslint`) are jq programs, and a report
collector can name its **own** with no recompile: a `jaq` (JSON→JSON,
primary), `starlark`, or `exec` script in the project that maps the
report into oxplow's coverage/test/analysis schema. The host pre-parses
the report for you (`report: { format: xml | json | lcov | lines | text }`)
and the script emits the schema. Coverage stays
`observed` because the in-process tiers can't do I/O.
