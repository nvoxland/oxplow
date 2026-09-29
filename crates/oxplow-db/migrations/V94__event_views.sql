-- The event log in the semantic layer (tsk409; .context/semantic-layer.md).
-- `v_event` is the timeline: every envelope with its anchors, oldest
-- first by `seq`. `v_event_dead_letter` is the parked work a person owes
-- a decision on, joined to the event it is about. `v_event_checkpoint`
-- says how far each consumer has read.
CREATE VIEW v_event AS
SELECT seq, id, type, v, at, source,
       stream_id, thread_id, effort_id, turn_id, snapshot_id,
       subject, payload, payload_hash, cause, dedupe_key
FROM event_log;

CREATE VIEW v_event_dead_letter AS
SELECT d.id, d.consumer, d.event_seq, e.id AS event_id, e.type AS event_type,
       d.error, d.attempts, d.first_failed_at, d.last_failed_at, d.state
FROM event_dead_letter d
JOIN event_log e ON e.seq = d.event_seq;

CREATE VIEW v_event_checkpoint AS
SELECT consumer, last_seq, updated_at
FROM event_consumer_checkpoint;
