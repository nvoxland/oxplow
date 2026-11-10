-- The files that changed during each turn: a join by time between the turn
-- and its stream's snapshot line, not snapshots the turn owns. Several
-- writers can change one worktree at once, so it is best effort, and each
-- file says how sure it is.
--
-- A file belongs to a turn when a take captured it after the turn started
-- (and no later than a minute after it ended) and either the watcher saw
-- it change inside the turn's window, or, with no change time (a change the
-- startup sweep found), the capture alone is the evidence. A file changed
-- before the prompt but captured by the turn's own end take is not the
-- turn's.
SELECT g.turn_id, g.turn, g.stream_id, g.path, g.first_changed_at, g.last_changed_at,
       g.timed, g.claim,
       CAST(CASE
         WHEN g.claim IS NOT NULL THEN 'claimed'
         -- Another turn on the stream ran while it changed.
         WHEN EXISTS (SELECT 1 FROM source('agent_turn') o
                        JOIN source('threads') oth ON oth.id = o.thread_id
                       WHERE oth.stream_id = g.stream_id AND o.id <> g.turn_id
                         AND o.started_at <= g.last_changed_at
                         AND (o.ended_at IS NULL OR o.ended_at >= g.first_changed_at))
           THEN 'shared'
         ELSE 'sole' END AS TEXT) AS confidence
  FROM (SELECT t.id AS turn_id,
               CAST(uri('oxplow/turn', 'id', 'trn' || t.id) AS TEXT) AS turn,
               th.stream_id, f.path,
               CAST(min(coalesce(f.first_changed_at, f.captured_at)) AS TEXT) AS first_changed_at,
               CAST(max(coalesce(f.last_changed_at, f.captured_at)) AS TEXT) AS last_changed_at,
               CAST(min(f.first_changed_at IS NOT NULL) AS INT) AS timed,
               -- A call of the turn named or reported it.
               CAST((SELECT p.basis FROM source('agent_tool_call') c
                       JOIN source('agent_tool_call_path') p ON p.call_id = c.id
                      WHERE c.turn_id = t.id AND p.path = f.path
                      ORDER BY p.basis = 'named' DESC LIMIT 1) AS TEXT) AS claim
          FROM source('agent_turn') t
          JOIN source('threads') th ON th.id = t.thread_id
          JOIN source('file_snapshot') f
            ON f.stream_id = th.stream_id
           AND f.captured_at > t.started_at
           AND (t.ended_at IS NULL
                OR f.captured_at <= strftime('%Y-%m-%dT%H:%M:%fZ', julianday(t.ended_at) + 60.0 / 86400))
           AND (f.first_changed_at IS NULL
                OR (f.first_changed_at <= coalesce(t.ended_at, '9999')
                    AND f.last_changed_at >= t.started_at))
         GROUP BY t.id, f.path) g
