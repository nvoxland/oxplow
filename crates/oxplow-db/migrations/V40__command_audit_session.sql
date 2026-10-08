-- The agent session a run came from. Every agent session has its own
-- bearer, and the control plane takes the caller from it alone, so a run
-- over MCP knows its session: two sessions on one thread are two actors.
-- NULL for a person's, a lens's for a person, oxplow's own, an effect's,
-- and an agent's run that came through no session.
ALTER TABLE command_audit ADD COLUMN session_id INTEGER;
