-- Efforts are removed, and their data with them: the request (V11) and
-- its evidence replace them. Rows only efforts own are deleted, every
-- `effort_id` column goes, and so do the turn's two snapshot columns
-- (a turn's change finds its snapshots by time, `turn_file`).
--
-- Migrations run with foreign keys off, so nothing cascades: children are
-- deleted here, and the tables whose `effort_id` is a foreign key are
-- rebuilt (a column with REFERENCES can't be dropped) before the effort
-- tables go. `agent_token_usage.effort_id` was ON DELETE CASCADE; the
-- rebuild keeps every token row.

-- The turn-file trigger reads `agent_turn` and sits on `snapshot_op`, both
-- rebuilt below; it is recreated, unchanged, at the end.
DROP TRIGGER snapshot_op_turn_file;

-- What efforts measured: their measures, the specs over them, and the
-- captures only the effort producers wrote.
CREATE TEMP TABLE v25_measure (id INTEGER PRIMARY KEY);
INSERT INTO v25_measure SELECT id FROM measure
 WHERE key IN ('oxplow.cycle_time', 'oxplow.task_effort', 'oxplow.effort_test_outcome',
               'oxplow.effort_tokens', 'oxplow.effort_steering', 'oxplow.token_waste',
               'oxplow.effort_time_to_green')
    OR key LIKE '%.effort_churn_lines';
CREATE TEMP TABLE v25_capture (id INTEGER PRIMARY KEY);
INSERT INTO v25_capture SELECT id FROM metric_capture
 WHERE producer IN ('effort-lifecycle', 'oxplow.effort_test_outcome')
    OR producer LIKE '%.effort_churn';
CREATE TEMP TABLE v25_fact (id INTEGER PRIMARY KEY);
INSERT INTO v25_fact SELECT f.id FROM fact f JOIN v25_capture c ON c.id = f.capture_id;
INSERT OR IGNORE INTO v25_fact SELECT f.id FROM fact f JOIN v25_measure m ON m.id = f.measure_id;

DELETE FROM metric_live_fact WHERE fact_id IN (SELECT id FROM v25_fact)
                                OR measure_id IN (SELECT id FROM v25_measure);
DELETE FROM fact WHERE id IN (SELECT id FROM v25_fact);
DELETE FROM fact_chain WHERE capture_id IN (SELECT id FROM v25_capture)
                          OR measure_id IN (SELECT id FROM v25_measure);
DELETE FROM fact_path_hold WHERE capture_id IN (SELECT id FROM v25_capture)
                              OR measure_id IN (SELECT id FROM v25_measure);
DELETE FROM metric_cube WHERE capture_id IN (SELECT id FROM v25_capture)
                           OR measure_id IN (SELECT id FROM v25_measure);
DELETE FROM metric_cube_state WHERE measure_id IN (SELECT id FROM v25_measure);
DELETE FROM metric_capture_exclusion WHERE capture_id IN (SELECT id FROM v25_capture);
UPDATE test_case_stat SET last_run_id = NULL WHERE last_run_id IN (SELECT id FROM v25_capture);
DELETE FROM metric_capture WHERE id IN (SELECT id FROM v25_capture);
DELETE FROM metric_spec WHERE source_measure IN (SELECT key FROM measure WHERE id IN (SELECT id FROM v25_measure));
DELETE FROM measure WHERE id IN (SELECT id FROM v25_measure);
UPDATE dimension SET subject_kind = 'request' WHERE key = 'oxplow.tests_stat';

-- The effort events, and the dead letters that point at them.
CREATE TEMP TABLE v25_event (seq INTEGER PRIMARY KEY);
INSERT INTO v25_event SELECT seq FROM event_log WHERE type >= 'effort.' AND type < 'effort/';
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v25_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v25_event);
-- A take an effort asked for stays, as a manual one.
UPDATE event_log SET payload = json_set(payload, '$.trigger', 'manual')
 WHERE type = 'snapshot.taken'
   AND json_extract(payload, '$.trigger') IN ('effort_start', 'effort_end');

-- An extension job's stored context names its anchors strictly.
DROP TRIGGER extension_job_context_immutable;
UPDATE extension_job_context
   SET context_json = json_remove(context_json, '$.anchors.effort_id')
 WHERE json_type(context_json, '$.anchors.effort_id') IS NOT NULL;
CREATE TRIGGER extension_job_context_immutable
BEFORE UPDATE ON extension_job_context
BEGIN
    SELECT RAISE(ABORT, 'extension job context is immutable');
END;

-- The edges efforts projected onto tasks, and any edge to or from one.
DELETE FROM page_ref
 WHERE ref_type IN ('summary_file_ref', 'summary_dir_ref', 'summary_wikilink',
                    'summary_task_mention', 'summary_finding_mention',
                    'summary_commit_mention', 'impact')
    OR (ref_type = 'touched_file' AND source_resource = 'oxplow/task')
    OR source_resource = 'oxplow/effort' OR target_resource = 'oxplow/effort';

-- Efforts in the search index.
DELETE FROM search_fts WHERE rowid IN (SELECT rowid FROM search_entry WHERE kind = 'oxplow/effort');
DELETE FROM search_entry WHERE kind = 'oxplow/effort';

-- The effort policy capability.
DELETE FROM capability_provider WHERE capability = 'effort_policy';

-- An effort's change analysis, and its files, imports and the rest.
CREATE TEMP TABLE v25_change (id INTEGER PRIMARY KEY);
INSERT INTO v25_change SELECT id FROM "change" WHERE kind = 'effort';
DELETE FROM change_file WHERE change_id IN (SELECT id FROM v25_change);
DELETE FROM change_import WHERE change_id IN (SELECT id FROM v25_change);
DELETE FROM change_duplicate WHERE change_id IN (SELECT id FROM v25_change);
DELETE FROM change_test_file WHERE change_id IN (SELECT id FROM v25_change);
DELETE FROM change_function WHERE change_id IN (SELECT id FROM v25_change);
DELETE FROM "change" WHERE id IN (SELECT id FROM v25_change);
CREATE TABLE change_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `working` | `turn` | `range` | `request`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'working', 'turn', 'range', 'request')),
    -- The sha, the turn id, the request id, `<base>..<head>` for a range,
    -- or '' for the working tree.
    target TEXT NOT NULL,
    base_revision TEXT,
    head_revision TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    error TEXT,
    computed_at TEXT,
    snapshot_id INTEGER,
    events_to INTEGER,
    files_at TEXT,
    files_events_to INTEGER,
    conflicted INTEGER,
    in_progress TEXT,
    elapsed_ms INTEGER,
    analyzed_from TEXT,
    duplicates_events_to INTEGER,
    UNIQUE (stream_id, kind, target)
);
INSERT INTO change_new SELECT id, stream_id, kind, target, base_revision, head_revision, status,
       error, computed_at, snapshot_id, events_to, files_at, files_events_to, conflicted,
       in_progress, elapsed_ms, analyzed_from, duplicates_events_to
  FROM "change";
