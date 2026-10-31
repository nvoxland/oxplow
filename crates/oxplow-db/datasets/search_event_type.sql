-- What site search indexes for each event type the log accepts: its name,
-- and in the body what it records and who declares it. One row per type,
-- from its newest version. Event types belong to the project, not a stream.
SELECT CAST(uri('oxplow/event_type', 'event_type', t.event_type) AS TEXT) AS uri,
       CAST(t.event_type AS TEXT) AS title,
       CAST(trim(coalesce(t.summary, '') || ' ' || coalesce(t.extension, 'core')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('event_type') t
WHERE t.latest
