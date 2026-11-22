SELECT c.uri, c.extension, c.namespace, c.local AS id, c.description,
       CAST(json_extract(c.runs_on, '$.on') AS TEXT) AS trigger,
       CAST(h.state IS NOT 'disabled' AS INTEGER) AS enabled,
       CAST(CASE
              WHEN h.state = 'disabled' THEN 'disabled'
              WHEN h.state = 'failing'
                OR r.outcome IN ('failed', 'denied', 'cancelled', 'interrupted', 'invalid') THEN 'failing'
              WHEN r.run_id IS NULL THEN 'never_run'
              ELSE 'ok'
            END AS TEXT) AS state,
       r.finished_at AS last_run_at, r.outcome AS last_outcome, r.error, p.row_counts_json,
       h.reason AS disabled_reason, h.next_due_at, h.fresh
FROM ref('contribution') c
LEFT JOIN source('collector_checkpoint') p ON p.owner = c.namespace AND p.id = c.local
LEFT JOIN source('contribution_run') r ON r.run_id = p.last_run_id
LEFT JOIN ref('contribution_health') h ON h.uri = c.uri
WHERE c.kind = 'collector'