DROP TABLE "change";
ALTER TABLE change_new RENAME TO "change";

-- agent_turn without its snapshot columns.
CREATE TABLE agent_turn_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    answer TEXT,
    session_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    agent_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    model TEXT,
    effort TEXT,
    context_tokens INTEGER,
    context_window INTEGER,
    source TEXT,
    harness_turn_id TEXT,
    cause_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    cause_ref TEXT,
    cause_basis TEXT,
    sender TEXT,
    outcome TEXT,
    error TEXT,
    question TEXT,
    request_id INTEGER REFERENCES request(id) ON DELETE SET NULL
);
INSERT INTO agent_turn_new (id, thread_id, prompt, answer, session_id, started_at, ended_at,
       agent_session_id, model, effort, context_tokens, context_window, source, harness_turn_id,
       cause_turn_id, cause_ref, cause_basis, sender, outcome, error, question, request_id)
SELECT id, thread_id, prompt, answer, session_id, started_at, ended_at,
       agent_session_id, model, effort, context_tokens, context_window, source, harness_turn_id,
       cause_turn_id, cause_ref, cause_basis, sender, outcome, error, question, request_id
  FROM agent_turn;
DROP TABLE agent_turn;
ALTER TABLE agent_turn_new RENAME TO agent_turn;
CREATE INDEX idx_agent_turn_thread ON agent_turn(thread_id, started_at DESC);
CREATE INDEX idx_agent_turn_open ON agent_turn(thread_id) WHERE ended_at IS NULL;
CREATE INDEX idx_agent_turn_open_session ON agent_turn(agent_session_id)
    WHERE ended_at IS NULL;
