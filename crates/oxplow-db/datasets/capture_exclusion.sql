-- `unchecked`: no later capture of the same producer and stream has scanned
-- the file since (a later one that also left it out is its own row), so the
-- facts it last had are still the last good ones, unchecked.
SELECT x.capture_id AS capture_id, c.producer AS producer, x.path AS path,
       x.reason AS reason,
       CAST(NOT EXISTS (
         SELECT 1
           FROM source('metric_capture') c2
          WHERE c2.stream_id = c.stream_id
            AND c2.producer = c.producer
            AND c2.status = 'done'
            AND (c2.captured_at > c.captured_at
                 OR (c2.captured_at = c.captured_at AND c2.id > c.id))
            AND (c2.scan_kind = 'full'
                 OR EXISTS (
                   SELECT 1 FROM source('file_snapshot') sf
                    WHERE sf.snapshot_id = c2.snapshot_id
                      AND sf.stream_id = c2.stream_id
                      AND sf.path = x.path))
       ) AS INT) AS unchecked
FROM source('metric_capture_exclusion') x
JOIN source('metric_capture') c ON c.id = x.capture_id
