-- The tasks each request worked on, in today's `v_oxplow_request_link` shape, over
-- the `request_task` model. One row per (request,
-- task, basis), with the request's totals so far. `basis` is the model's:
-- `filed` and `edited` are core's `command`.
-- Built once per statement (`MATERIALIZED`): another dataset joining these
-- rows would otherwise rebuild the joins below for every row it probes. The
-- turns of each request are totalled once (a per-row subquery over the turn
-- dataset ran its view for every link), and the request is joined by the id
-- its URI carries rather than by comparing a URI built for every request.
WITH turns AS (
  SELECT request_id, COUNT(*) AS n, MIN(started_at) AS first_at, MAX(ended_at) AS last_at
    FROM ref('agent_turn') GROUP BY request_id),
links AS MATERIALIZED (
  SELECT CAST(m.request AS TEXT) AS request,
         CAST(m.task AS TEXT) AS task,
         CAST(m.basis AS TEXT) AS basis,
         CAST(CASE WHEN m.turn_id IS NULL THEN NULL
                   ELSE uri('oxplow/turn', 'id', 'trn' || m.turn_id) END AS TEXT) AS turn,
         CAST(t.thread_id AS INT) AS thread_id,
         CAST(m.at AS TEXT) AS at,
         CAST(coalesce(turns.n, 0) AS INT) AS turns,
         CAST(turns.first_at AS TEXT) AS started_at,
         CAST(turns.last_at AS TEXT) AS last_ended_at,
         CAST(r.id AS INT) AS request_id
    FROM ref('request_task') m
    JOIN ref('request') r ON r.id = CAST(substr(uri_param(m.request, 'id'), 4) AS INT)
    LEFT JOIN ref('agent_turn') t ON t.id = m.turn_id
    LEFT JOIN turns ON turns.request_id = r.id)
SELECT request, task, basis, turn, thread_id, at, turns, started_at, last_ended_at, request_id
  FROM links
