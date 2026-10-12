SELECT o.job_id,
       o.attempt,
       o.seq,
       o.kind,
       CAST(strftime('%Y-%m-%dT%H:%M:%fZ', o.at_ms / 1000.0, 'unixepoch') AS TEXT) AS at,
       CAST(CASE o.kind WHEN 'text' THEN json_extract(o.content_json, '$.text') END AS TEXT) AS text,
       o.content_json AS data
FROM source('extension_job_output') o
