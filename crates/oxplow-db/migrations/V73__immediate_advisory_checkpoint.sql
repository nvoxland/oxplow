-- The new consumer takes over part of the old advisory stream. Start it
-- at the same checkpoint so an upgrade cannot remind agents about old work.
INSERT INTO event_consumer_checkpoint (consumer, last_seq, updated_at)
SELECT 'advisories.post_tool.immediate', last_seq, updated_at
FROM event_consumer_checkpoint
WHERE consumer = 'advisories.post_tool';
