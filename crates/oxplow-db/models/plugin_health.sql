SELECT h.plugin, h.contribution, h.kind, h.state, h.reason, h.consecutive_failures,
       h.last_ok_at, h.last_error, h.mean_ms, h.next_due_at, h.updated_at,
       (SELECT count(*) FROM source('event_dead_letter') d
          JOIN source('event_log') e ON e.seq = d.event_seq
         WHERE d.state = 'pending'
           AND (d.consumer = 'extension:' || h.plugin || '/' || h.contribution
                OR EXISTS (SELECT 1 FROM json_each(e.subject) s
                            WHERE s.value = 'plugin:' || h.plugin))) AS dead_letters,
       (h.next_due_at IS NULL OR h.next_due_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) AS fresh
FROM source('plugin_health') h
