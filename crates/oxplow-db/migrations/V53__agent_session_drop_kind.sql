-- A session's kind was `terminal` or `action`, a one-shot session on a
-- programmatic surface that was never built. Every session is a terminal
-- agent, so the column goes. The rows stay.
ALTER TABLE agent_session DROP COLUMN kind;
