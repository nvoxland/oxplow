-- What a running job produced as it went: a streamed model reply's text,
-- the tool calls it made and what they answered, each round's usage. Rows
-- belong to one attempt and are only appended while it is open, so a
-- worker whose attempt was closed (cancelled, expired, replaced) cannot
-- add to it.
CREATE TABLE extension_job_output (
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'tool_call', 'tool_result', 'usage')),
    at_ms INTEGER NOT NULL CHECK (at_ms >= 0),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)
        AND length(CAST(content_json AS BLOB)) <= 262144),
    PRIMARY KEY (job_id, attempt, seq),
    FOREIGN KEY (job_id, attempt, fence)
        REFERENCES extension_job_attempt(job_id, attempt, fence)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER extension_job_output_append_only
BEFORE UPDATE ON extension_job_output
BEGIN
    SELECT RAISE(ABORT, 'extension job output is append-only');
END;

CREATE TRIGGER extension_job_output_open_attempt
BEFORE INSERT ON extension_job_output
WHEN NOT EXISTS (
    SELECT 1 FROM extension_job_attempt
     WHERE job_id = NEW.job_id AND attempt = NEW.attempt AND fence = NEW.fence
       AND closed_at_ms IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'extension job attempt is not open');
END;
