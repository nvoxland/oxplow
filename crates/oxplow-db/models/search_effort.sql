-- What site search indexes for each effort: its title, and in the body its
-- summary and the work item it was on (the ref, so the item's id finds it).
-- Efforts on archived threads are included.
SELECT CAST(e.ref AS TEXT) AS ref,
       CAST(e.title AS TEXT) AS title,
       CAST(coalesce(e.summary, '') || char(10) || coalesce(e.work_item, '') AS TEXT) AS body,
       CAST('str' || e.stream_id AS TEXT) AS stream_id
FROM ref('effort') e
