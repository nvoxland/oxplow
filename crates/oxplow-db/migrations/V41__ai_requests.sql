-- A model call keeps what it was asked and what it answered, by content
-- hash into `event_content` (namespace `ai`, under retention): replayable
-- and inspectable while the body is kept.
ALTER TABLE ai_call ADD COLUMN request_hash TEXT;
ALTER TABLE ai_call ADD COLUMN response_hash TEXT;

-- A recorded result is keyed by its request's hash, so a changed prompt
-- is a new result without anyone bumping a version. A result recorded
-- under a hand-bumped version is never read again: it goes.
DELETE FROM ai_result;
ALTER TABLE ai_result RENAME COLUMN prompt_version TO request_hash;
