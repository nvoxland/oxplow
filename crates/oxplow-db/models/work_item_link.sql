-- The active work list's items' links: to another item, or to any ref
-- (`about` a collector, an effort). A target needn't resolve: what it
-- names may be gone.
SELECT l.from_ref, l.to_ref, l.to_kind, l.link_type, l.created_at
FROM source('work_item_link') l
JOIN ref('work_item') f ON f.ref = l.from_ref
