-- Blocks the change copied: a duplicated block of a changed file that
-- overlaps a line the change touched (`change_hunk`). A changed file's
-- copies the change didn't touch were there before, so they are no signal
-- of it. One row per block, with every place it repeats in `related`
-- (the scan gives one row per pair). The scan arrives after the rest of the
-- analysis, so these rows come late.
-- One aggregate over the table itself, the overlap an EXISTS: a subquery
-- under the GROUP BY (or a join that repeats a pair per hunk) kept
-- `WHERE change_id = ?` out, and every change's blocks were read
-- (`review_plan_tests`).
SELECT d.change_id, 'oxplow/review_signal_duplication' AS source, 'code' AS area,
       'duplicated-block' AS signal, 'info' AS severity, d.path,
       d.start_line AS line, d.end_line AS end_line, 'Duplicated block' AS title,
       printf('%d lines that repeat code in %d place%s.', d.end_line - d.start_line + 1, count(*),
              CASE count(*) WHEN 1 THEN '' ELSE 's' END) AS why,
       NULL AS uri,
       json_group_array(json_object(
         'text', printf('%s:%d-%d', d.peer_path, d.peer_start_line, d.peer_end_line),
         'uri', uri('oxplow/file', 'path', d.peer_path) || printf('#lines=%d-%d', d.peer_start_line, d.peer_end_line)
       ) ORDER BY d.peer_path, d.peer_start_line) AS related
FROM ref('change_duplicate') d
WHERE EXISTS (SELECT 1 FROM ref('change_hunk') hk
              WHERE hk.change_id = d.change_id AND hk.path = d.path
                AND d.start_line <= hk.head_end AND d.end_line >= hk.head_start)
GROUP BY d.change_id, d.path, d.start_line, d.end_line
