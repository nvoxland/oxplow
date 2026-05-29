-- Rename the "caveman" audience key to "terse" in existing JSON blobs.
-- The blobs store only the non-developer variants, e.g.
--   {"executive":"...","caveman":"..."} → {"executive":"...","terse":"..."}
-- Rows with no caveman key are unaffected.

UPDATE task
SET description_variants = json_insert(
        json_remove(description_variants, '$.caveman'),
        '$.terse',
        json_extract(description_variants, '$.caveman')
    )
WHERE description_variants IS NOT NULL
  AND json_extract(description_variants, '$.caveman') IS NOT NULL;

UPDATE task_effort
SET summary_variants = json_insert(
        json_remove(summary_variants, '$.caveman'),
        '$.terse',
        json_extract(summary_variants, '$.caveman')
    )
WHERE summary_variants IS NOT NULL
  AND json_extract(summary_variants, '$.caveman') IS NOT NULL;
