SELECT s.thread_id, s.hint, s.evaluated, s.last_evaluated_at,
       (SELECT count(*) FROM source('agent_nudge') n
         WHERE n.thread_id = s.thread_id AND n.kind = s.hint) AS fired,
       (SELECT count(*) FROM source('agent_nudge') n
         WHERE n.thread_id = s.thread_id AND n.kind = s.hint AND n.delivered_at IS NOT NULL) AS delivered,
       (SELECT count(*) FROM source('agent_nudge') n
         WHERE n.thread_id = s.thread_id AND n.kind = s.hint AND n.delivered_at IS NULL) AS held,
       (SELECT count(*) FROM source('once_mark') m
         WHERE m.thread_id = s.thread_id AND m.mark = 'mute:' || s.hint) AS muted
FROM source('hint_stat') s
