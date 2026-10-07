-- Links between the active work list's items.
SELECT l.from_ref, l.to_ref, l.link_type, l.created_at
FROM source('work_item_link') l
JOIN ref('work_item') f ON f.ref = l.from_ref
JOIN ref('work_item') t ON t.ref = l.to_ref
