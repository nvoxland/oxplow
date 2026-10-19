# schema-review

A review policy of its own, for work that touches database migrations.
oxplow-bundled's verdict on an effort asks the same questions of every
change: did the tests run, how much of the change did they cover, is
every claim backed. This one asks what matters to a schema change, and
shows its answer as the effort's verdict instead.

| | |
|---|---|
| **The verdict** (an effort's page, first line) | `Schema review: accepted`, or `changes requested` with a line per reason. Replaces oxplow-bundled's verdict (`effort.verdict`). |
| **Schema Changes** (an effort's page, below) | The migration files the effort changed, and how many tables each creates, alters and drops. |
| **A badge** (wherever a lens lists an effort) | `Schema OK` or `Schema: changes`. |

An effort asks for changes when:

- a migration it changed **drops a table**;
- its latest test run **wasn't observed green** — oxplow parsed no
  report (the agent only said the tests passed), a test failed, or none
  ran;
- its **diff coverage is under 90%**;
- a function it **added is more complex than 15**.

## How it's built

Only from oxplow's public contracts, so nothing in oxplow changed for it:

- **Its own facts.** The `ddl` collector counts `CREATE TABLE`,
  `ALTER TABLE` and `DROP TABLE` in each `migrations/**/*.sql` file of
  every snapshot: a `schema_review.ddl` fact per file and kind, sliced by
  its own `schema_review.ddl_kind` dimension. It runs no program and asks
  no model, so there is nothing to approve.
- **oxplow's evidence.** The `verdict` model joins those facts to the
  effort's files (`v_effort_file`), its latest test run (`v_test_run`, with
  its provenance), its diff coverage (`v_effort_observation`) and the
  functions its change added (`v_change_function`).
- **oxplow's page.** The verdict lens replaces `effort.verdict`, a slot
  lens adds the table, and a decorator badges the effort.

## Using it

Copy `schema-review/` into your project's `.oxplow/extensions/`. Two
review policies now offer an effort's verdict, oxplow-bundled's and this
one: choose in **Settings → Integrations** ("effort verdict"). Change the
rules by editing `models/verdict.sql`.
