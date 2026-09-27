-- Every model call oxplow itself makes (tsk299): who asked, which role /
-- provider / model, tokens, latency, outcome. AI usage is thereby itself
-- semantic-layer data.
CREATE TABLE ai_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    -- What asked: `mcp:ai_decide`, `source:<ext>/<id>`, `inferred-decisions`, …
    caller TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    ok INTEGER NOT NULL,
    error TEXT,
    at TEXT NOT NULL
);
CREATE INDEX idx_ai_call_role_at ON ai_call(role, at);

CREATE VIEW v_ai_call AS
SELECT id, role, provider, model, caller, input_tokens, output_tokens,
       latency_ms, ok, error, at
FROM ai_call;
