-- A nudge is once per event, kind AND effort (tsk510): one event can nudge
-- two efforts of the same kind, and the old (cause, kind) key dropped the
-- second. A nudge with no effort keys as effort 0.
DROP INDEX idx_agent_nudge_cause_kind;
CREATE UNIQUE INDEX idx_agent_nudge_cause_kind
    ON agent_nudge(cause, kind, coalesce(effort_id, 0)) WHERE cause IS NOT NULL;
