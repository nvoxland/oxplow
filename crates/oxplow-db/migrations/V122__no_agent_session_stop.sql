-- A person no longer stops a session and keeps its slot: closing the tab
-- ends it and Earlier sessions brings it back. The mark leaves the table
-- (.context/data-model.md "agent_session").
ALTER TABLE agent_session DROP COLUMN stopped_by_user_at;
