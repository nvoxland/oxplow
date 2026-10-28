SELECT id, title, status, priority, thread_id, stream_id, last_touched_at, days_idle
FROM ref('stale_task')
ORDER BY days_idle DESC, last_touched_at ASC, id ASC
