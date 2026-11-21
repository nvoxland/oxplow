-- What history says is surprising about the change's files (`v_oxplow_change_co_change`):
-- a file changed without the files it usually changes with, or one that was
-- untouched for a long time. A file the change adds has no history, so it
-- is never "dormant": those rows are left out.
SELECT co.change_id, 'oxplow/review_signal_history' AS source, 'history' AS area,
       CASE co.reason WHEN 'dormant' THEN 'dormant-file' ELSE 'partners-unchanged' END AS signal,
       'info' AS severity, co.path AS path, NULL AS line, NULL AS end_line,
       CASE co.reason WHEN 'dormant' THEN 'Long-untouched file changed'
                      ELSE 'Usual co-changers not changed' END AS title,
       CASE co.reason WHEN 'dormant' THEN printf('Untouched for %d days.', co.dormant_days)
                      ELSE 'Usually changes with ' || co.expected || ', which this change doesn''t touch.' END AS why,
       NULL AS uri
FROM ref('change_co_change') co
JOIN ref('change_file') f ON f.change_id = co.change_id AND f.path = co.path
WHERE f.status <> 'added'
