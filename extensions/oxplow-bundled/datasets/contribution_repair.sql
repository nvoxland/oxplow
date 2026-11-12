-- The open repair task for each contribution: the one linked `about` it
-- that is not done or canceled (filed by the repair effect).
SELECT CAST(l.to_uri AS TEXT) AS uri, CAST(max(w.uri) AS TEXT) AS repair_item
FROM ref('task_link') l
JOIN ref('task') w ON w.uri = l.from_uri
WHERE l.link_type = 'about' AND w.state NOT IN ('done', 'canceled')
GROUP BY l.to_uri
