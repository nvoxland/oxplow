SELECT CAST(h.ref AS TEXT) AS ref, h.extension, h.contribution, h.kind, h.state, h.reason, h.consecutive_failures,
       h.last_ok_at, h.last_error, h.mean_ms, h.next_due_at, h.updated_at,
       (SELECT count(*) FROM source('event_dead_letter') d
          JOIN source('event_log') e ON e.seq = d.event_seq
         WHERE d.state = 'pending'
           AND (d.consumer = h.ref
                OR EXISTS (SELECT 1 FROM json_each(e.subject) s WHERE s.value = h.ref))) AS dead_letters,
       ((h.next_due_at IS NULL OR h.next_due_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        AND NOT (h.kind = 'collector' AND h.state <> 'ok')) AS fresh,
       (SELECT w.ref FROM ref('work_item_link') l JOIN ref('work_item') w ON w.ref = l.from_ref
         WHERE l.to_ref = h.ref AND l.link_type = 'about'
           AND w.state NOT IN ('done', 'canceled')
         ORDER BY w.ref DESC LIMIT 1) AS repair_item
FROM (SELECT *, CAST(kind || ':' || extension || '/' || contribution AS TEXT) AS ref
        FROM source('contribution_health')) h
