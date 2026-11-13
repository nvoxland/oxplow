-- The files each tool call named or the harness reported it changed, one
-- row per (call, path): `named` by an edit tool's own input, `reported` by
-- the harness's statement that a shell call changed it.
SELECT CAST(p.call_id AS INT) AS call_id, CAST(c.turn_id AS INT) AS turn_id,
       CAST(p.path AS TEXT) AS path, CAST(p.basis AS TEXT) AS basis
  FROM source('agent_tool_call_path') p
  JOIN source('agent_tool_call') c ON c.id = p.call_id
