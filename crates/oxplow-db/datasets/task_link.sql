-- The active task tracker's items' links: to another item, or to any uri
-- (`about` a collector, a request). A target needn't resolve: what it
-- names may be gone.
SELECT l.from_uri, l.to_uri, l.to_kind, l.link_type, l.created_at
FROM source('task_link') l
JOIN ref('task') f ON f.uri = l.from_uri
