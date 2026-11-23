-- A function the change removed the last call to (the `oxplow.review.unused`
-- check's rows): it may be unused now. A text search, so it says "possibly".
-- And, once per change, the files too large for the check to read
-- (`too-large-to-check`, `info`, no path).
SELECT u.change_id, 'oxplow/review_signal_unused' AS source, 'code' AS area, u.signal,
       CASE u.signal WHEN 'too-large-to-check' THEN 'info' ELSE 'medium' END AS severity, u.path, u.line, NULL AS end_line, u.title, u.why, u.uri
FROM ref('review_unused') u
