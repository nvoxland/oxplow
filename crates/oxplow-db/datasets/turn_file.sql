-- The files that changed during each turn: the stored join by time between
-- the turn and its stream's snapshot line (`turn_file`, written as takes
-- capture files; V21 has the rule). Several writers can change one worktree
-- at once, so it is best effort, and each file says how sure it is. A plain
-- select over the table, so a filter on a turn, a request's turns or a
-- stream and path reaches its index.
SELECT f.turn_id,
       CAST(uri('oxplow/turn', 'id', 'trn' || f.turn_id) AS TEXT) AS turn,
       f.stream_id, f.path, f.first_changed_at, f.last_changed_at,
       CAST(f.timed AS INT) AS timed,
       -- A call of the turn named or reported it.
       CAST((SELECT p.basis FROM source('agent_tool_call') c
               JOIN source('agent_tool_call_path') p ON p.call_id = c.id
              WHERE c.turn_id = f.turn_id AND p.path = f.path
              ORDER BY p.basis = 'named' DESC LIMIT 1) AS TEXT) AS claim,
       CAST(CASE
         WHEN EXISTS (SELECT 1 FROM source('agent_tool_call') c
                        JOIN source('agent_tool_call_path') p ON p.call_id = c.id
                       WHERE c.turn_id = f.turn_id AND p.path = f.path) THEN 'claimed'
         -- Another turn on the stream ran while it changed.
         WHEN f.shared THEN 'shared'
         ELSE 'sole' END AS TEXT) AS confidence
  FROM source('turn_file') f
