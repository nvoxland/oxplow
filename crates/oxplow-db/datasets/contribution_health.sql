SELECT CAST(h.uri AS TEXT) AS uri, h.extension, h.contribution, h.kind, h.state, h.reason, h.consecutive_failures,
       h.last_ok_at, h.last_error, h.mean_ms, h.next_due_at, h.updated_at,
       (SELECT count(*) FROM source('event_dead_letter') d
          JOIN source('event_log') e ON e.seq = d.event_seq
         WHERE d.state = 'pending'
           AND (d.consumer = h.uri
                OR EXISTS (SELECT 1 FROM json_each(e.subject) s WHERE s.value = h.uri))) AS dead_letters,
       ((h.next_due_at IS NULL OR h.next_due_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        AND NOT (h.kind = 'collector' AND h.state <> 'ok')) AS fresh,
       (SELECT w.uri FROM ref('work_item_link') l JOIN ref('work_item') w ON w.uri = l.from_uri
         WHERE l.to_uri = h.uri AND l.link_type = 'about'
           AND w.state NOT IN ('done', 'canceled')
         ORDER BY w.uri DESC LIMIT 1) AS repair_item
FROM (SELECT *, CAST(uri('oxplow/' || kind, kind, extension || '/' || contribution) AS TEXT) AS uri
        FROM source('contribution_health')) h
