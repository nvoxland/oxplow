-- Remove the audience-variant feature (developer / executive / terse).
-- The variants added in V27 (and renamed in V28) are gone: every entity
-- now carries a single canonical prose body. Drop the now-unused columns.
-- The developer text stays in its existing column (task.description,
-- task_effort.summary); comment.section_anchor only ever existed to
-- re-anchor a comment across variants, so it goes too.
ALTER TABLE task DROP COLUMN description_variants;
ALTER TABLE task_effort DROP COLUMN summary_variants;
ALTER TABLE comment DROP COLUMN section_anchor;
