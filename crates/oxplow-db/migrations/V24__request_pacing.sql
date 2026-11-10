-- Advisories and nudges pace per request: a one-shot mark and a nudge's
-- dedupe also scope by the request of the turn they belong to, so a
-- once-per-request hint fires again for the next request.
ALTER TABLE once_mark ADD COLUMN request_id INTEGER REFERENCES request(id) ON DELETE CASCADE;
DROP INDEX idx_once_mark;
CREATE UNIQUE INDEX idx_once_mark
    ON once_mark(thread_id, coalesce(effort_id, 0), coalesce(request_id, 0), mark);

ALTER TABLE agent_nudge ADD COLUMN request_id INTEGER REFERENCES request(id) ON DELETE SET NULL;
UPDATE agent_nudge
   SET request_id = (SELECT t.request_id FROM agent_turn t WHERE t.id = agent_nudge.turn_id)
 WHERE turn_id IS NOT NULL;
DROP INDEX idx_agent_nudge_cause_kind;
CREATE UNIQUE INDEX idx_agent_nudge_cause_kind
    ON agent_nudge(cause, kind, coalesce(effort_id, 0), coalesce(request_id, 0))
    WHERE cause IS NOT NULL;
