-- The links of every store's tasks, in today's `v_task_link` shape, over the
-- shared model's links (TEMPORARY name: the switch renames it `task_link`).
-- `to_kind` is the target's resource kind.
SELECT l.parent AS from_uri, l."to" AS to_uri,
       CAST(CASE WHEN l."to" LIKE 'oxplow:%'
                 THEN substr(l."to", 8, instr(substr(l."to", 8) || '?', '?') - 1) END AS TEXT) AS to_kind,
       l.type AS link_type, CAST(NULL AS TEXT) AS created_at
FROM ref('task_record_links') l
