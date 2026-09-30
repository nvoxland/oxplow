-- P5.E1: recorded AI computations. A classify / score / summarize /
-- extract result is kept by the hash of its input, the model and the
-- prompt version, so the same computation is never paid for twice; an
-- ai_call row says which call produced it (tokens only — no cost).
ALTER TABLE ai_call ADD COLUMN input_hash TEXT;

CREATE TABLE ai_result (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_hash TEXT NOT NULL,
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
    UNIQUE (input_hash, model, prompt_version)
);
