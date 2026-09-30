-- tsk560: a recorded AI computation is the provider's as well as the
-- model's — two providers serving the same model name (a local llama3.1
-- and a hosted one) are two computations, so rebinding a role to another
-- provider computes afresh. SQLite can't change a UNIQUE constraint in
-- place, so the table is rebuilt; each existing row takes the provider of
-- the call that computed it, and a row whose call is gone is dropped (it
-- is a memo: dropping it costs one recompute).
CREATE TABLE ai_result_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    op TEXT NOT NULL,
    role TEXT NOT NULL,
    caller TEXT NOT NULL,
    output_json TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    at TEXT NOT NULL,
    ai_call_id INTEGER REFERENCES ai_call(id) ON DELETE SET NULL,
    UNIQUE (input_hash, provider, model, prompt_version)
);

INSERT INTO ai_result_next (id, input_hash, provider, model, prompt_version, op, role, caller,
                            output_json, input_tokens, output_tokens, at, ai_call_id)
SELECT r.id, r.input_hash, c.provider, r.model, r.prompt_version, r.op, r.role, r.caller,
       r.output_json, r.input_tokens, r.output_tokens, r.at, r.ai_call_id
FROM ai_result r JOIN ai_call c ON c.id = r.ai_call_id;

DROP TABLE ai_result;
ALTER TABLE ai_result_next RENAME TO ai_result;
