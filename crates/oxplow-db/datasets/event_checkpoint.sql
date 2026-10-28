SELECT consumer, last_seq, updated_at
FROM source('event_consumer_checkpoint')