CREATE INDEX idx_agent_turn_request ON agent_turn(request_id) WHERE request_id IS NOT NULL;
CREATE INDEX idx_agent_turn_harness_id ON agent_turn(thread_id, harness_turn_id)
    WHERE harness_turn_id IS NOT NULL;
CREATE INDEX idx_agent_turn_ended ON agent_turn(ended_at);

-- agent_nudge without effort_id. A nudge's cause dedupes per kind and
-- request now; one that only an effort told apart keeps its row, without
-- its cause.
CREATE TABLE agent_nudge_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    trigger TEXT,
    created_at TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    cause TEXT,
    delivered_at TEXT,
    audience TEXT NOT NULL DEFAULT 'agent' CHECK (audience IN ('agent', 'person')),
    request_id INTEGER REFERENCES request(id) ON DELETE SET NULL
);
INSERT INTO agent_nudge_new (id, thread_id, kind, message, trigger, created_at, turn_id, cause,
       delivered_at, audience, request_id)
SELECT id, thread_id, kind, message, trigger, created_at, turn_id,
       CASE WHEN cause IS NOT NULL AND row_number() OVER (
                PARTITION BY cause, kind, coalesce(request_id, 0) ORDER BY id) > 1
            THEN NULL ELSE cause END,
       delivered_at, audience, request_id
  FROM agent_nudge;
DROP TABLE agent_nudge;
ALTER TABLE agent_nudge_new RENAME TO agent_nudge;
CREATE INDEX idx_agent_nudge_thread ON agent_nudge(thread_id, created_at DESC);
CREATE INDEX idx_agent_nudge_undelivered ON agent_nudge(thread_id, id) WHERE delivered_at IS NULL;
CREATE UNIQUE INDEX idx_agent_nudge_cause_kind
    ON agent_nudge(cause, kind, coalesce(request_id, 0))
    WHERE cause IS NOT NULL;

-- agent_token_usage without effort_id: every row stays.
CREATE TABLE agent_token_usage_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    agent_kind TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed')),
    recorded_at TEXT NOT NULL,
    prompt TEXT,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    cause TEXT
);
INSERT INTO agent_token_usage_new (id, stream_id, thread_id, session_id, agent_kind, model,
       input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, provenance, recorded_at, prompt, turn_id, cause)
SELECT id, stream_id, thread_id, session_id, agent_kind, model,
       input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
       message_count, provenance, recorded_at, prompt, turn_id, cause
  FROM agent_token_usage;
DROP TABLE agent_token_usage;
ALTER TABLE agent_token_usage_new RENAME TO agent_token_usage;
CREATE INDEX idx_agent_token_usage_thread
    ON agent_token_usage(thread_id, recorded_at DESC);
CREATE UNIQUE INDEX idx_agent_token_usage_cause ON agent_token_usage(cause) WHERE cause IS NOT NULL;
CREATE INDEX idx_agent_token_usage_turn ON agent_token_usage(turn_id)
    WHERE turn_id IS NOT NULL;

-- metric_capture without effort_id: a run is its turn's.
CREATE TABLE metric_capture_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    producer TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'done'
        CHECK (status IN ('running', 'done', 'failed')),
    error TEXT,
    scope TEXT,
    trigger TEXT,
    basis_ref TEXT,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed', 'asserted')),
    source TEXT NOT NULL,
    snapshot_id INTEGER,
    closest_vcs_rev TEXT,
    vcs_rev_exact INTEGER NOT NULL DEFAULT 0,
    branch TEXT,
    captured_at TEXT NOT NULL,
    ended_at TEXT,
    detail_json TEXT,
    idempotency_key TEXT,
    producer_version TEXT,
    scan_kind TEXT NOT NULL DEFAULT 'delta',
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
INSERT INTO metric_capture_new (id, stream_id, thread_id, producer, status, error, scope, trigger,
       basis_ref, provenance, source, snapshot_id, closest_vcs_rev, vcs_rev_exact, branch,
       captured_at, ended_at, detail_json, idempotency_key, producer_version, scan_kind, turn_id)
SELECT id, stream_id, thread_id, producer, status, error, scope, trigger,
       basis_ref, provenance, source, snapshot_id, closest_vcs_rev, vcs_rev_exact, branch,
       captured_at, ended_at, detail_json, idempotency_key, producer_version, scan_kind, turn_id
  FROM metric_capture;
