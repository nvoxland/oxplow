# schema-review

Facts about a project's database migrations, and a page on an effort's diff
that lists the migration files it changed.

| | |
|---|---|
| **Schema Changes** (an effort's page) | The migration files the effort changed, and how many tables each creates, alters and drops. |

## How it's built

Only from oxplow's public contracts, so nothing in oxplow changed for it:

- **Its own facts.** The `ddl` collector counts `CREATE TABLE`,
  `ALTER TABLE` and `DROP TABLE` in each `migrations/**/*.sql` file of
  every snapshot: a `schema_review.ddl` fact per file and kind, sliced by
  its own `schema_review.ddl_kind` dimension. It runs no program and asks
  no model, so it asks for nothing beyond `files: read`.
- **oxplow's page.** A slot page mounted on the effort's diff joins those
  facts to the effort's files (`v_effort_file`).

## Using it

Copy `schema_review/` into your project's `.oxplow/extensions/`.
