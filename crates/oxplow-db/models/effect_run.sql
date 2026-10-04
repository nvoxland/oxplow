SELECT r.id, r.effect, substr(r.effect, 1, instr(r.effect, '/') - 1) AS extension,
       r.event_id, r.event_seq, r.attempt, r.origin,
       r.attempt = (SELECT max(l.attempt) FROM source('effect_run') l
                     WHERE l.effect = r.effect AND l.event_id = r.event_id) AS latest,
       r.state, r.reason, r.audit_id, r.proposal_id,
       r.started_at, r.finished_at, r.retry_at
FROM source('effect_run') r
