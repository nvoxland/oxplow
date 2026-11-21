-- A function the change removed the last call to (the `oxplow.review.unused`
-- check's rows): it may be unused now. A text search, so it says "possibly".
SELECT u.change_id, 'oxplow/review_signal_unused' AS source, 'code' AS area, u.signal,
       'medium' AS severity, u.path, u.line, NULL AS end_line, u.title, u.why, u.uri
FROM ref('review_unused') u
