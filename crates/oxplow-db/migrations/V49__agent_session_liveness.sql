-- Which process runs an agent session, and when it last spoke.
-- `pid` is the OS pid of the process the daemon spawned for the session
-- (rewritten on every spawn); `last_seen_at` is the newest hook seen from
-- it. Together they tell a live agent from a dead or reused pid.
ALTER TABLE agent_session ADD COLUMN pid INTEGER;
ALTER TABLE agent_session ADD COLUMN last_seen_at TEXT;
