SELECT id, thread_id, from_item_id AS from_task_id, to_item_id AS to_task_id,
       link_type, created_at
FROM source('task_link')
