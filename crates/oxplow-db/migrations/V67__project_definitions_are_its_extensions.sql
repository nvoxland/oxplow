-- The project's own collectors, measures, dimensions and metrics are its
-- own extension's (`oxplow/extensions/project/`), no longer
-- `.oxplow/project.yaml`'s, so none has the `project` scope any more. The
-- rows stay (deleting a measure cascades its facts): they become what an
-- extension's rows are stored as, `scope = 'global'` plus the `extension`
-- column, and the extension's activation restates them from its manifest.
-- Collector runs and health already name the owner `project`, the
-- extension's name.
UPDATE measure SET scope = 'global', extension = 'project' WHERE scope = 'project';
UPDATE dimension SET scope = 'global', extension = 'project' WHERE scope = 'project';
UPDATE metric_spec SET scope = 'global', extension = 'project' WHERE scope = 'project';
UPDATE metric_capture SET scope = 'extension:project' WHERE scope = 'project';
