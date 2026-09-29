-- P3.2 (tsk472) — agent activity on the spine: turn anchors, idempotent
-- projections, and content by hash (.context/target-architecture.md §5.2,
-- §5.4; .context/data-model.md "event_content").
--
-- Every change is additive (ADD COLUMN / new table / new index) or a view
-- rebuild; no table is rebuilt, so no FK cascade can fire.

-- Large or sensitive event bodies (tool input and output, prompts), by the
-- xxh3-128 of their bytes — the same hash the snapshot blob store uses. An
-- event's payload carries `{hash, size}`; retention deletes the row, the
-- event stays. Separate from the blob store because its retention differs.
CREATE TABLE event_content (
    hash       TEXT    PRIMARY KEY,
    namespace  TEXT    NOT NULL,
    bytes      BLOB    NOT NULL,
    size       INTEGER NOT NULL,
    created_at TEXT    NOT NULL
) STRICT;
CREATE INDEX event_content_ns_created ON event_content(namespace, created_at);

-- `payload` is NOT NULL: payload retention writes '{}' and stamps this.
ALTER TABLE event_log ADD COLUMN payload_expired_at TEXT;

-- agent_tool_call becomes a projection of `agent.tool.finished`: one row
-- per event (redelivery is a no-op), anchored to its turn.
ALTER TABLE agent_tool_call ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE agent_tool_call ADD COLUMN event_id TEXT;
CREATE UNIQUE INDEX idx_agent_tool_call_event ON agent_tool_call(event_id) WHERE event_id IS NOT NULL;
CREATE INDEX idx_agent_tool_call_turn ON agent_tool_call(turn_id) WHERE turn_id IS NOT NULL;

-- Token rows anchor to their turn; a row counted from a turn's own report
-- (ACP) is keyed by the `agent.turn.ended` event that carried it.
ALTER TABLE agent_token_usage ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE agent_token_usage ADD COLUMN cause TEXT;
CREATE UNIQUE INDEX idx_agent_token_usage_cause ON agent_token_usage(cause) WHERE cause IS NOT NULL;

-- Nudges: the event that fired one (redelivery can't fire it twice), its
-- turn, and when a hook response carried it to the agent (a nudge that
-- misses its hook's window goes out on the thread's next one).
ALTER TABLE agent_nudge ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE agent_nudge ADD COLUMN cause TEXT;
ALTER TABLE agent_nudge ADD COLUMN delivered_at TEXT;
CREATE UNIQUE INDEX idx_agent_nudge_cause_kind ON agent_nudge(cause, kind) WHERE cause IS NOT NULL;
CREATE INDEX idx_agent_nudge_undelivered ON agent_nudge(thread_id, id) WHERE delivered_at IS NULL;
-- Rows written before this version were delivered by the hook that fired them.
UPDATE agent_nudge SET delivered_at = created_at;

-- One-shot marks per effort ("report-less-run", "<extension>/<advisory>",
-- "<extension>/<advisory>#<row key>"): durable, so a restart doesn't
-- repeat guidance the agent already had. Replaces the in-memory sets.
CREATE TABLE effort_once_mark (
    effort_id INTEGER NOT NULL REFERENCES effort(id) ON DELETE CASCADE,
    mark      TEXT    NOT NULL,
    fired_at  TEXT    NOT NULL,
    PRIMARY KEY (effort_id, mark)
) STRICT;

-- Decisions and claims anchor to the turn they were made in.
ALTER TABLE decision ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE claim    ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;

-- Views: each gains its new columns.
DROP VIEW v_event;
CREATE VIEW v_event AS
SELECT seq, id, type, v, at, source,
       stream_id, thread_id, effort_id, turn_id, snapshot_id,
       subject, payload, payload_hash, payload_expired_at, cause, dedupe_key
FROM event_log;

CREATE VIEW v_event_content AS
SELECT hash, namespace, size, created_at FROM event_content;

DROP VIEW v_tool_call;
CREATE VIEW v_tool_call AS
SELECT id, thread_id, effort_id, turn_id, tool, path, detail, ok, at, event_id FROM agent_tool_call;

DROP VIEW v_token_usage;
CREATE VIEW v_token_usage AS
SELECT id, stream_id, thread_id, effort_id, turn_id, agent_kind, model, prompt, input_tokens,
       output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, recorded_at
FROM agent_token_usage;

DROP VIEW v_agent_nudge;
CREATE VIEW v_agent_nudge AS
SELECT id, thread_id, effort_id, turn_id, kind, message, trigger, created_at, delivered_at
FROM agent_nudge;

DROP VIEW v_decision;
CREATE VIEW v_decision AS
SELECT id, thread_id, task_id, effort_id, turn_id, question, choice,
       alternatives_json AS alternatives, confidence, why, provenance, created_at
FROM decision;

DROP VIEW v_claim;
CREATE VIEW v_claim AS
SELECT c.id, c.thread_id, c.task_id, c.effort_id, c.turn_id, c.statement, c.kind,
       c.evidence_ref,
       CASE
         WHEN c.evidence_ref IS NOT NULL THEN 1
         WHEN c.kind = 'tests_pass' AND c.effort_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM (
             SELECT r.failed, r.total FROM v_test_run r
             WHERE r.effort_id = c.effort_id
             ORDER BY r.captured_at DESC, r.id DESC
             LIMIT 1
           ) latest
           WHERE latest.failed = 0 AND latest.total > 0
         ) THEN 1
         ELSE 0
       END AS verified,
       c.created_at
FROM claim c;
