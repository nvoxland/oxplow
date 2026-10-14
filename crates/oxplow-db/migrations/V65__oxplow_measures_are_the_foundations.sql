-- oxplow's own measures and dimensions are declared by the oxplow-foundation
-- extension (its `measures:` / `dimensions:`), like any extension's, so none
-- has the `built-in` scope any more. The rows stay (deleting a measure
-- cascades its facts): they become what an extension's rows are stored as,
-- `scope = 'global'` plus the `extension` column, and the foundation's first
-- activation restates them from its manifest.
UPDATE measure SET scope = 'global', extension = 'oxplow-foundation' WHERE scope = 'built-in';
UPDATE dimension SET scope = 'global', extension = 'oxplow-foundation' WHERE scope = 'built-in';
