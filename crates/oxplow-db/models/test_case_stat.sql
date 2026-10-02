SELECT stream_id, branch, producer, subject, last_status, last_ms, max_ms,
       CASE WHEN timed_runs > 0 THEN total_ms / timed_runs END AS mean_ms,
       runs, failures, flips, first_seen_at, last_seen_at, last_failed_at,
       last_passed_at, last_run_id
FROM source('test_case_stat')
