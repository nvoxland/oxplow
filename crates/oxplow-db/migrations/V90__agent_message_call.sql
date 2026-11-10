-- The tool call a message was sent by: its request's call id in the sending
-- session. A match reads that call's state, so a send that was denied (it
-- never ran) or failed is not a candidate for a received message.
ALTER TABLE agent_message ADD COLUMN call_id TEXT;
CREATE INDEX idx_agent_message_call ON agent_message(agent_session_id, call_id);
