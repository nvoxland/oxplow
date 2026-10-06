-- The schema as of the squash (tsk1080): what V1..V168 built, as one
-- migration. Views aren't here: the models compile them at every open.

CREATE TABLE streams (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('primary', 'worktree')),
    title TEXT NOT NULL,
    branch TEXT NOT NULL,
    branch_ref TEXT NOT NULL,
    branch_source TEXT NOT NULL,
    worktree_path TEXT NOT NULL,
    working_pane TEXT NOT NULL DEFAULT '',
    talking_pane TEXT NOT NULL DEFAULT '',
    working_session_id TEXT NOT NULL DEFAULT '',
    talking_session_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, archived_at TEXT, custom_prompt TEXT);
CREATE TABLE runtime_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL
);
INSERT INTO runtime_state VALUES(1,NULL);
CREATE TABLE threads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'queued', 'closed')),
    sort_index INTEGER NOT NULL DEFAULT 0,
    pane_target TEXT NOT NULL DEFAULT 'working',
    resume_session_id TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    summary_updated_at TEXT,
    closed_at TEXT,
    custom_prompt TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, archived_at TEXT, agent TEXT NOT NULL DEFAULT 'claude'
    CHECK (agent IN ('claude', 'codex', 'opencode', 'acp')), acp_agent TEXT);
CREATE TABLE thread_selection (
    stream_id INTEGER PRIMARY KEY REFERENCES streams(id) ON DELETE CASCADE,
    selected_thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL
);
CREATE TABLE task (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Nullable: null means the task is on the project-wide backlog.
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    parent_id INTEGER REFERENCES task(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('ready', 'in_progress', 'blocked', 'done', 'canceled', 'archived')),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'urgent')),
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL CHECK (created_by IN ('user', 'agent', 'system')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    deleted_at TEXT,
    -- Semantic origin (vs. created_by which is the writer).
    author TEXT CHECK (author IN ('user', 'agent')));
CREATE TABLE task_link (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from', 'duplicates', 'supersedes', 'replies_to')),
    created_at TEXT NOT NULL,
    CHECK (from_item_id <> to_item_id)
);
CREATE TABLE task_note (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- Mutually exclusive: a note is attached to either a task or
    -- a thread, never both, never neither.
    CHECK (
        (task_id IS NOT NULL AND thread_id IS NULL)
        OR (task_id IS NULL AND thread_id IS NOT NULL)
    )
);
CREATE TABLE IF NOT EXISTS "wiki_page" (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body_path TEXT NOT NULL,
    body_excerpt TEXT NOT NULL DEFAULT '',
    body_size_bytes INTEGER NOT NULL DEFAULT 0,
    file_refs_json TEXT NOT NULL DEFAULT '[]',
    related_notes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, dir_refs_json TEXT NOT NULL DEFAULT '[]', body_hash TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '');
CREATE TABLE page_visit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    page_kind TEXT NOT NULL,
    page_id TEXT NOT NULL,
    visited_at TEXT NOT NULL,
    duration_ms INTEGER
, thread_id INTEGER, label TEXT NULL);
CREATE TABLE usage_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);
CREATE TABLE code_quality_scan (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    started_at TEXT NOT NULL,
    ended_at TEXT,
    error TEXT
, file_filter TEXT, revision TEXT NOT NULL DEFAULT 'working');
CREATE TABLE code_quality_finding (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id INTEGER NOT NULL REFERENCES code_quality_scan(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    kind TEXT NOT NULL,
    metric_value REAL NOT NULL,
    extra_json TEXT
);
CREATE TABLE agent_turn (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    answer TEXT,
    session_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT
, snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL, start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL);
CREATE TABLE IF NOT EXISTS "effort_file" (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('created', 'updated', 'deleted')), local_snapshot_id INTEGER NOT NULL DEFAULT 0, closest_vcs_rev TEXT, vcs_rev_exact INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (effort_id, path)
);
CREATE TABLE IF NOT EXISTS "wiki_page_thread_update" (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    slug TEXT NOT NULL REFERENCES "wiki_page"(slug) ON DELETE CASCADE,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, slug)
);
CREATE TABLE page_ref (
  source_kind  TEXT NOT NULL,
  source_id    TEXT NOT NULL,
  target_kind  TEXT NOT NULL,
  target_id    TEXT NOT NULL,
  ref_type     TEXT NOT NULL,
  source_extra TEXT, local_snapshot_id INTEGER, closest_vcs_rev TEXT, vcs_rev_exact INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (source_kind, source_id, target_kind, target_id, ref_type)
);
CREATE TABLE IF NOT EXISTS "snapshot" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    revision TEXT
, branch TEXT, tree_hash TEXT);
CREATE TABLE IF NOT EXISTS "effort" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    end_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    summary TEXT,
    impacts_json TEXT
, work_item TEXT NOT NULL DEFAULT '');
CREATE TABLE effort_acknowledged_path (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    acknowledged_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (effort_id, path)
);
CREATE TABLE comment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    -- The selected text: the durable anchor + the context handed to
    -- the agent.
    quote TEXT NOT NULL,
    -- Opaque per-surface position hint (re-validated on load).
    selectors_json TEXT NOT NULL,
    -- 'note' (note-to-self) | 'followup' (wants the agent to act).
    intent TEXT NOT NULL DEFAULT 'note',
    -- 'open' | 'resolved'.
    status TEXT NOT NULL DEFAULT 'open',
    -- 1 when the quote could no longer be located in current content;
    -- still listed in the inbox, just without an inline highlight.
    orphaned INTEGER NOT NULL DEFAULT 0,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- Bumped on every new message; drives sorting + GC.
    last_activity_at TEXT NOT NULL
