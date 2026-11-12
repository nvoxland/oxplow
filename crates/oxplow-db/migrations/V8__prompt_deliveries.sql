-- One event per prompt delivery: agent.prompt.submitted@3 carries the
-- delivery's source; agent.peer_message.received folds into it.
--
-- @2 rows were logged for every prompt that was not a hand-back or another
-- session's message (a person's, and a background job's report): their
-- source is not known from the row, so `unknown`.
UPDATE event_log
   SET payload = json_set(payload, '$.source', 'unknown')
 WHERE type = 'agent.prompt.submitted' AND v = 2 AND payload_expired_at IS NULL;
UPDATE event_log SET v = 3 WHERE type = 'agent.prompt.submitted' AND v = 2;

-- A peer's message becomes the delivery it was, with its sender.
UPDATE event_log
   SET type = 'agent.prompt.submitted', v = 3,
       payload = json_object(
         'thread', json_extract(payload, '$.thread'),
         'turn', json_extract(payload, '$.turn'),
         'reprompt', json('false'),
         'source', 'peer',
         'sender', json_extract(payload, '$.from'))
 WHERE type = 'agent.peer_message.received' AND payload_expired_at IS NULL;
UPDATE event_log SET type = 'agent.prompt.submitted', v = 3
 WHERE type = 'agent.peer_message.received';
