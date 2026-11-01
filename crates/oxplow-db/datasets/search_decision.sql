-- What site search indexes for each decision: the question, and in the body
-- the choice, why, and the alternatives that were weighed. Decisions on
-- archived threads are included. A decision never changes, so `version` is
-- when it was made. `seq` is its row id: an insert-only recompute reads on
-- from the highest one indexed.
SELECT CAST(d.uri AS TEXT) AS uri,
       CAST(d.question AS TEXT) AS title,
       CAST(coalesce(d.choice, '') || char(10) || coalesce(d.why, '') || char(10)
            || coalesce(d.alternatives, '') AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id,
       CAST(d.created_at AS TEXT) AS version,
       d.id AS seq
FROM ref('decision') d
LEFT JOIN source('threads') th ON th.id = d.thread_id
