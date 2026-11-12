-- A send that asked to be told when its recipient is idle (`notify_when_idle`)
-- says so on its own row, and the idle notice that answers it is recorded on
-- that row too: `notify_when_idle` is the ask, `idle_notice_turn_id` the turn
-- the notice joined (NULL until it arrives, or when it never matched).
-- The ask used to be a second row carrying a marker in `text_hash`; those
-- rows are folded into their sends and removed.
ALTER TABLE agent_message ADD COLUMN notify_when_idle INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_message ADD COLUMN idle_notice_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;

UPDATE agent_message
   SET notify_when_idle = 1,
       idle_notice_turn_id = (
         SELECT k.received_turn_id FROM agent_message k
          WHERE k.text_hash = 'oxplow:agent_message:notify_when_idle'
            AND k.thread_id = agent_message.thread_id
            AND k.call_id IS agent_message.call_id
            AND k.agent_session_id IS agent_message.agent_session_id
            AND k.recipient = agent_message.recipient
          ORDER BY k.id LIMIT 1)
 WHERE text_hash <> 'oxplow:agent_message:notify_when_idle'
   AND EXISTS (
         SELECT 1 FROM agent_message k
          WHERE k.text_hash = 'oxplow:agent_message:notify_when_idle'
            AND k.thread_id = agent_message.thread_id
            AND k.call_id IS agent_message.call_id
            AND k.agent_session_id IS agent_message.agent_session_id
            AND k.recipient = agent_message.recipient);

DELETE FROM agent_message WHERE text_hash = 'oxplow:agent_message:notify_when_idle';
