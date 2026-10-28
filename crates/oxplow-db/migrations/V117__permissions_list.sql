-- An extension's permissions are a list of scope ids now, in a script's
-- `needs:` grammar: `{ sql: read, process: true, network: [h] }` is
-- `["sql.read", "process.run", "network:h"]`. What deployments and
-- listings stored as the map is rewritten once.
UPDATE extension_deployment SET permissions_json = (
    SELECT json_group_array(id) FROM (
        SELECT 'sql.read' AS id WHERE json_extract(permissions_json, '$.sql') = 'read'
        UNION ALL SELECT 'files.' || json_extract(permissions_json, '$.files')
            WHERE json_extract(permissions_json, '$.files') IN ('read', 'write')
        UNION ALL SELECT 'ai.compute' WHERE json_extract(permissions_json, '$.ai') = 1
        UNION ALL SELECT 'process.run' WHERE json_extract(permissions_json, '$.process') = 1
        UNION ALL SELECT 'agent.write' WHERE json_extract(permissions_json, '$.agent') = 1
        UNION ALL SELECT 'network:' || value FROM json_each(permissions_json, '$.network')
        UNION ALL SELECT 'secrets:' || value FROM json_each(permissions_json, '$.secrets')
        UNION ALL SELECT 'commands:' || value FROM json_each(permissions_json, '$.commands')
    )
)
WHERE json_type(permissions_json) = 'object';

UPDATE extension_listing SET permissions_json = (
    SELECT json_group_array(id) FROM (
        SELECT 'sql.read' AS id WHERE json_extract(permissions_json, '$.sql') = 'read'
        UNION ALL SELECT 'files.' || json_extract(permissions_json, '$.files')
            WHERE json_extract(permissions_json, '$.files') IN ('read', 'write')
        UNION ALL SELECT 'ai.compute' WHERE json_extract(permissions_json, '$.ai') = 1
        UNION ALL SELECT 'process.run' WHERE json_extract(permissions_json, '$.process') = 1
        UNION ALL SELECT 'agent.write' WHERE json_extract(permissions_json, '$.agent') = 1
        UNION ALL SELECT 'network:' || value FROM json_each(permissions_json, '$.network')
        UNION ALL SELECT 'secrets:' || value FROM json_each(permissions_json, '$.secrets')
        UNION ALL SELECT 'commands:' || value FROM json_each(permissions_json, '$.commands')
    )
)
WHERE json_type(permissions_json) = 'object';
