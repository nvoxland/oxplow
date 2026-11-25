-- The open list's own rows: none. Every row comes from a dataset that adds
-- to it (`adds_to: oxplow/review_signal`), this extension's and others'.
SELECT NULL AS change_id, NULL AS source, NULL AS area, NULL AS signal,
       NULL AS severity, NULL AS path, NULL AS line, NULL AS end_line,
       NULL AS title, NULL AS why, NULL AS uri, NULL AS related
WHERE 0
