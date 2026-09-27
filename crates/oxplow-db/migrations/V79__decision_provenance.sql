-- Inferred decisions (tsk302): a decision is either `recorded` by the agent
-- (MCP record_decision) or `inferred` by oxplow's summarize model from the
-- effort's activity. Inferred ones are proposals a reviewer confirms; they
-- are never fed back to the agent as its own decisions.
ALTER TABLE decision ADD COLUMN provenance TEXT NOT NULL DEFAULT 'recorded'
    CHECK (provenance IN ('recorded', 'inferred'));

DROP VIEW v_decision;
CREATE VIEW v_decision AS
SELECT id, thread_id, task_id, effort_id, question, choice,
       alternatives_json AS alternatives, confidence, why, provenance, created_at
FROM decision;
