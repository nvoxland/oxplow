-- Semantic layer: the read-only `v_*` contract over core data.
--
-- These views are the VERSIONED CONTRACT that lenses, extensions and
-- agents (`query_sql`) read. Physical tables stay internal. Column
-- docs live in `crates/oxplow-db/src/semantic_layer.rs` (CATALOG) and a
-- test asserts they match these views exactly. Changing a view's
-- columns = a new migration + a doc change in .context/semantic-layer.md.

CREATE VIEW v_stream AS
SELECT id, kind, title, branch, worktree_path, created_at, updated_at, archived_at
FROM streams;

CREATE VIEW v_thread AS
SELECT id, stream_id, title, status, agent, sort_index, created_at, updated_at,
       closed_at, archived_at
FROM threads;

CREATE VIEW v_task AS
SELECT t.id, t.thread_id, th.stream_id, t.parent_id, t.title, t.description,
       t.status, t.priority, t.author, t.sort_index, t.created_at, t.updated_at,
       t.completed_at
FROM task t
LEFT JOIN threads th ON th.id = t.thread_id
WHERE t.deleted_at IS NULL;

CREATE VIEW v_effort AS
SELECT e.id, e.task_id, e.thread_id, th.stream_id, e.started_at, e.ended_at,
       e.start_snapshot_id, e.end_snapshot_id, e.summary
FROM task_effort e
LEFT JOIN threads th ON th.id = e.thread_id;

CREATE VIEW v_comment AS
SELECT c.id, c.stream_id, c.thread_id, c.target_kind, c.target_id, c.quote,
       c.intent, c.status, c.orphaned, c.author, c.created_at, c.updated_at,
       c.last_activity_at, c.resolved_at,
       (SELECT m.body FROM comment_message m WHERE m.comment_id = c.id
         ORDER BY m.id LIMIT 1) AS body,
       (SELECT count(*) FROM comment_message m WHERE m.comment_id = c.id)
         AS message_count
FROM comment c;

CREATE VIEW v_wiki_page AS
SELECT slug, title, body_excerpt, body_size_bytes, created_at, updated_at
FROM wiki_page;

CREATE VIEW v_snapshot AS
SELECT id, stream_id, created_at, git_commit, git_branch
FROM snapshot;

CREATE VIEW v_measure AS
SELECT key, title, unit, subject_kind, temporal_semantics, scope, description
FROM measure;

CREATE VIEW v_capture AS
SELECT id, stream_id, thread_id, effort_id, producer, status, trigger,
       provenance, source, snapshot_id, branch, closest_git_version,
       captured_at, ended_at, scan_kind
FROM metric_capture;

CREATE VIEW v_fact AS
SELECT f.id, f.capture_id, m.key AS measure_key, f.value, f.numerator,
       f.denominator, f.subject_kind, f.subject_ref, f.path, f.line,
       f.severity, f.rule, f.detail, f.dims_json,
       c.stream_id, c.thread_id, c.effort_id, c.captured_at, c.branch
FROM fact f
JOIN measure m ON m.id = f.measure_id
JOIN metric_capture c ON c.id = f.capture_id;