, resolved_at TEXT, context_chain_json TEXT NOT NULL DEFAULT '[]', referenced_refs_json TEXT NOT NULL DEFAULT '[]');
CREATE TABLE comment_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comment(id) ON DELETE CASCADE,
    -- Free-form, e.g. 'user' or 'agent'.
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE search_entry (
    rowid     INTEGER PRIMARY KEY,
    kind      TEXT NOT NULL,
    ref_id    TEXT NOT NULL,
    stream_id TEXT
, content_hash TEXT);
CREATE VIRTUAL TABLE search_fts USING fts5(
    title,
    body,
    tokenize = 'porter unicode61',
    prefix = '2 3'
);
CREATE TABLE agent_nudge (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    trigger TEXT,
    created_at TEXT NOT NULL
, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL, cause TEXT, delivered_at TEXT);
CREATE TABLE effort_unattributed_file (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, path)
);
CREATE TABLE agent_token_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    agent_kind TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed')),
    recorded_at TEXT NOT NULL
, prompt TEXT, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL, cause TEXT);
CREATE TABLE agent_token_cursor (
    session_id TEXT PRIMARY KEY,
    byte_offset INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "file_snapshot" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    blob_hash TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT NOT NULL,
    storage TEXT NOT NULL DEFAULT 'oxplow'
        CHECK (storage IN ('oxplow', 'git', 'oversize', 'deleted')),
    snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE CASCADE,
    mtime_ms INTEGER
, content_hash TEXT);
CREATE TABLE effort_attribution (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('claimed', 'unattributed', 'acknowledged')),
    detail_json TEXT,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, kind, ref)
);
CREATE TABLE measure (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,                  -- namespaced; `oxplow.*` reserved
    title TEXT NOT NULL,
    unit TEXT,
    -- The grain's subject kind (symbol | file | test | dependency | model | …).
    subject_kind TEXT,
    -- Additivity OVER TIME — the BI semi-additive distinction. A snapshot measure
    -- (complexity, todo count, coverage) is semi-additive (sum across subjects,
    -- last/avg across time); an event measure (tokens, lint hits) is additive
    -- (sum incl. time); a ratio (coverage %) is non-additive (re-derive Σn/Σd).
    temporal_semantics TEXT NOT NULL DEFAULT 'semi-additive'
        CHECK (temporal_semantics IN ('additive', 'semi-additive', 'non-additive')),
    -- For ratio bases: whether this measure is the numerator/denominator of a
    -- derived ratio metric. Most measures are `none`.
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, capture_scope TEXT NOT NULL DEFAULT 'complete', extension TEXT);
INSERT INTO measure VALUES(1,'oxplow.complexity','Cyclomatic complexity','count','symbol','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
INSERT INTO measure VALUES(2,'oxplow.fn_length','Function length','lines','symbol','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
INSERT INTO measure VALUES(3,'oxplow.parameter_count','Parameter count','count','symbol','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
INSERT INTO measure VALUES(4,'oxplow.todo','TODO markers','count','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
INSERT INTO measure VALUES(5,'oxplow.coverage','Line coverage','%','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(6,'oxplow.test_case','Test case','count','test','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-subject',NULL);
INSERT INTO measure VALUES(7,'oxplow.lint_hit','Static-analysis hit','count','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(8,'oxplow.duplicate_lines','Duplicated lines','lines','symbol','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(9,'oxplow.tokens','Agent tokens','count','model','additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(10,'oxplow.cycle_time','Effort cycle time','ms','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(11,'oxplow.ast_hit','AST idiom hits','count','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
INSERT INTO measure VALUES(12,'oxplow.turn','Agent turns','count','model','additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(13,'oxplow.task_effort','Efforts per task','count','task','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(14,'oxplow.nudge','Nudges fired','count',NULL,'additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(15,'oxplow.effort_test_outcome','Effort test outcome','count','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(16,'oxplow.test_duration','Test duration','ms','test','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-subject',NULL);
INSERT INTO measure VALUES(17,'oxplow.cache_tokens','Prompt-cache tokens','count','model','additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(18,'oxplow.cache_usage','Prompt-cache hit ratio','%','model','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(19,'oxplow.effort_tokens','Effort token spend','count','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(20,'oxplow.effort_steering','Effort steering events','count','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(21,'oxplow.effort_time_to_green','Effort time to green','ms','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(22,'oxplow.token_waste','Reverted-effort token waste','count','effort','non-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(23,'oxplow.coverage.branch','Branch coverage','%','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(24,'oxplow.coverage.function','Function coverage','%','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete',NULL);
INSERT INTO measure VALUES(25,'oxplow.doc_coverage','Doc coverage','%','file','semi-additive','built-in',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path',NULL);
CREATE TABLE dimension (
    key TEXT PRIMARY KEY,                      -- namespaced; `oxplow.*` reserved
    label TEXT NOT NULL,
    value_type TEXT NOT NULL
        CHECK (value_type IN ('categorical', 'numeric', 'temporal', 'entity-ref')),
    subject_kind TEXT,                         -- for entity-ref dims
    vocabulary_json TEXT,                      -- optional controlled value set
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    promoted INTEGER NOT NULL DEFAULT 0
, entity_json TEXT, extension TEXT);
INSERT INTO dimension VALUES('oxplow.language','Language','categorical',NULL,NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.severity','Severity','categorical',NULL,NULL,'built-in',1,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.status','Status','categorical',NULL,NULL,'built-in',1,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.branch','Branch','categorical',NULL,NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.model','Model','categorical','model',NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.agent','Agent','categorical','agent',NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.package','Package','categorical',NULL,NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.test_suite','Test suite','categorical',NULL,NULL,'built-in',0,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.rule','Rule','categorical',NULL,NULL,'built-in',1,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.token_kind','Token kind','categorical',NULL,NULL,'built-in',1,NULL,NULL);
INSERT INTO dimension VALUES('oxplow.tests_stat','Test outcome stat','categorical','effort',NULL,'built-in',1,NULL,NULL);
CREATE TABLE metric_capture (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    -- The PRODUCING effort (provenance), stamped only when unambiguous (single
    -- open effort or explicit task id), else NULL; backfilled by the attribution
    -- ledger at close. SET NULL on GC: the capture (and its facts) outlive the
    -- effort. NOT the reporting overlay (that stays time-window + claim ledger).
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
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
    ended_at TEXT
, detail_json TEXT, idempotency_key TEXT, producer_version TEXT, scan_kind TEXT NOT NULL DEFAULT 'delta', turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL);
CREATE TABLE fact (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    value REAL NOT NULL,
    -- Ratio components, so roll-ups re-aggregate as Σnum/Σden (never naive-AVG).
    numerator REAL,
    denominator REAL,
    -- Subject + location-at-capture (per-fact, unlike the version which is the
    -- capture's). `subject_ref` is the logical id; `path`/`line` the coordinate.
    subject_kind TEXT,
    subject_ref TEXT,
    path TEXT,
    line INTEGER,
    -- Reported finding metadata (lint/CVE); NULL for pure measurements.
    severity TEXT,
    rule TEXT,
    detail TEXT,
    -- Open conformed-dimension tail, keyed by namespaced dimension key.
    dims_json TEXT
);
CREATE TABLE metric_spec (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,               -- namespaced; `oxplow.*` reserved
    title TEXT NOT NULL,
    unit TEXT,
    -- The measure whose facts this metric aggregates. NULL for a formula metric
    -- (derived purely from other metrics via `formula`).
    source_measure TEXT,
    -- How the source measure's facts combine WITHIN a capture. (Cross-time collapse
    -- is the source measure's `temporal_semantics`, applied by the engine.)
    aggregation TEXT NOT NULL DEFAULT 'last'
        CHECK (aggregation IN
            ('count', 'count_distinct', 'sum', 'avg', 'min', 'max', 'last', 'p95', 'ratio')),
    -- Conjunctive predicate over facts (min_value / severity / dim equality), JSON.
    -- This is what makes a metric a count-over-threshold rather than a raw measure.
    filter_json TEXT,
    -- Derived-metric formula referencing other metric keys ({op, left, right}); NULL
    -- for a base metric. Mutually informative with `source_measure`.
    formula TEXT,
    -- Conformed dims this metric may be sliced by (JSON array of dimension keys).
    sliceable_dims_json TEXT,
    -- Presentation: how a good/bad reading is derived + rendered at READ time.
    direction TEXT NOT NULL DEFAULT 'neutral'
        CHECK (direction IN ('higher-better', 'lower-better', 'neutral')),
    target REAL,
    warn_at REAL,
    fail_at REAL,
    description TEXT,
    category TEXT,
    language TEXT,
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    -- Read-time presentation kind (gauge | findings | test | coverage | event).
    display_kind TEXT NOT NULL DEFAULT 'gauge'
        CHECK (display_kind IN ('gauge', 'findings', 'test', 'coverage', 'event')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, extension TEXT, entity_json TEXT);
CREATE TABLE metric_cube (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    -- The producer whose live facts this bucket holds — NOT necessarily the
    -- capture's own producer (the state at capture N includes every producer's
    -- live facts, including ones that last ran earlier).
    --
    -- In the grain for two reasons. (1) It mirrors the fold, whose state is
    -- already keyed by producer. (2) The read derives its capture list from "the
    -- producers that ever emitted a fact matching this spec's filter", so a cube
    -- that merged producers into one row could not reproduce that derivation, and
    -- two specs over the same measure with different filters would silently get
    -- the same capture list. Merging across producers is decomposable, so the
    -- point is unchanged; the cost is only row count (~152 -> ~304 for tests,
    -- still nothing).
    producer TEXT NOT NULL,
    -- Canonical JSON of the promoted dimension values for this bucket
    -- (`{"oxplow.status":"passed"}`), '{}' when no dim is promoted. Built by the
    -- SAME `dim_value` the read uses — the bucketing is done in Rust precisely so
    -- there is never a second implementation of dim extraction to drift.
    dims_key TEXT NOT NULL,
    -- The decomposable components. `fact_count` is the bucket's fact count, NOT a
    -- subject count (a path may contribute many facts).
    fact_count INTEGER NOT NULL,
    value_sum REAL NOT NULL,
    value_min REAL,
    value_max REAL,
    numerator REAL NOT NULL DEFAULT 0,
    denominator REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (measure_id, capture_id, producer, dims_key)
) WITHOUT ROWID;
CREATE TABLE metric_live_fact (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL,
    branch TEXT NOT NULL,
    producer TEXT NOT NULL,
    subject_key TEXT NOT NULL,
    fact_id INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
    PRIMARY KEY (measure_id, stream_id, branch, producer, subject_key, fact_id)
) WITHOUT ROWID;
CREATE TABLE metric_cube_epoch (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    epoch INTEGER NOT NULL
, version INTEGER NOT NULL DEFAULT 0);
INSERT INTO metric_cube_epoch VALUES(1,0,0);
CREATE TABLE metric_cube_state (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    last_capture_id INTEGER NOT NULL,
    last_captured_at TEXT NOT NULL,
    PRIMARY KEY (measure_id, stream_id, branch)
) WITHOUT ROWID;
CREATE TABLE dashboard (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, settings_json TEXT);
CREATE TABLE dashboard_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dashboard_id INTEGER NOT NULL REFERENCES dashboard(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    -- `metric` (charts one metric) | `text` (a heading / markdown note).
    kind TEXT NOT NULL,
    -- The metric spec key for a `metric` tile; NULL for a `text` tile.
    options_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE claim (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    statement TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tests_pass', 'no_behavior_change', 'handles_case', 'other')),
    -- What backs it: `run:<capture id>`, a test name, a file, … NULL = unbacked.
    evidence_ref TEXT,
    created_at TEXT NOT NULL
, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL);
CREATE TABLE agent_tool_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    tool TEXT NOT NULL,
    -- Repo-relative when inside the project; NULL for tools without a path.
    path TEXT,
    -- Short context: the Bash command (truncated), a Grep pattern, …
    detail TEXT,
    -- 1 ok, 0 failed, NULL unknown (Claude's Bash response often has no exit code).
    ok INTEGER,
    at TEXT NOT NULL
, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL, event_id TEXT);
CREATE TABLE ai_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    -- What asked: `mcp:ai_decide`, `source:<ext>/<id>`, `inferred-decisions`, …
    caller TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    ok INTEGER NOT NULL,
    error TEXT,
    at TEXT NOT NULL
, input_hash TEXT);
CREATE TABLE effort_metric_delta (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    title TEXT NOT NULL,
    unit TEXT,
    direction TEXT NOT NULL,
    kind TEXT NOT NULL,
    category TEXT,
    language TEXT,
    agg TEXT NOT NULL,
    baseline REAL,
    current REAL NOT NULL,
    delta REAL,
    changed INTEGER NOT NULL,
    attributed_files INTEGER,
    sample_count INTEGER NOT NULL,
    target REAL,
    warn_at REAL,
    fail_at REAL,
    crossing TEXT,
    latest_capture_id INTEGER,
    refreshed_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, key)
);
CREATE TABLE effort_observation_row (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source TEXT NOT NULL,
    metric_value REAL,
    payload_json TEXT,
    local_snapshot_id INTEGER,
    created_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, seq)
);
CREATE TABLE change_file (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    additions INTEGER NOT NULL,
    deletions INTEGER NOT NULL,
    zone TEXT,
    is_test INTEGER NOT NULL,
    PRIMARY KEY (change_id, path)
);
CREATE TABLE change_function (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    container TEXT NOT NULL,
    name TEXT NOT NULL,
    -- `added` | `deleted` | `modified`
    status TEXT NOT NULL,
    signature_changed INTEGER NOT NULL,
    body_changed INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    visibility TEXT NOT NULL,
    is_test INTEGER NOT NULL,
    complexity REAL,
    length INTEGER,
    params_before INTEGER,
    params_after INTEGER,
    complexity_delta REAL,
    length_delta INTEGER,
    added_lines INTEGER,
    deleted_lines INTEGER,
    modified_lines INTEGER,
    churn_share REAL,
    PRIMARY KEY (change_id, path, container, name)
);
CREATE TABLE change_import (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    module TEXT NOT NULL,
    -- `added` | `removed`
    direction TEXT NOT NULL,
    start_line INTEGER,
    from_zone TEXT,
    to_zone TEXT,
    cross_zone INTEGER NOT NULL
);
CREATE TABLE change_duplicate (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    lines INTEGER NOT NULL,
    peer_path TEXT NOT NULL,
    peer_start_line INTEGER NOT NULL,
    peer_end_line INTEGER NOT NULL
);
CREATE TABLE change_test_file (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    tests_before INTEGER NOT NULL,
    tests_after INTEGER NOT NULL,
    assertions_before INTEGER NOT NULL,
    assertions_after INTEGER NOT NULL,
    skips_before INTEGER NOT NULL,
    skips_after INTEGER NOT NULL,
    PRIMARY KEY (change_id, path)
);
CREATE TABLE git_commit (
    sha TEXT PRIMARY KEY,
    author TEXT NOT NULL,
    email TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    parents_json TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE git_commit_file (
    sha TEXT NOT NULL REFERENCES git_commit(sha) ON DELETE CASCADE,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (sha, path)
);
CREATE TABLE git_branch (
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('local', 'remote')),
    remote TEXT,
    head_sha TEXT,
    stream_id INTEGER,
    updated_at TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, remote, name)
);
CREATE TABLE lsp_diagnostic (
    stream_id INTEGER NOT NULL,
    language TEXT NOT NULL,
    path TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    source TEXT,
    code TEXT,
    line INTEGER NOT NULL,
    col INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    end_col INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE event_log (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    type         TEXT    NOT NULL,
    v            INTEGER NOT NULL,
    at           TEXT    NOT NULL,
    source       TEXT    NOT NULL,
    stream_id    INTEGER,
    thread_id    INTEGER,
    effort_id    INTEGER,
    turn_id      INTEGER,
    snapshot_id  INTEGER,
    -- JSON array of canonical refs the event is about.
    subject      TEXT    NOT NULL,
    -- JSON, validated against `type@v`'s schema on append.
    payload      TEXT    NOT NULL,
    -- Reserved for forgettable payloads: the content hash of a body that
    -- lives in the content store and may be purged while the row stays.
    payload_hash TEXT,
    cause        TEXT,
    -- Emitter-derived key; a second append of the same occurrence fails.
    dedupe_key   TEXT    UNIQUE
, payload_expired_at TEXT) STRICT;
CREATE TABLE event_consumer_checkpoint (
    consumer   TEXT    PRIMARY KEY,
    last_seq   INTEGER NOT NULL,
    updated_at TEXT    NOT NULL
) STRICT;
CREATE TABLE event_dead_letter (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    consumer        TEXT    NOT NULL,
    event_seq       INTEGER NOT NULL REFERENCES event_log(seq),
    error           TEXT    NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 1,
    first_failed_at TEXT    NOT NULL,
    last_failed_at  TEXT    NOT NULL,
    state           TEXT    NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'retried', 'discarded')),
    UNIQUE (consumer, event_seq)
) STRICT;
CREATE TABLE IF NOT EXISTS "change" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working` | `turn`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working', 'turn')),
    -- The sha, the effort id, the turn id, or '' for the working tree.
    target TEXT NOT NULL,
    base_revision TEXT,
    head_revision TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    error TEXT,
    computed_at TEXT, snapshot_id INTEGER, events_to INTEGER,
    UNIQUE (stream_id, kind, target)
);
CREATE TABLE event_content (
    hash       TEXT    PRIMARY KEY,
    namespace  TEXT    NOT NULL,
    bytes      BLOB    NOT NULL,
    size       INTEGER NOT NULL,
    created_at TEXT    NOT NULL
) STRICT;
CREATE TABLE effort_once_mark (
    effort_id INTEGER NOT NULL REFERENCES effort(id) ON DELETE CASCADE,
    mark      TEXT    NOT NULL,
    fired_at  TEXT    NOT NULL,
    PRIMARY KEY (effort_id, mark)
) STRICT;
CREATE TABLE model_input (
    view  TEXT NOT NULL REFERENCES model(view) ON DELETE CASCADE,
    input TEXT NOT NULL,
    kind  TEXT NOT NULL CHECK (kind IN ('ref', 'source')),
    PRIMARY KEY (view, input)
) STRICT;
CREATE TABLE model_contract (
    view         TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    -- [{"name", "type", "doc"}] in column order.
    columns_json TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL, key_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (view, version)
) STRICT;
CREATE TABLE model_test (
    view   TEXT NOT NULL REFERENCES model(view) ON DELETE CASCADE,
    test   TEXT NOT NULL,
    state  TEXT NOT NULL CHECK (state IN ('passed', 'failed', 'error')),
    detail TEXT,
    ran_at TEXT NOT NULL,
    PRIMARY KEY (view, test)
) STRICT;
CREATE TABLE metric_catalog (
    key        TEXT    PRIMARY KEY,
    title      TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    language   TEXT,
    scope      TEXT    NOT NULL,
    enabled    INTEGER NOT NULL,
    target     REAL,
    trigger    TEXT    NOT NULL,
    toggleable INTEGER NOT NULL,
    category   TEXT
) STRICT;
CREATE TABLE git_tag (
    name TEXT PRIMARY KEY,
    sha TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE work_item (
    ref TEXT PRIMARY KEY,            -- work_item:<provider>:<id>
    provider TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('todo', 'in_progress', 'blocked', 'done', 'canceled')),
    native_state TEXT NOT NULL,
    native TEXT NOT NULL DEFAULT '{}',
    parent_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
, filed_in_thread INTEGER);
CREATE TABLE symbol (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ref TEXT NOT NULL,
    snapshot_id INTEGER NOT NULL,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    container TEXT,
    language TEXT NOT NULL,
    line INTEGER NOT NULL,
    col INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    end_col INTEGER NOT NULL
, start_line INTEGER NOT NULL DEFAULT 0, start_col INTEGER NOT NULL DEFAULT 0);
CREATE TABLE symbol_capture (
    snapshot_id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    files_collected INTEGER NOT NULL,
    files_over_budget INTEGER NOT NULL,
    files_without_server INTEGER NOT NULL,
    captured_at TEXT NOT NULL
, files_failed INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS "ai_result" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    op TEXT NOT NULL,
    role TEXT NOT NULL,
    caller TEXT NOT NULL,
    output_json TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    at TEXT NOT NULL,
    ai_call_id INTEGER REFERENCES ai_call(id) ON DELETE SET NULL,
    UNIQUE (input_hash, provider, model, prompt_version)
);
CREATE TABLE thread_answer (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    lens TEXT,
    spec TEXT,
    params TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    kept_lens TEXT,
    CHECK ((lens IS NULL) <> (spec IS NULL))
);
CREATE TABLE panel_layout (
    panel TEXT PRIMARY KEY,
    position INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0, 1))
);
CREATE TABLE command_proposal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    command TEXT NOT NULL,
    input_json TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL,
    key TEXT NOT NULL,
    preview_json TEXT NOT NULL,
    dry_run_json TEXT,
    decision TEXT NOT NULL
        CHECK (decision IN ('pending', 'approved', 'declined', 'superseded')),
    decided_at TEXT,
    audit_id INTEGER REFERENCES command_audit(id) ON DELETE SET NULL,
    superseded_by INTEGER REFERENCES command_proposal(id) ON DELETE SET NULL
);
CREATE TABLE capability_provider (
    capability TEXT NOT NULL,
    provider TEXT NOT NULL,
    extension TEXT,
    features_json TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    PRIMARY KEY (capability, provider)
);
CREATE TABLE provider_collector_state (
    instance TEXT NOT NULL,
    collector TEXT NOT NULL,
    state_json TEXT,
    status TEXT NOT NULL DEFAULT 'never' CHECK (status IN ('never', 'reading', 'ok', 'error')),
    error TEXT,
    last_read_at TEXT,
    records INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (instance, collector)
) STRICT;
CREATE TABLE asset_state (
    asset TEXT PRIMARY KEY,
    computed_at TEXT NOT NULL,
    events_to INTEGER NOT NULL,
    snapshot_id INTEGER,
    elapsed_ms INTEGER NOT NULL
, mode TEXT CHECK (mode IN ('full', 'incremental')), watermark INTEGER, row_count INTEGER, definition TEXT) STRICT;
CREATE TABLE IF NOT EXISTS "decision" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    question TEXT NOT NULL,
    choice TEXT NOT NULL,
    alternatives_json TEXT NOT NULL DEFAULT '[]',
    confidence TEXT NOT NULL CHECK (confidence IN ('low', 'medium', 'high')),
    why TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    provenance TEXT NOT NULL DEFAULT 'recorded'
        CHECK (provenance IN ('recorded', 'inferred', 'confirmed', 'dismissed')),
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
CREATE TABLE test_case_stat (
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    producer TEXT NOT NULL,
    subject TEXT NOT NULL,
    last_status TEXT NOT NULL,
    last_ms REAL,
    recorded_ms REAL,
    max_ms REAL,
    timed_runs INTEGER NOT NULL DEFAULT 0,
    total_ms REAL NOT NULL DEFAULT 0,
    runs INTEGER NOT NULL,
    failures INTEGER NOT NULL,
    flips INTEGER NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    last_failed_at TEXT,
    last_passed_at TEXT,
    last_run_id INTEGER REFERENCES metric_capture(id) ON DELETE SET NULL,
    PRIMARY KEY (stream_id, branch, producer, subject)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS "model" (
    view        TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    owner       TEXT    NOT NULL,
    version     INTEGER NOT NULL,
    description TEXT    NOT NULL,
    sql         TEXT    NOT NULL,
    compiled_at TEXT    NOT NULL,
    kind        TEXT    NOT NULL DEFAULT 'sql' CHECK (kind IN ('sql', 'entity')),
    materialize TEXT    CHECK (materialize = 'on_change'
                               OR materialize LIKE 'every %'
                               OR materialize LIKE 'incremental %')
) STRICT;
CREATE TABLE event_type_contract (
    event_type TEXT NOT NULL,
    v INTEGER NOT NULL CHECK (v >= 1),
    -- NULL for a core type.
    extension TEXT,
    schema_json TEXT NOT NULL,
    summary TEXT,
    registered INTEGER NOT NULL CHECK (registered IN (0, 1)),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (event_type, v)
) STRICT;
CREATE TABLE plugin_event_retention (
    namespace TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    payload_days INTEGER NOT NULL CHECK (payload_days >= 1),
    content_days INTEGER NOT NULL CHECK (content_days >= 1),
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE ref_kind (
    kind TEXT PRIMARY KEY,
    -- NULL for a core kind.
    extension TEXT,
    label TEXT,
    id_pattern TEXT NOT NULL,
    revisioned INTEGER NOT NULL CHECK (revisioned IN (0, 1)),
    -- JSON array of its `[[prefix:…]]` sugar.
    wikilinks TEXT NOT NULL,
    resolve TEXT,
    page TEXT,
    icon TEXT
, searchable TEXT) STRICT;
CREATE TABLE IF NOT EXISTS "command_audit" (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT    NOT NULL,
    command      TEXT    NOT NULL,
    actor_kind   TEXT    NOT NULL
                 CHECK (actor_kind IN ('human', 'agent', 'lens', 'system', 'effect')),
    actor_id     TEXT,
    thread_id    INTEGER,
    input_json   TEXT    NOT NULL,
    outcome      TEXT    NOT NULL CHECK (outcome IN ('ok', 'denied', 'invalid', 'error')),
    error        TEXT,
    event_id     TEXT,
    inverse_json TEXT,
    undone_by    INTEGER,
    result_json  TEXT
) STRICT;
CREATE TABLE effect_state (
    effect TEXT PRIMARY KEY,
    start_after_seq INTEGER NOT NULL,
    approved_at TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS "plugin_health" (
    plugin TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('provider', 'collector', 'effect')),
    state TEXT NOT NULL CHECK (state IN ('ok', 'failing', 'disabled')),
    reason TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_ok_at TEXT,
    last_error TEXT,
    mean_ms REAL,
    next_due_at TEXT,
    updated_at TEXT NOT NULL,
    repair_item TEXT,
    repair_seq INTEGER,
    PRIMARY KEY (plugin, kind, contribution)
) STRICT;
CREATE TABLE asset_failure (
    asset TEXT PRIMARY KEY,
    failed_at TEXT NOT NULL,
    error TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS "collector_run" (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'error', 'needs_approval', 'skipped')),
    last_run_at TEXT NOT NULL,
    error TEXT,
    -- {"<entity>": <row count>, …} from the last successful run.
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    cursor_json TEXT,
    last_event_id INTEGER,
    PRIMARY KEY (owner, id)
) STRICT;
CREATE TABLE IF NOT EXISTS "effect_run" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    effect TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    origin TEXT NOT NULL DEFAULT 'live'
        CHECK (origin IN ('live', 'retry', 'backfill', 'auto')),
    state TEXT NOT NULL
        CHECK (state IN ('started', 'ok', 'skipped', 'proposed', 'failed')),
    reason TEXT,
    audit_id INTEGER,
    proposal_id INTEGER,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    retry_at TEXT, resend_json TEXT,
    UNIQUE (effect, event_id, attempt)
) STRICT;
CREATE TABLE IF NOT EXISTS "snapshot_op" (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'effort_start', 'effort_end', 'startup',
                          'manual', 'git_refs', 'head_moved', 'run_measured', 'legacy')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id          INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL
) STRICT;
CREATE TABLE effort_evidence_state (
    effort_id       INTEGER PRIMARY KEY REFERENCES effort(id) ON DELETE CASCADE,
    attribution_sig TEXT NOT NULL,
    refreshed_at    TEXT NOT NULL
) STRICT;
CREATE TRIGGER metric_cube_version_ai AFTER INSERT ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER metric_cube_version_au AFTER UPDATE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER metric_cube_version_ad AFTER DELETE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER work_item_follows_task_delete AFTER DELETE ON task
BEGIN
    DELETE FROM work_item WHERE ref = 'work_item:oxplow:tsk' || OLD.id;
END;
CREATE UNIQUE INDEX idx_streams_one_primary ON streams(kind) WHERE kind = 'primary';
CREATE INDEX idx_streams_branch ON streams(branch);
CREATE INDEX idx_threads_stream_sort ON threads(stream_id, sort_index);
CREATE UNIQUE INDEX idx_threads_one_active_per_stream
    ON threads(stream_id) WHERE status = 'active';
CREATE INDEX idx_task_thread_parent ON task(thread_id, parent_id, sort_index);
CREATE INDEX idx_task_thread_status ON task(thread_id, status, sort_index);
CREATE INDEX idx_task_thread_deleted ON task(thread_id, deleted_at, sort_index);
CREATE INDEX idx_task_backlog ON task(deleted_at, sort_index) WHERE thread_id IS NULL;
CREATE INDEX idx_task_link_thread_from ON task_link(thread_id, from_item_id);
CREATE INDEX idx_task_link_thread_to ON task_link(thread_id, to_item_id);
CREATE INDEX idx_task_note_task ON task_note(task_id, created_at);
CREATE INDEX idx_task_note_thread ON task_note(thread_id, created_at);
CREATE INDEX idx_page_visit_time ON page_visit(visited_at DESC);
CREATE INDEX idx_page_visit_kind_id ON page_visit(page_kind, page_id);
CREATE INDEX idx_usage_event_time ON usage_event(occurred_at DESC);
CREATE INDEX idx_code_quality_scan_started ON code_quality_scan(started_at DESC);
CREATE INDEX idx_code_quality_finding_scan ON code_quality_finding(scan_id, path);
CREATE INDEX idx_agent_turn_thread ON agent_turn(thread_id, started_at DESC);
CREATE INDEX idx_agent_turn_open ON agent_turn(thread_id) WHERE ended_at IS NULL;
CREATE INDEX idx_page_visit_thread_time ON page_visit(thread_id, visited_at DESC);
CREATE INDEX idx_streams_archived ON streams(archived_at);
CREATE INDEX idx_threads_archived ON threads(archived_at);
CREATE INDEX idx_wiki_page_updated ON wiki_page(updated_at DESC);
CREATE INDEX idx_page_ref_target ON page_ref(target_kind, target_id);
CREATE INDEX idx_page_ref_source ON page_ref(source_kind, source_id);
CREATE INDEX idx_snapshot_stream ON snapshot(stream_id, created_at DESC);
CREATE INDEX idx_page_ref_snapshot         ON page_ref(local_snapshot_id);
CREATE INDEX idx_comment_stream ON comment(stream_id, status, last_activity_at DESC);
CREATE INDEX idx_comment_thread ON comment(thread_id, last_activity_at DESC);
CREATE INDEX idx_comment_target ON comment(target_kind, target_id);
CREATE INDEX idx_comment_message_comment ON comment_message(comment_id, created_at);
CREATE UNIQUE INDEX search_entry_identity
    ON search_entry (kind, ref_id, COALESCE(stream_id, ''));
CREATE INDEX search_entry_stream ON search_entry (stream_id);
CREATE INDEX idx_agent_nudge_effort
    ON agent_nudge(effort_id, created_at DESC);
CREATE INDEX idx_agent_nudge_thread
    ON agent_nudge(thread_id, created_at DESC);
CREATE INDEX idx_agent_token_usage_effort
    ON agent_token_usage(effort_id, recorded_at DESC);
CREATE INDEX idx_agent_token_usage_thread
    ON agent_token_usage(thread_id, recorded_at DESC);
CREATE INDEX idx_file_snapshot_stream_path ON file_snapshot(stream_id, path, captured_at DESC);
CREATE INDEX idx_file_snapshot_path ON file_snapshot(path, captured_at DESC);
CREATE INDEX idx_file_snapshot_snapshot ON file_snapshot(snapshot_id);
CREATE INDEX idx_effort_attribution_kind_state
    ON effort_attribution(kind, state);
CREATE INDEX idx_metric_capture_stream ON metric_capture(stream_id);
CREATE INDEX idx_metric_capture_captured_at ON metric_capture(captured_at);
CREATE INDEX idx_metric_capture_branch ON metric_capture(branch);
CREATE INDEX idx_metric_capture_version ON metric_capture(closest_vcs_rev);
CREATE INDEX idx_metric_capture_effort ON metric_capture(effort_id);
CREATE INDEX idx_metric_capture_producer ON metric_capture(producer, captured_at DESC);
CREATE INDEX idx_metric_capture_trigger ON metric_capture(trigger, captured_at);
CREATE INDEX idx_metric_capture_thread ON metric_capture(thread_id);
CREATE INDEX idx_fact_measure_capture ON fact(measure_id, capture_id);
CREATE INDEX idx_fact_subject ON fact(subject_kind, subject_ref);
CREATE INDEX idx_fact_capture ON fact(capture_id);
CREATE INDEX idx_metric_spec_scope ON metric_spec(scope);
CREATE INDEX idx_metric_spec_measure ON metric_spec(source_measure);
CREATE INDEX idx_metric_spec_language ON metric_spec(language, category);
CREATE UNIQUE INDEX idx_metric_capture_idempotency
    ON metric_capture(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX idx_fact_measure_path ON fact(measure_id, path);
CREATE INDEX idx_metric_capture_snapshot ON metric_capture(snapshot_id);
CREATE INDEX idx_metric_cube_capture ON metric_cube(capture_id);
CREATE INDEX idx_metric_live_fact_fact ON metric_live_fact(fact_id);
CREATE INDEX idx_dashboard_item_dashboard_sort ON dashboard_item(dashboard_id, sort_index);
CREATE INDEX idx_claim_effort ON claim(effort_id);
CREATE INDEX idx_agent_tool_call_effort ON agent_tool_call(effort_id, tool);
CREATE INDEX idx_ai_call_role_at ON ai_call(role, at);
CREATE INDEX idx_git_commit_time ON git_commit(committed_at);
CREATE INDEX idx_git_commit_file_path ON git_commit_file(path);
CREATE INDEX idx_lsp_diagnostic_file ON lsp_diagnostic(stream_id, language, path);
CREATE INDEX event_log_type_seq ON event_log (type, seq);
CREATE INDEX event_log_stream ON event_log (stream_id, seq) WHERE stream_id IS NOT NULL;
CREATE INDEX event_log_thread ON event_log (thread_id, seq) WHERE thread_id IS NOT NULL;
CREATE INDEX event_log_effort ON event_log (effort_id, seq) WHERE effort_id IS NOT NULL;
CREATE INDEX event_log_snapshot ON event_log (snapshot_id, seq) WHERE snapshot_id IS NOT NULL;
CREATE INDEX idx_agent_turn_snapshot ON agent_turn (snapshot_id) WHERE snapshot_id IS NOT NULL;
CREATE INDEX event_log_turn ON event_log (turn_id, seq) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_effort_work_item ON effort(work_item, started_at DESC);
CREATE INDEX idx_effort_thread ON effort(thread_id, started_at DESC);
CREATE UNIQUE INDEX idx_effort_open_unique ON effort(work_item) WHERE ended_at IS NULL;
CREATE INDEX idx_effort_file_snapshot ON effort_file(local_snapshot_id);
CREATE INDEX event_content_ns_created ON event_content(namespace, created_at);
CREATE UNIQUE INDEX idx_agent_tool_call_event ON agent_tool_call(event_id) WHERE event_id IS NOT NULL;
CREATE INDEX idx_agent_tool_call_turn ON agent_tool_call(turn_id) WHERE turn_id IS NOT NULL;
CREATE UNIQUE INDEX idx_agent_token_usage_cause ON agent_token_usage(cause) WHERE cause IS NOT NULL;
CREATE INDEX idx_agent_nudge_undelivered ON agent_nudge(thread_id, id) WHERE delivered_at IS NULL;
CREATE INDEX event_log_live_payload ON event_log (type, at) WHERE payload_expired_at IS NULL;
CREATE UNIQUE INDEX idx_agent_nudge_cause_kind
    ON agent_nudge(cause, kind, coalesce(effort_id, 0)) WHERE cause IS NOT NULL;
CREATE INDEX idx_code_quality_scan_revision
    ON code_quality_scan(tool, revision, file_filter);
CREATE INDEX idx_work_item_provider ON work_item(provider, deleted_at);
CREATE INDEX idx_work_item_parent ON work_item(parent_ref);
CREATE INDEX idx_symbol_file ON symbol(stream_id, path);
CREATE INDEX idx_symbol_name ON symbol(name);
CREATE UNIQUE INDEX idx_symbol_ref ON symbol(ref);
CREATE INDEX idx_thread_answer_thread ON thread_answer(thread_id, id);
CREATE INDEX idx_command_proposal_pending ON command_proposal(decision, key);
CREATE INDEX idx_decision_effort ON decision(effort_id);
CREATE INDEX event_log_work_item_ref
    ON event_log (json_extract(payload, '$.item.ref'), seq)
    WHERE type = 'work_item.recorded';
CREATE INDEX idx_metric_capture_turn ON metric_capture (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
CREATE INDEX effect_run_retry_due ON effect_run (retry_at) WHERE retry_at IS NOT NULL;
CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_snapshot_op_effort ON snapshot_op (effort_id) WHERE effort_id IS NOT NULL;
