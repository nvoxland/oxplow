-- What site search indexes for each handoff: its body, under the
-- recipient thread's stream. Settled handoffs stay findable; archived
-- work leaves search, as a note does.
SELECT h.ref AS ref,
       CAST('' AS TEXT) AS title,
       CAST(h.body AS TEXT) AS body,
       CAST(CASE WHEN th.stream_id IS NULL THEN NULL ELSE 'str' || th.stream_id END AS TEXT)
         AS stream_id,
       -- The body is written once.
       CAST(h.sent_at AS TEXT) AS version
FROM ref('handoff') h
LEFT JOIN source('threads') th ON th.id = h.to_thread_id
LEFT JOIN source('streams') s ON s.id = th.stream_id
WHERE th.archived_at IS NULL
  AND s.archived_at IS NULL
