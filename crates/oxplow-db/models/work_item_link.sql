-- The active work list's items' links: to another item, or to any uri
-- (`about` a collector, an effort). A target needn't resolve: what it
-- names may be gone.
SELECT l.from_uri, l.to_uri, l.to_kind, l.link_type, l.created_at
FROM source('work_item_link') l
JOIN ref('work_item') f ON f.uri = l.from_uri
