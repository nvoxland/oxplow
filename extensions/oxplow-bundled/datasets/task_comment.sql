-- Comments on every store's tasks, in today's `v_oxplow_task_comment` shape, over
-- the shared model's comments. `id` is the store's own comment id (oxtask: `cmt<N>`).
SELECT c.id, c.parent AS uri, c.body, c.author, c.at AS created_at
FROM ref('task_record_comments') c
