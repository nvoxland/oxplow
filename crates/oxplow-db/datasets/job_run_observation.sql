SELECT o.id,
       o.job_id,
       o.attempt,
       o.fence,
       CAST(strftime('%Y-%m-%dT%H:%M:%fZ', o.observed_at_ms / 1000.0, 'unixepoch') AS TEXT) AS at,
       o.outcome,
       o.evidence_json AS evidence
FROM source('extension_job_observation') o
