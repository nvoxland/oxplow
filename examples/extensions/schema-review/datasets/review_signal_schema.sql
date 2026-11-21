-- The change review's signal for a changed migration, from the DDL the
-- `ddl` collector found in it as of the change's head (`v_change_fact`): a
-- migration that drops a table is `high`. Any other changed migration is
-- named with no severity of its own, which the review reads as `medium`.
SELECT f.change_id,
       'schema_review/review_signal_schema' AS source,
       'code' AS area,
       CASE WHEN sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'drop') > 0
            THEN 'drops-table' ELSE 'schema-changed' END AS signal,
       CASE WHEN sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'drop') > 0
            THEN 'high' END AS severity,
       f.path,
       NULL AS line,
       NULL AS end_line,
       CASE WHEN sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'drop') > 0
            THEN 'Migration drops a table' ELSE 'Migration changed' END AS title,
       printf('It creates %d, alters %d and drops %d tables.',
              coalesce(sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'create'), 0),
              coalesce(sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'alter'), 0),
              coalesce(sum(f.value) FILTER (WHERE json_extract(f.dims_json, '$."schema_review.ddl_kind"') = 'drop'), 0)) AS why,
       NULL AS uri
FROM ref('change_fact') f
WHERE f.measure_key = 'schema_review.ddl'
GROUP BY f.change_id, f.path
