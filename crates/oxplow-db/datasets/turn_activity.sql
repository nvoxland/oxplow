-- What happened inside each turn, counted from its events (anchored to the
-- turn) and its tool calls: kept apart from v_agent_turn, which many pages
-- read whole. Every figure is read through the turn's own indexes.
--
-- Where its time went, once it ended: tool_ms is the union of its tool
-- calls' intervals (calls run in parallel), wait_ms the time its waits on
-- the person ran (a status `awaiting_user` until the next status change,
-- cut at the turn's end), other_ms the rest — the turn less the union of
-- both, so a permission wait inside a call is not counted twice.
SELECT t.id AS turn_id, t.thread_id,
       CAST(uri('oxplow/turn', 'id', 'trn' || t.id) AS TEXT) AS turn,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.context.compacted') AS INTEGER) AS compactions,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.prompt.submitted') AS INTEGER) AS deliveries,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.prompt.submitted'
                AND json_extract(e.payload, '$.source') = 'person') AS INTEGER) AS person_prompts,
       CAST(CASE WHEN t.ended_at IS NULL THEN NULL
                 ELSE round((julianday(t.ended_at) - julianday(t.started_at)) * 86400000) END
            AS INTEGER) AS duration_ms,
       CAST(CASE WHEN t.ended_at IS NULL THEN NULL
                 -- A call with no start: the harness gives no tool times.
                 WHEN EXISTS (SELECT 1 FROM source('agent_tool_call') c
                               WHERE c.turn_id = t.id AND c.state <> 'denied'
                                 AND c.started_at IS NULL) THEN NULL
                 ELSE coalesce(round((
                   SELECT sum(max(0, u.e - max(u.s, coalesce(u.pe, u.s))))
                     FROM (SELECT c.s, c.e,
                                  max(c.e) OVER (ORDER BY c.s ROWS BETWEEN UNBOUNDED PRECEDING
                                                                   AND 1 PRECEDING) AS pe
                             FROM (SELECT julianday(c.started_at) AS s,
                                          julianday(coalesce(c.ended_at, t.ended_at)) AS e
                                     FROM source('agent_tool_call') c
                                    WHERE c.turn_id = t.id AND c.state <> 'denied') c) u
                 ) * 86400000), 0) END AS INTEGER) AS tool_ms,
       CAST(CASE WHEN t.ended_at IS NULL THEN NULL ELSE (
         SELECT count(*) FROM source('event_log') e
          WHERE e.turn_id = t.id AND e.type = 'agent.status.changed'
            AND json_extract(e.payload, '$.state') = 'awaiting_user'
            AND julianday(e.at) < julianday(t.ended_at)) END AS INTEGER) AS waits,
       CAST(CASE WHEN t.ended_at IS NULL THEN NULL ELSE coalesce(round((
         SELECT sum(min(coalesce(w.nxt, w.fin), w.fin) - w.a)
           FROM (SELECT julianday(e.at) AS a, json_extract(e.payload, '$.state') AS st,
                        lead(julianday(e.at)) OVER (ORDER BY e.seq) AS nxt,
                        julianday(t.ended_at) AS fin
                   FROM source('event_log') e
                  WHERE e.turn_id = t.id AND e.type = 'agent.status.changed') w
          WHERE w.st = 'awaiting_user' AND w.a < w.fin
       ) * 86400000), 0) END AS INTEGER) AS wait_ms,
       CAST(CASE WHEN t.ended_at IS NULL THEN NULL
                 WHEN EXISTS (SELECT 1 FROM source('agent_tool_call') c
                               WHERE c.turn_id = t.id AND c.state <> 'denied'
                                 AND c.started_at IS NULL) THEN NULL
                 ELSE max(0, round((julianday(t.ended_at) - julianday(t.started_at)) * 86400000)
                             - coalesce(round((
                   SELECT sum(max(0, u.e - max(u.s, coalesce(u.pe, u.s))))
                     FROM (SELECT b.s, b.e,
                                  max(b.e) OVER (ORDER BY b.s ROWS BETWEEN UNBOUNDED PRECEDING
                                                                   AND 1 PRECEDING) AS pe
                             FROM (SELECT julianday(c.started_at) AS s,
                                          julianday(coalesce(c.ended_at, t.ended_at)) AS e
                                     FROM source('agent_tool_call') c
                                    WHERE c.turn_id = t.id AND c.state <> 'denied'
                                   UNION ALL
                                   SELECT w.a, min(coalesce(w.nxt, w.fin), w.fin)
                                     FROM (SELECT julianday(e.at) AS a,
                                                  json_extract(e.payload, '$.state') AS st,
                                                  lead(julianday(e.at)) OVER (ORDER BY e.seq) AS nxt,
                                                  julianday(t.ended_at) AS fin
                                             FROM source('event_log') e
                                            WHERE e.turn_id = t.id
                                              AND e.type = 'agent.status.changed') w
                                    WHERE w.st = 'awaiting_user' AND w.a < w.fin) b) u
                 ) * 86400000), 0)) END AS INTEGER) AS other_ms
  FROM source('agent_turn') t