DROP TABLE metric_capture;
ALTER TABLE metric_capture_new RENAME TO metric_capture;
CREATE INDEX idx_metric_capture_stream ON metric_capture(stream_id);
CREATE INDEX idx_metric_capture_captured_at ON metric_capture(captured_at);
CREATE INDEX idx_metric_capture_branch ON metric_capture(branch);
CREATE INDEX idx_metric_capture_version ON metric_capture(closest_vcs_rev);
CREATE INDEX idx_metric_capture_producer ON metric_capture(producer, captured_at DESC);
CREATE INDEX idx_metric_capture_trigger ON metric_capture(trigger, captured_at);
CREATE INDEX idx_metric_capture_thread ON metric_capture(thread_id);
CREATE UNIQUE INDEX idx_metric_capture_idempotency
    ON metric_capture(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX idx_metric_capture_snapshot ON metric_capture(snapshot_id);
CREATE INDEX idx_metric_capture_turn ON metric_capture (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_metric_capture_stream_producer ON metric_capture(stream_id, producer, captured_at);

-- agent_tool_call without effort_id.
CREATE TABLE agent_tool_call_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    -- Repo-relative when inside the project; NULL for tools without a path.
    path TEXT,
    -- Short context: the Bash command (truncated), a Grep pattern, …
    detail TEXT,
    -- 1 ok, 0 failed, NULL unknown (Claude's Bash response often has no exit code).
    ok INTEGER,
    at TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    event_id TEXT,
    kind TEXT NOT NULL DEFAULT 'other',
    agent_session_id INTEGER,
    subagent_id TEXT,
    subagent_kind TEXT,
    call_id TEXT,
    started_at TEXT,
    ended_at TEXT,
    duration_ms INTEGER,
    state TEXT NOT NULL DEFAULT 'finished',
    request_event_id TEXT
);
INSERT INTO agent_tool_call_new (id, thread_id, tool, path, detail, ok, at, turn_id, event_id,
       kind, agent_session_id, subagent_id, subagent_kind, call_id, started_at, ended_at,
       duration_ms, state, request_event_id)
SELECT id, thread_id, tool, path, detail, ok, at, turn_id, event_id,
       kind, agent_session_id, subagent_id, subagent_kind, call_id, started_at, ended_at,
       duration_ms, state, request_event_id
  FROM agent_tool_call;
DROP TABLE agent_tool_call;
ALTER TABLE agent_tool_call_new RENAME TO agent_tool_call;
CREATE UNIQUE INDEX idx_agent_tool_call_event ON agent_tool_call(event_id) WHERE event_id IS NOT NULL;
CREATE INDEX idx_agent_tool_call_turn ON agent_tool_call(turn_id) WHERE turn_id IS NOT NULL;
CREATE UNIQUE INDEX idx_agent_tool_call_request ON agent_tool_call(request_event_id)
    WHERE request_event_id IS NOT NULL;
CREATE INDEX idx_agent_tool_call_running ON agent_tool_call(turn_id)
    WHERE state = 'running';

-- once_mark without effort_id: a mark only an effort scoped goes; one a
-- request scoped stays its request's.
CREATE TABLE once_mark_new (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    mark      TEXT    NOT NULL,
    fired_at  TEXT    NOT NULL,
    request_id INTEGER REFERENCES request(id) ON DELETE CASCADE
) STRICT;
CREATE UNIQUE INDEX idx_once_mark_new
    ON once_mark_new(thread_id, coalesce(request_id, 0), mark);
INSERT OR IGNORE INTO once_mark_new (thread_id, mark, fired_at, request_id)
SELECT thread_id, mark, fired_at, request_id FROM once_mark
 WHERE effort_id IS NULL OR request_id IS NOT NULL
 ORDER BY fired_at;
DROP TABLE once_mark;
ALTER TABLE once_mark_new RENAME TO once_mark;
DROP INDEX idx_once_mark_new;
CREATE UNIQUE INDEX idx_once_mark
    ON once_mark(thread_id, coalesce(request_id, 0), mark);

-- snapshot_op without effort_id or the effort triggers: a take an effort
-- asked for stays, as a manual one.
CREATE TABLE snapshot_op_new (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'startup', 'manual', 'git_refs',
                          'head_moved', 'run_measured')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL,
    provider           TEXT,
    contents           INTEGER NOT NULL DEFAULT 1,
    handle             TEXT
) STRICT;
INSERT INTO snapshot_op_new (seq, stream_id, snapshot_id, parent_snapshot_id, trigger,
       thread_id, turn_id, at, elapsed_ms, budget_ms, over_budget, file_count, provider,
       contents, handle)
SELECT seq, stream_id, snapshot_id, parent_snapshot_id,
       CASE WHEN trigger IN ('effort_start', 'effort_end') THEN 'manual' ELSE trigger END,
       thread_id, turn_id, at, elapsed_ms, budget_ms, over_budget, file_count, provider,
       contents, handle
  FROM snapshot_op;
DROP TABLE snapshot_op;
ALTER TABLE snapshot_op_new RENAME TO snapshot_op;
CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;

-- event_log and extension_run hold effort_id as a plain column.
DROP INDEX event_log_effort;
ALTER TABLE event_log DROP COLUMN effort_id;
ALTER TABLE extension_run DROP COLUMN effort_id;

-- The effort tables.
DROP TABLE effort_evidence_state;
DROP TABLE effort_observation_row;
DROP TABLE effort_metric_delta;
DROP TABLE effort_file;
DROP TABLE effort;

CREATE TRIGGER snapshot_op_turn_file AFTER INSERT ON snapshot_op
BEGIN
  INSERT INTO turn_file (turn_id, path, stream_id, first_changed_at, last_changed_at, timed, shared)
  SELECT t.id, f.path, f.stream_id,
         min(coalesce(f.first_changed_at, f.captured_at)),
         max(coalesce(f.last_changed_at, f.captured_at)),
         min(f.first_changed_at IS NOT NULL),
         -- Two checks, each through an index: one running, or one that
         -- ended inside the window (it started at most a day before it).
         max(EXISTS (SELECT 1 FROM agent_turn o INDEXED BY idx_agent_turn_ended JOIN threads oth ON oth.id = o.thread_id
                      WHERE o.ended_at IS NULL AND oth.stream_id = f.stream_id AND o.id <> t.id
                        AND o.started_at <= coalesce(f.last_changed_at, f.captured_at))
             OR EXISTS (SELECT 1 FROM agent_turn o JOIN threads oth ON oth.id = o.thread_id
                         WHERE oth.stream_id = f.stream_id AND o.id <> t.id
                           AND o.started_at <= coalesce(f.last_changed_at, f.captured_at)
                           AND o.started_at >= strftime('%Y-%m-%dT%H:%M:%fZ',
                                 julianday(coalesce(f.first_changed_at, f.captured_at)) - 1.0)
                           AND o.ended_at >= coalesce(f.first_changed_at,
                                 strftime('%Y-%m-%dT%H:%M:%fZ', julianday(f.captured_at) - 60.0 / 86400))))
    FROM file_snapshot f
    JOIN (SELECT t.id, t.started_at, t.ended_at FROM agent_turn t
            JOIN threads th ON th.id = t.thread_id
           WHERE th.stream_id = NEW.stream_id AND t.ended_at IS NULL
          UNION ALL
          SELECT t.id, t.started_at, t.ended_at FROM agent_turn t
            JOIN threads th ON th.id = t.thread_id
           WHERE th.stream_id = NEW.stream_id
             AND t.ended_at >= strftime('%Y-%m-%dT%H:%M:%fZ',
                   julianday(NEW.at) - 120.0 / 86400 - NEW.elapsed_ms / 86400000.0)) t
   WHERE f.snapshot_id = NEW.snapshot_id
     AND t.started_at < f.captured_at
     AND (t.ended_at IS NULL
          OR t.ended_at >= strftime('%Y-%m-%dT%H:%M:%fZ', julianday(f.captured_at) - 60.0 / 86400))
     AND CASE WHEN f.first_changed_at IS NULL
              -- No change time: a capture while it ran, or its own end take.
              THEN t.ended_at IS NULL OR f.captured_at <= t.ended_at
                   OR (NEW.trigger = 'turn_end' AND NEW.turn_id = t.id)
              ELSE f.first_changed_at <= coalesce(t.ended_at, '9999')
                   AND f.last_changed_at >= t.started_at END
   GROUP BY t.id, f.path
  ON CONFLICT (turn_id, path) DO UPDATE SET
    first_changed_at = min(first_changed_at, excluded.first_changed_at),
    last_changed_at = max(last_changed_at, excluded.last_changed_at),
    timed = min(timed, excluded.timed),
    shared = max(shared, excluded.shared);
END;

DROP TABLE v25_measure;
DROP TABLE v25_capture;
DROP TABLE v25_fact;
DROP TABLE v25_event;
DROP TABLE v25_change;
