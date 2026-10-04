-- An effort's impacts name their target in the one vocabulary the agent's
-- tools document (`wiki | task | file | directory | git_commit |
-- finding`, tsk920): `effort.report` refuses any other, and the projection
-- reads no alias. Impacts stored under another spelling — a canonical ref
-- kind (`work_item`, `dir`, `commit`) or the old page-ref kind
-- (`git-commit`) — take the documented name. Their ids are unchanged.
UPDATE effort
SET impacts_json = (
    SELECT json_group_array(
        CASE json_extract(value, '$.kind')
            WHEN 'work_item' THEN json_set(value, '$.kind', 'task')
            WHEN 'dir' THEN json_set(value, '$.kind', 'directory')
            WHEN 'commit' THEN json_set(value, '$.kind', 'git_commit')
            WHEN 'git-commit' THEN json_set(value, '$.kind', 'git_commit')
            ELSE json(value)
        END)
    FROM json_each(effort.impacts_json)
)
WHERE json_valid(impacts_json)
  AND json_type(impacts_json) = 'array'
  AND EXISTS (
      SELECT 1 FROM json_each(effort.impacts_json)
      WHERE json_extract(value, '$.kind') IN ('work_item', 'dir', 'commit', 'git-commit')
  );
