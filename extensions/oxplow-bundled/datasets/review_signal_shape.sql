-- Large or complex files in the change, from the "look here first" score's
-- reasons (`v_oxplow_change_interest`): a routine file has none and no row.
SELECT change_id, 'oxplow/review_signal_shape' AS source, 'code' AS area,
       'shape' AS signal, 'info' AS severity, path, NULL AS line, NULL AS end_line,
       'Large or complex change' AS title, reasons AS why, NULL AS uri
FROM ref('change_interest')
WHERE reasons <> ''
