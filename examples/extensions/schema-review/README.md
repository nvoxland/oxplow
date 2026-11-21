# schema-review

Facts about a project's database migrations, a page on a request's diff
that lists the migration files it changed, and a signal on oxplow's change
review: a changed migration that drops a table is `high`.

| | |
|---|---|
| **Schema Changes** (a request's page) | The migration files the request changed, and how many tables each creates, alters and drops. |

## How it's built

Only from oxplow's public contracts, so nothing in oxplow changed for it:

- **Its own facts.** The `ddl` collector counts `CREATE TABLE`,
  `ALTER TABLE` and `DROP TABLE` in each `migrations/**/*.sql` file of
  every snapshot: a `schema_review.ddl` fact per file and kind, sliced by
  its own `schema_review.ddl_kind` dimension. It runs no program and asks
  no model, so it asks for nothing beyond `files: read`.
- **oxplow's page.** A slot page mounted on the request's diff joins those
  facts to the request's files (`v_turn_file`).
- **A review signal.** The dataset `review_signal_schema` says
  `adds_to: oxplow/review_signal`, so its rows join every other source on
  the Review page, ranked with them. It reads the facts as of the change's
  head (`v_change_fact`). A migration that drops a table is `high`; any
  other changed migration gives no severity, and the review shows it as
  `medium`. It needs `oxplow-bundled`, which owns the review.

## Using it

Copy `schema_review/` into your project's `.oxplow/extensions/`.
