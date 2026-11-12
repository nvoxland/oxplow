-- The project's task workflow and definition of done, as set (the
-- defaults when nothing is): one row.
SELECT CAST(config('/tasks/workflow') AS TEXT) AS workflow,
       CAST(coalesce(config('/tasks/definitionOfDone'), '') AS TEXT) AS definition_of_done
