-- Every read of a knowledge page: an agent's (the `knowledge_read_record`
-- model, written from `record.read`, through a tool or the page's file) and
-- a person's (their page visits).
WITH agent AS (
  SELECT CAST(m.page AS TEXT) AS uri, CAST('agent' AS TEXT) AS actor, CAST(th.id AS INT) AS thread_id,
         CAST(m.turn_id AS INT) AS turn_id, CAST(m.via AS TEXT) AS via, CAST(m.at AS TEXT) AS at
  FROM ref('knowledge_read_record') m LEFT JOIN ref('thread') th ON th.uri = m.thread),
person AS (
  SELECT CAST(v.page_id AS TEXT) AS uri, CAST('person' AS TEXT) AS actor, CAST(v.thread_id AS INT) AS thread_id,
         CAST(NULL AS INT) AS turn_id, CAST('open' AS TEXT) AS via, CAST(v.visited_at AS TEXT) AS at
  FROM ref('page_visit') v WHERE v.page_kind = 'oxplow/knowledge')
SELECT * FROM agent UNION ALL SELECT * FROM person
