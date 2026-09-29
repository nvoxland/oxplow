-- The retention sweep's lookup (tsk502): live payloads of one namespace
-- older than its window. Partial, so it shrinks as payloads expire; the
-- sweep reads it through a `type` range (a LIKE can't use a BINARY index).
CREATE INDEX event_log_live_payload ON event_log (type, at) WHERE payload_expired_at IS NULL;
