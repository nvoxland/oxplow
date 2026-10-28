SELECT effort_id, seq, kind, provenance, source, metric_value, payload_json,
       local_snapshot_id, created_at
FROM source('effort_observation_row')
