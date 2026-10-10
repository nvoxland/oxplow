-- When the person deliberately stopped an agent session. Set only by a
-- person's stop; a crash or a backend restart leaves it NULL, so the
-- agent resumes when its tab is shown. Cleared by starting it again.
ALTER TABLE agent_session ADD COLUMN stopped_by_user_at TEXT;
