SELECT owner, id, event_seq, since, touched
FROM source('collector_pending')
