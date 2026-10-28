SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id,
       effort_id, at, elapsed_ms, budget_ms, over_budget, file_count, provider, contents,
       handle
  FROM source('snapshot_op')
