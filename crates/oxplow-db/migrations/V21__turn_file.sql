-- The files that changed during each turn, stored: a join by time between a
-- turn and its stream's snapshot line, written when a take captures a file
-- not computed per
-- read (the join over every turn and every captured file took tens of
-- seconds). Written when a take is logged (`snapshot_op`, after its file
-- rows): a take that changed nothing logs none. Several writers can change
-- one worktree at once, so it is best effort.
--
-- A captured file is a turn's when the take captured it after the turn
-- started, while it ran or within a minute after it ended (its end take can
-- land just after it closed), and the watcher saw the file change inside the
-- turn's window; with no change time (the startup sweep), only a capture
-- while the turn ran, or in its own end take, is evidence. `shared`: another turn on the stream ran
-- while it changed — known at capture, since a turn that overlaps a change
-- had started by then.
CREATE TABLE turn_file (
    turn_id INTEGER NOT NULL REFERENCES agent_turn(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    stream_id INTEGER NOT NULL,
    first_changed_at TEXT NOT NULL,
    last_changed_at TEXT NOT NULL,
    timed INTEGER NOT NULL,
    shared INTEGER NOT NULL,
    PRIMARY KEY (turn_id, path)
) WITHOUT ROWID;
CREATE INDEX idx_turn_file_stream_path ON turn_file(stream_id, path, last_changed_at);
-- The turns running or just ended: what a capture is matched against.
CREATE INDEX idx_agent_turn_ended ON agent_turn(ended_at);

CREATE TRIGGER snapshot_op_turn_file AFTER INSERT ON snapshot_op
BEGIN
  INSERT INTO turn_file (turn_id, path, stream_id, first_changed_at, last_changed_at, timed, shared)
  SELECT t.id, f.path, f.stream_id,
         min(coalesce(f.first_changed_at, f.captured_at)),
         max(coalesce(f.last_changed_at, f.captured_at)),
         min(f.first_changed_at IS NOT NULL),
         max(EXISTS (SELECT 1 FROM agent_turn o JOIN threads oth ON oth.id = o.thread_id
                      WHERE oth.stream_id = f.stream_id AND o.id <> t.id
                        AND o.started_at <= coalesce(f.last_changed_at, f.captured_at)
                        AND (o.ended_at IS NULL
                             OR o.ended_at >= coalesce(f.first_changed_at, f.captured_at))))
    FROM file_snapshot f
    JOIN threads th ON th.stream_id = f.stream_id
    JOIN agent_turn t ON t.thread_id = th.id
   WHERE f.snapshot_id = NEW.snapshot_id
     AND t.started_at < f.captured_at
     AND (t.ended_at IS NULL
          OR t.ended_at >= strftime('%Y-%m-%dT%H:%M:%fZ', julianday(f.captured_at) - 60.0 / 86400))
     AND CASE WHEN f.first_changed_at IS NULL
              -- No change time: a capture while it ran, or its own end take.
              THEN t.ended_at IS NULL OR f.captured_at <= t.ended_at
                   OR (NEW.trigger = 'turn_end' AND NEW.turn_id = t.id)
              ELSE f.first_changed_at <= coalesce(t.ended_at, '9999')
                   AND f.last_changed_at >= t.started_at END
   GROUP BY t.id, f.path
  ON CONFLICT (turn_id, path) DO UPDATE SET
    first_changed_at = min(first_changed_at, excluded.first_changed_at),
    last_changed_at = max(last_changed_at, excluded.last_changed_at),
    timed = min(timed, excluded.timed),
    shared = max(shared, excluded.shared);
END;

-- What the snapshot line already holds, once.
INSERT INTO turn_file (turn_id, path, stream_id, first_changed_at, last_changed_at, timed, shared)
SELECT g.turn_id, g.path, g.stream_id, g.first_changed_at, g.last_changed_at, g.timed,
       EXISTS (SELECT 1 FROM agent_turn o JOIN threads oth ON oth.id = o.thread_id
                WHERE oth.stream_id = g.stream_id AND o.id <> g.turn_id
                  AND o.started_at <= g.last_changed_at
                  AND (o.ended_at IS NULL OR o.ended_at >= g.first_changed_at))
  FROM (SELECT t.id AS turn_id, f.path, th.stream_id,
               min(coalesce(f.first_changed_at, f.captured_at)) AS first_changed_at,
               max(coalesce(f.last_changed_at, f.captured_at)) AS last_changed_at,
               min(f.first_changed_at IS NOT NULL) AS timed
          FROM agent_turn t
          JOIN threads th ON th.id = t.thread_id
          JOIN file_snapshot f
            ON f.stream_id = th.stream_id
           AND f.captured_at > t.started_at
           AND (t.ended_at IS NULL
                OR f.captured_at <= strftime('%Y-%m-%dT%H:%M:%fZ', julianday(t.ended_at) + 60.0 / 86400))
           AND CASE WHEN f.first_changed_at IS NULL
                    THEN t.ended_at IS NULL OR f.captured_at <= t.ended_at
                         OR EXISTS (SELECT 1 FROM snapshot_op o
                                     WHERE o.snapshot_id = f.snapshot_id
                                       AND o.trigger = 'turn_end' AND o.turn_id = t.id)
                    ELSE f.first_changed_at <= coalesce(t.ended_at, '9999')
                         AND f.last_changed_at >= t.started_at END
         GROUP BY t.id, f.path) g;
