-- When the person last cleared each thread's Finished list in the Work
-- panel (`oxplow_bundled.clear_finished`). Read from the envelope alone
-- (its subject names the thread), which retention keeps.
SELECT CAST(substr(CAST(json_extract(e.subject, '$[0]') AS TEXT), 11) AS INTEGER) AS thread_id,
       CAST(max(e.at) AS TEXT) AS cleared_at
FROM ref('event') e
WHERE e.type = 'oxplow_bundled.finished_cleared'
GROUP BY 1
