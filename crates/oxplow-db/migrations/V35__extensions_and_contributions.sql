-- oxplow's word for a package is "extension" and for one thing an
-- extension declares "contribution" (`.context/extensions.md`). What the
-- old names left in the data is rewritten, once: the tables, the command,
-- the event types and their payloads, the repair consumer's checkpoint
-- and the `extension:<name>` refs.

-- Each contribution's health, keyed by its extension.
ALTER TABLE plugin_health RENAME TO contribution_health;
ALTER TABLE contribution_health RENAME COLUMN plugin TO extension;

-- The windows extensions declare for their namespaces.
ALTER TABLE plugin_event_retention RENAME TO extension_event_retention;

-- The command a person enables a contribution with, its input's
-- `extension` and the health row it answered with.
UPDATE command_audit
   SET command = 'oxplow.contribution.enable',
       input_json = CASE WHEN json_valid(input_json)
                              AND json_type(input_json, '$.plugin') IS NOT NULL
                         THEN json_set(json_remove(input_json, '$.plugin'), '$.extension',
                                       json_extract(input_json, '$.plugin'))
                         ELSE input_json END,
       result_json = CASE WHEN json_valid(result_json)
                               AND json_type(result_json, '$.plugin') IS NOT NULL
                          THEN json_set(json_remove(result_json, '$.plugin'), '$.extension',
                                        json_extract(result_json, '$.plugin'))
                          ELSE result_json END
 WHERE command = 'oxplow.plugin.enable';
UPDATE command_proposal
   SET command = 'oxplow.contribution.enable',
       input_json = CASE WHEN json_valid(input_json)
                              AND json_type(input_json, '$.plugin') IS NOT NULL
                         THEN json_set(json_remove(input_json, '$.plugin'), '$.extension',
                                       json_extract(input_json, '$.plugin'))
                         ELSE input_json END
 WHERE command = 'oxplow.plugin.enable';
-- Anywhere else a stored call names it (a sequence's steps).
UPDATE command_audit
   SET input_json = replace(input_json, '"oxplow.plugin.enable"', '"oxplow.contribution.enable"'),
       inverse_json = replace(inverse_json, '"oxplow.plugin.enable"', '"oxplow.contribution.enable"')
 WHERE input_json LIKE '%"oxplow.plugin.enable"%'
    OR inverse_json LIKE '%"oxplow.plugin.enable"%';
UPDATE command_proposal
   SET input_json = replace(input_json, '"oxplow.plugin.enable"', '"oxplow.contribution.enable"'),
       preview_json = replace(preview_json, '"oxplow.plugin.enable"', '"oxplow.contribution.enable"')
 WHERE input_json LIKE '%"oxplow.plugin.enable"%'
    OR preview_json LIKE '%"oxplow.plugin.enable"%';
UPDATE event_log
   SET payload = replace(payload, '"oxplow.plugin.enable"', '"oxplow.contribution.enable"')
 WHERE type LIKE 'command.%' AND payload LIKE '%"oxplow.plugin.enable"%';

-- `plugin.disabled@1` / `plugin.enabled@1` are `contribution.*@1`, their
-- `plugin` the `extension` (an `extension:<name>` ref).
UPDATE event_log
   SET type = 'contribution.' || substr(type, length('plugin.') + 1),
       payload = CASE WHEN json_type(payload, '$.plugin') IS NOT NULL
                      THEN json_set(json_remove(payload, '$.plugin'), '$.extension',
                                    'extension:' || substr(json_extract(payload, '$.plugin'),
                                                           length('plugin:') + 1))
                      ELSE payload END
 WHERE type IN ('plugin.disabled', 'plugin.enabled');
DELETE FROM event_type_contract
 WHERE event_type IN ('plugin.disabled', 'plugin.enabled') AND extension IS NULL;
UPDATE event_log SET source = 'system:extensions' WHERE source = 'system:plugins';

-- The `plugin:<name>` ref kind is `extension:<name>`.
UPDATE event_log
   SET subject = replace(subject, '"plugin:', '"extension:')
 WHERE subject LIKE '%"plugin:%';
UPDATE page_ref SET source_kind = 'extension' WHERE source_kind = 'plugin';
UPDATE page_ref SET target_kind = 'extension' WHERE target_kind = 'plugin';
UPDATE comment SET target_kind = 'extension' WHERE target_kind = 'plugin';
UPDATE bookmark
   SET ref = 'extension:' || substr(ref, length('plugin:') + 1)
 WHERE ref LIKE 'plugin:%';

-- The repair consumer keeps its place in the log and its dead letters.
UPDATE event_consumer_checkpoint SET consumer = 'contribution.repair'
 WHERE consumer = 'plugin.repair';
UPDATE event_dead_letter SET consumer = 'contribution.repair'
 WHERE consumer = 'plugin.repair';

-- What an `exec` collector's program produced is tagged `exec:<ids>`;
-- `test.run.recorded@2` names it so (its shape is v1's).
UPDATE metric_capture
   SET source = 'exec:' || substr(source, length('plugin-exec:') + 1)
 WHERE source LIKE 'plugin-exec:%';
UPDATE effort_observation_row
   SET source = 'exec:' || substr(source, length('plugin-exec:') + 1)
 WHERE source LIKE 'plugin-exec:%';
UPDATE event_log
   SET v = 2,
       payload = CASE WHEN json_extract(payload, '$.source') LIKE 'plugin-exec:%'
                      THEN json_set(payload, '$.source',
                                    'exec:' || substr(json_extract(payload, '$.source'),
                                                      length('plugin-exec:') + 1))
                      ELSE payload END
 WHERE type = 'test.run.recorded' AND v = 1;
DELETE FROM event_type_contract
 WHERE event_type = 'test.run.recorded' AND v = 1 AND extension IS NULL;
