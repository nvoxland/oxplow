-- The runs a request's turns made on one stream (tests, diff coverage,
-- static analysis), as its stored analysis computed them, oldest first.
SELECT o.request_id,
       CAST(uri('oxplow/request', 'id', 'req' || o.request_id) AS TEXT) AS request,
       o.stream_id, CAST(o.seq AS INT) AS seq, o.kind, o.provenance, o.source, o.metric_value,
       o.payload_json, CAST(o.local_snapshot_id AS INT) AS local_snapshot_id, o.created_at
  FROM source('request_observation') o
