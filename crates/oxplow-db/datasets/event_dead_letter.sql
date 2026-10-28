SELECT d.id, d.consumer, d.event_seq, e.id AS event_id, e.type AS event_type,
       d.error, d.attempts, d.first_failed_at, d.last_failed_at, d.state
FROM source('event_dead_letter') d
JOIN source('event_log') e ON e.seq = d.event_seq
