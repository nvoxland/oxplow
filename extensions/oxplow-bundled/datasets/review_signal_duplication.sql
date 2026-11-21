-- Blocks of the change's files that copy code elsewhere in the tree. The
-- scan arrives after the rest of the analysis, so these rows come late.
SELECT change_id, 'oxplow/review_signal_duplication' AS source, 'code' AS area,
       'duplicated-block' AS signal, 'info' AS severity, path,
       start_line AS line, end_line AS end_line, 'Duplicated block' AS title,
       printf('%d lines copy %s:%d-%d.', lines, peer_path, peer_start_line, peer_end_line) AS why,
       NULL AS uri
FROM ref('change_duplicate')
