-- The files that changed during each turn, stored: a join by time between a
-- turn and its stream's snapshot line, not computed per read (the join over
-- every turn and every captured file took tens of seconds). Written when a
-- take is logged (`snapshot_op`, after its file rows): a take that changed
-- nothing logs none. Several writers can change one worktree at once, so it
-- is best effort.
--
-- A captured file is a turn's when the take captured it after the turn
-- started, while it ran or within a minute after it ended (its end take can
-- land just after it closed), and the watcher saw the file change inside the
-- turn's window; with no change time (the startup sweep), only a capture
-- while the turn ran, or in its own end take, is evidence. `shared`: another
-- turn on the stream ran while it changed — for a capture with no change
-- time, while the minute before the capture (another turn's end take lands
-- up to a minute after it closed). Known at capture, since a turn that
-- overlaps a change had started by then. An ended turn that started more
-- than a day before the change isn't looked for (a running one always is).
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
-- The turns running or just ended: what a take's files are matched against.
CREATE INDEX idx_agent_turn_ended ON agent_turn(ended_at);

-- A take's files against its candidate turns: the stream's turns running,
-- or ended since a minute before the take began (its op is logged after its
-- captures; `elapsed_ms` is how long they took), found once per take
-- through `idx_agent_turn_ended` (an `OR` there scans every turn).
CREATE TRIGGER snapshot_op_turn_file AFTER INSERT ON snapshot_op
BEGIN
  INSERT INTO turn_file (turn_id, path, stream_id, first_changed_at, last_changed_at, timed, shared)
  SELECT t.id, f.path, f.stream_id,
         min(coalesce(f.first_changed_at, f.captured_at)),
         max(coalesce(f.last_changed_at, f.captured_at)),
         min(f.first_changed_at IS NOT NULL),
         -- Two checks, each through an index: one running, or one that
         -- ended inside the window (it started at most a day before it).
         max(EXISTS (SELECT 1 FROM agent_turn o INDEXED BY idx_agent_turn_ended JOIN threads oth ON oth.id = o.thread_id
                      WHERE o.ended_at IS NULL AND oth.stream_id = f.stream_id AND o.id <> t.id
                        AND o.started_at <= coalesce(f.last_changed_at, f.captured_at))
             OR EXISTS (SELECT 1 FROM agent_turn o JOIN threads oth ON oth.id = o.thread_id
                         WHERE oth.stream_id = f.stream_id AND o.id <> t.id
                           AND o.started_at <= coalesce(f.last_changed_at, f.captured_at)
                           AND o.started_at >= strftime('%Y-%m-%dT%H:%M:%fZ',
                                 julianday(coalesce(f.first_changed_at, f.captured_at)) - 1.0)
                           AND o.ended_at >= coalesce(f.first_changed_at,
                                 strftime('%Y-%m-%dT%H:%M:%fZ', julianday(f.captured_at) - 60.0 / 86400))))
    FROM file_snapshot f
    JOIN (SELECT t.id, t.started_at, t.ended_at FROM agent_turn t
            JOIN threads th ON th.id = t.thread_id
           WHERE th.stream_id = NEW.stream_id AND t.ended_at IS NULL
          UNION ALL
          SELECT t.id, t.started_at, t.ended_at FROM agent_turn t
            JOIN threads th ON th.id = t.thread_id
           WHERE th.stream_id = NEW.stream_id
             AND t.ended_at >= strftime('%Y-%m-%dT%H:%M:%fZ',
                   julianday(NEW.at) - 120.0 / 86400 - NEW.elapsed_ms / 86400000.0)) t
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

-- What the snapshot line already holds, once: each turn's captures through
-- the (stream, captured_at) index, bounded on both sides.
INSERT INTO turn_file (turn_id, path, stream_id, first_changed_at, last_changed_at, timed, shared)
SELECT g.turn_id, g.path, g.stream_id, g.first_changed_at, g.last_changed_at, g.timed,
       EXISTS (SELECT 1 FROM agent_turn o INDEXED BY idx_agent_turn_ended JOIN threads oth ON oth.id = o.thread_id
                WHERE o.ended_at IS NULL AND oth.stream_id = g.stream_id AND o.id <> g.turn_id
                  AND o.started_at <= g.last_changed_at)
       OR EXISTS (SELECT 1 FROM agent_turn o JOIN threads oth ON oth.id = o.thread_id
                   WHERE oth.stream_id = g.stream_id AND o.id <> g.turn_id
                     AND o.started_at <= g.last_changed_at
                     AND o.started_at >= strftime('%Y-%m-%dT%H:%M:%fZ', julianday(g.shared_from) - 1.0)
                     AND o.ended_at >= g.shared_from)
  FROM (SELECT t.id AS turn_id, f.path, th.stream_id,
               min(coalesce(f.first_changed_at, f.captured_at)) AS first_changed_at,
               max(coalesce(f.last_changed_at, f.captured_at)) AS last_changed_at,
               min(f.first_changed_at IS NOT NULL) AS timed,
               min(coalesce(f.first_changed_at,
                   strftime('%Y-%m-%dT%H:%M:%fZ', julianday(f.captured_at) - 60.0 / 86400))) AS shared_from
          FROM agent_turn t
          JOIN threads th ON th.id = t.thread_id
          JOIN file_snapshot f INDEXED BY idx_file_snapshot_stream_captured
            ON f.stream_id = th.stream_id
           AND f.captured_at > t.started_at
           AND f.captured_at <= coalesce(
                 strftime('%Y-%m-%dT%H:%M:%fZ', julianday(t.ended_at) + 60.0 / 86400), '9999')
           AND CASE WHEN f.first_changed_at IS NULL
                    THEN t.ended_at IS NULL OR f.captured_at <= t.ended_at
                         OR EXISTS (SELECT 1 FROM snapshot_op o
                                     WHERE o.snapshot_id = f.snapshot_id
                                       AND o.trigger = 'turn_end' AND o.turn_id = t.id)
                    ELSE f.first_changed_at <= coalesce(t.ended_at, '9999')
                         AND f.last_changed_at >= t.started_at END
         GROUP BY t.id, f.path) g;
