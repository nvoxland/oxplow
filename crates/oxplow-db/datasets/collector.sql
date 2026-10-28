SELECT c.uri, c.extension, c.local AS id,
       CAST(json_extract(c.runs_on, '$.on') AS TEXT) AS trigger,
       CAST(h.state IS NOT 'disabled' AS INTEGER) AS enabled,
       CAST(CASE
              WHEN h.state = 'disabled' THEN 'disabled'
              WHEN h.state = 'failing' OR r.status = 'error' THEN 'failing'
              WHEN r.owner IS NULL THEN 'never_run'
              ELSE 'ok'
            END AS TEXT) AS state,
       r.last_run_at, r.status AS last_status, r.error, r.row_counts_json,
       h.reason AS disabled_reason, h.next_due_at, h.fresh
FROM ref('contribution') c
LEFT JOIN ref('collector_run') r ON r.uri = c.uri
LEFT JOIN ref('contribution_health') h ON h.uri = c.uri
WHERE c.kind = 'collector'
