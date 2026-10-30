-- The schema the migrations built, as one migration, with the rows they
-- seeded. Views aren't here: the datasets compile them at every open.

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
, archived_at TEXT, custom_prompt TEXT, host TEXT NOT NULL DEFAULT 'local');
CREATE TABLE runtime_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL
);
INSERT INTO runtime_state VALUES(1,NULL);
CREATE TABLE threads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    summary TEXT NOT NULL DEFAULT '',
    summary_updated_at TEXT,
    closed_at TEXT,
    custom_prompt TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, archived_at TEXT, mark TEXT
    CHECK (mark IN ('amber', 'red', 'green', 'blue', 'violet')));
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
CREATE TABLE "wiki_page" (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body_path TEXT NOT NULL,
    body_excerpt TEXT NOT NULL DEFAULT '',
    body_size_bytes INTEGER NOT NULL DEFAULT 0,
    files_json TEXT NOT NULL DEFAULT '[]',
    related_notes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, dirs_json TEXT NOT NULL DEFAULT '[]', body_hash TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '');
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
, snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL, start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL, agent_session_id INTEGER
    REFERENCES agent_session(id) ON DELETE SET NULL, model TEXT, effort TEXT, context_tokens INTEGER, context_window INTEGER);
CREATE TABLE "effort_file" (
    effort_id INTEGER NOT NULL REFERENCES "effort"(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('created', 'updated', 'deleted')), local_snapshot_id INTEGER NOT NULL DEFAULT 0, closest_vcs_rev TEXT, vcs_rev_exact INTEGER NOT NULL DEFAULT 0, source TEXT NOT NULL DEFAULT 'claimed'
    CHECK (source IN ('claimed', 'observed')), agent_session_id INTEGER, shared INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (effort_id, path)
);
CREATE TABLE "wiki_page_thread_update" (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    slug TEXT NOT NULL REFERENCES "wiki_page"(slug) ON DELETE CASCADE,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, slug)
);
CREATE TABLE "snapshot" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    revision TEXT
, branch TEXT, tree_hash TEXT);
CREATE TABLE "effort" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    end_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    summary TEXT,
    impacts_json TEXT
, closed_by TEXT
    CHECK (closed_by IN ('commit', 'switch', 'person', 'agent', 'system')), work_item TEXT, title TEXT);
CREATE TABLE search_entry (
    rowid     INTEGER PRIMARY KEY,
    kind      TEXT NOT NULL,
    uri    TEXT NOT NULL,
    stream_id TEXT
, version TEXT);
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
, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL, cause TEXT, delivered_at TEXT, audience TEXT NOT NULL DEFAULT 'agent'
    CHECK (audience IN ('agent', 'person')));
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
CREATE TABLE "file_snapshot" (
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
, capture_scope TEXT NOT NULL DEFAULT 'complete', extension TEXT, fold TEXT);
INSERT INTO measure VALUES(1,'oxplow.complexity','Cyclomatic complexity','count','symbol','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
INSERT INTO measure VALUES(2,'oxplow.fn_length','Function length','lines','symbol','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
INSERT INTO measure VALUES(3,'oxplow.parameter_count','Parameter count','count','symbol','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
INSERT INTO measure VALUES(4,'oxplow.todo','TODO markers','count','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
INSERT INTO measure VALUES(5,'oxplow.coverage','Line coverage','%','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','last');
INSERT INTO measure VALUES(6,'oxplow.test_case','Test case','count','test','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-subject','oxplow-foundation','last');
INSERT INTO measure VALUES(7,'oxplow.lint_hit','Static-analysis hit','count','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','last');
INSERT INTO measure VALUES(8,'oxplow.duplicate_lines','Duplicated lines','lines','symbol','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','last');
INSERT INTO measure VALUES(9,'oxplow.tokens','Agent tokens','count','model','additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(10,'oxplow.cycle_time','Effort cycle time','ms','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(11,'oxplow.ast_hit','AST idiom hits','count','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
INSERT INTO measure VALUES(12,'oxplow.turn','Agent turns','count','model','additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(13,'oxplow.work_item_effort','Efforts per work item','count','work_item','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(14,'oxplow.nudge','Nudges fired','count',NULL,'additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(15,'oxplow.effort_test_outcome','Effort test outcome','count','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(16,'oxplow.test_duration','Test duration','ms','test','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-subject','oxplow-foundation','last');
INSERT INTO measure VALUES(17,'oxplow.cache_tokens','Prompt-cache tokens','count','model','additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(18,'oxplow.cache_usage','Prompt-cache hit ratio','%','model','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(19,'oxplow.effort_tokens','Effort token spend','count','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(20,'oxplow.effort_steering','Effort steering events','count','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(21,'oxplow.effort_time_to_green','Effort time to green','ms','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(22,'oxplow.token_waste','Reverted-effort token waste','count','effort','non-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','sum');
INSERT INTO measure VALUES(23,'oxplow.coverage.branch','Branch coverage','%','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','last');
INSERT INTO measure VALUES(24,'oxplow.coverage.function','Function coverage','%','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','complete','oxplow-foundation','last');
INSERT INTO measure VALUES(25,'oxplow.doc_coverage','Doc coverage','%','file','semi-additive','global',NULL,'1970-01-01T00:00:00.000000Z','1970-01-01T00:00:00.000000Z','per-path','oxplow-foundation','last');
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
, entity_json TEXT, extension TEXT, ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json)), description TEXT);
INSERT INTO dimension VALUES('oxplow.language','Language','categorical',NULL,NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.severity','Severity','categorical',NULL,NULL,'global',1,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.status','Status','categorical',NULL,NULL,'global',1,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.branch','Branch','categorical',NULL,NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.model','Model','categorical','model',NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.agent','Agent','categorical','agent',NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.package','Package','categorical',NULL,NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.test_suite','Test suite','categorical',NULL,NULL,'global',0,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.rule','Rule','categorical',NULL,NULL,'global',1,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.token_kind','Token kind','categorical',NULL,NULL,'global',1,NULL,'oxplow-foundation',NULL,NULL);
INSERT INTO dimension VALUES('oxplow.tests_stat','Test outcome stat','categorical','effort',NULL,'global',1,NULL,'oxplow-foundation',NULL,NULL);
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
, extension TEXT, entity_json TEXT, ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json)), expression TEXT);
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
INSERT INTO metric_cube_epoch VALUES(1,2,0);
CREATE TABLE metric_cube_state (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    last_capture_id INTEGER NOT NULL,
    last_captured_at TEXT NOT NULL,
    PRIMARY KEY (measure_id, stream_id, branch)
) WITHOUT ROWID;
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
, turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL, event_id TEXT, kind TEXT NOT NULL DEFAULT 'other', agent_session_id INTEGER, subagent_id TEXT, subagent_kind TEXT);
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
, input_hash TEXT, request_hash TEXT, response_hash TEXT, run_id TEXT);
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
    is_test INTEGER NOT NULL, staged INTEGER, untracked INTEGER,
    PRIMARY KEY (change_id, path)
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
    updated_at TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0, upstream TEXT, ahead INTEGER NOT NULL DEFAULT 0, behind INTEGER NOT NULL DEFAULT 0,
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
, payload_expired_at TEXT, agent_session_id INTEGER) STRICT;
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
CREATE TABLE event_content (
    hash       TEXT    PRIMARY KEY,
    namespace  TEXT    NOT NULL,
    bytes      BLOB    NOT NULL,
    size       INTEGER NOT NULL,
    created_at TEXT    NOT NULL
) STRICT;
CREATE TABLE "dataset_input" (
    view  TEXT NOT NULL REFERENCES "dataset"(view) ON DELETE CASCADE,
    input TEXT NOT NULL,
    kind  TEXT NOT NULL CHECK (kind IN ('ref', 'source')),
    PRIMARY KEY (view, input)
) STRICT;
CREATE TABLE "dataset_contract" (
    view         TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    -- [{"name", "type", "doc"}] in column order.
    columns_json TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL, key_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (view, version)
) STRICT;
CREATE TABLE "dataset_test" (
    view   TEXT NOT NULL REFERENCES "dataset"(view) ON DELETE CASCADE,
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
    uri TEXT PRIMARY KEY,            -- work_item:<provider>:<id>
    provider TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK (state IN ('todo', 'in_progress', 'blocked', 'done', 'canceled')),
    native_state TEXT NOT NULL,
    native TEXT NOT NULL DEFAULT '{}',
    parent_uri TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
, thread_id INTEGER, rank REAL, closed_at TEXT);
CREATE TABLE symbol (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uri TEXT NOT NULL,
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
, start_line INTEGER NOT NULL DEFAULT 0, start_col INTEGER NOT NULL DEFAULT 0, signature TEXT);
CREATE TABLE symbol_capture (
    snapshot_id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    files_collected INTEGER NOT NULL,
    captured_at TEXT NOT NULL
, files_failed INTEGER NOT NULL DEFAULT 0);
CREATE TABLE "ai_result" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    op TEXT NOT NULL,
    role TEXT NOT NULL,
    caller TEXT NOT NULL,
    output_json TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    at TEXT NOT NULL,
    ai_call_id INTEGER REFERENCES ai_call(id) ON DELETE SET NULL,
    UNIQUE (input_hash, provider, model, request_hash)
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
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)), title TEXT NOT NULL DEFAULT '', source TEXT NOT NULL DEFAULT 'core', available INTEGER NOT NULL DEFAULT 1, chosen_by TEXT, capability_title TEXT NOT NULL DEFAULT '', choosable INTEGER NOT NULL DEFAULT 0, optional INTEGER NOT NULL DEFAULT 0, fields_json TEXT NOT NULL DEFAULT '[]', id_pattern TEXT, description TEXT,
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
, mode TEXT CHECK (mode IN ('full', 'incremental')), watermark INTEGER, row_count INTEGER, definition TEXT, built_from TEXT) STRICT;
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
CREATE TABLE "dataset" (
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
, ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json)), column_ai_context_json TEXT CHECK (column_ai_context_json IS NULL OR json_valid(column_ai_context_json)), relationships_json TEXT CHECK (relationships_json IS NULL OR json_valid(relationships_json))) STRICT;
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
CREATE TABLE "extension_event_retention" (
    namespace TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    payload_days INTEGER NOT NULL CHECK (payload_days >= 1),
    content_days INTEGER NOT NULL CHECK (content_days >= 1),
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE effect_state (
    effect TEXT PRIMARY KEY,
    start_after_seq INTEGER NOT NULL,
    approved_at TEXT NOT NULL
) STRICT;
CREATE TABLE "contribution_health" (
    extension TEXT NOT NULL,
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
    PRIMARY KEY (extension, kind, contribution)
) STRICT;
CREATE TABLE asset_failure (
    asset TEXT PRIMARY KEY,
    failed_at TEXT NOT NULL,
    error TEXT NOT NULL
) STRICT;
CREATE TABLE "collector_run" (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'error', 'needs_approval', 'skipped')),
    last_run_at TEXT NOT NULL,
    error TEXT,
    -- {"<entity>": <row count>, …} from the last successful run.
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    cursor_json TEXT,
    last_event_id INTEGER, run_id TEXT,
    PRIMARY KEY (owner, id)
) STRICT;
CREATE TABLE "effect_run" (
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
    retry_at TEXT, resend_json TEXT, run_id TEXT,
    UNIQUE (effect, event_id, attempt)
) STRICT;
CREATE TABLE effort_evidence_state (
    effort_id       INTEGER PRIMARY KEY REFERENCES effort(id) ON DELETE CASCADE,
    attribution_sig TEXT NOT NULL,
    refreshed_at    TEXT NOT NULL
) STRICT;
CREATE TABLE pending_run (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    since TEXT NOT NULL,
    touched TEXT NOT NULL,
    PRIMARY KEY (owner, id)
) STRICT;
CREATE TABLE bookmark (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uri TEXT NOT NULL,
    page_kind TEXT NOT NULL,
    label TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('thread', 'stream', 'project')),
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL,
    CHECK ((scope = 'thread') = (thread_id IS NOT NULL)),
    CHECK ((scope = 'stream') = (stream_id IS NOT NULL))
) STRICT;
CREATE TABLE once_mark (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES effort(id) ON DELETE CASCADE,
    mark      TEXT    NOT NULL,
    fired_at  TEXT    NOT NULL
) STRICT;
CREATE TABLE hint_stat (
    thread_id         INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    hint              TEXT    NOT NULL,
    evaluated         INTEGER NOT NULL,
    last_evaluated_at TEXT    NOT NULL,
    PRIMARY KEY (thread_id, hint)
) STRICT;
CREATE TABLE table_generation (
    name TEXT PRIMARY KEY,
    gen  INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
CREATE TABLE work_item_comment (
    -- The provider's own id for it (oxplow: `task_note:<id>`).
    id TEXT PRIMARY KEY,
    uri TEXT NOT NULL,
    body TEXT NOT NULL,
    author TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE "claim" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    -- `work_item:<provider>:<id>`; NULL when made on no item.
    work_item TEXT,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    statement TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tests_pass', 'no_behavior_change', 'handles_case', 'other')),
    -- What backs it: `run:<capture id>`, a test name, a file, … NULL = unbacked.
    evidence TEXT,
    created_at TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
, agent_session_id INTEGER, review TEXT CHECK (review IN ('verified', 'dismissed')), provenance TEXT NOT NULL DEFAULT 'recorded'
  CHECK (provenance IN ('recorded', 'inferred')));
CREATE TABLE "decision" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    -- `work_item:<provider>:<id>`; NULL when made on no item.
    work_item TEXT,
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
CREATE TABLE "task_note" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE "work_item_link" (
    from_uri TEXT NOT NULL,
    to_uri TEXT NOT NULL,
    link_type TEXT NOT NULL,
    created_at TEXT NOT NULL, to_kind TEXT NOT NULL DEFAULT 'work_item',
    PRIMARY KEY (from_uri, to_uri, link_type)
);
CREATE TABLE fact_path (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);
CREATE TABLE fact_dims (
    id INTEGER PRIMARY KEY,
    json TEXT NOT NULL UNIQUE
);
CREATE TABLE "fact" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    value REAL NOT NULL,
    -- Ratio components, so roll-ups re-aggregate as Σnum/Σden (never naive-AVG).
    numerator REAL,
    denominator REAL,
    -- What the fact is about (`fact_subject`: kind + logical id) and where
    -- it was at capture (`fact_path` + `line`).
    subject_id INTEGER REFERENCES fact_subject(id),
    path_id INTEGER REFERENCES fact_path(id),
    line INTEGER,
    -- Reported finding metadata (lint/CVE); NULL for pure measurements.
    severity TEXT,
    rule TEXT,
    detail TEXT,
    -- Open conformed-dimension tail (`fact_dims`: its JSON, keyed by
    -- namespaced dimension key).
    dims_id INTEGER REFERENCES fact_dims(id)
, last_capture_id INTEGER);
CREATE TABLE fact_chain (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    from_capture_id INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    PRIMARY KEY (capture_id, measure_id)
) WITHOUT ROWID;
CREATE TABLE fact_path_hold (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES fact_path(id),
    from_capture_id INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    PRIMARY KEY (capture_id, measure_id, path_id)
) WITHOUT ROWID;
CREATE TABLE fact_line (
    fact_id INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
    -- From this capture on (a bound, like `from_capture_id`).
    capture_id INTEGER NOT NULL,
    line INTEGER,
    PRIMARY KEY (fact_id, capture_id)
) WITHOUT ROWID;
CREATE TABLE contribution (
    id TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    kind TEXT NOT NULL,
    local TEXT NOT NULL,
    origin TEXT NOT NULL,
    version TEXT,
    contract_version INTEGER,
    runs_on_json TEXT NOT NULL,
    needs_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    global_name TEXT,
    file TEXT NOT NULL,
    line INTEGER
, order_level INTEGER, description TEXT) STRICT;
CREATE TABLE extension_state (
    extension TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('project', 'personal')),
    area TEXT NOT NULL CHECK (area IN ('state', 'cache')),
    schema_version INTEGER NOT NULL
        CHECK (schema_version > 0 AND schema_version <= 4294967295),
    revision INTEGER NOT NULL CHECK (revision > 0),
    values_json TEXT NOT NULL
        CHECK (json_valid(values_json)
            AND json_type(values_json) = 'object'
            AND length(CAST(values_json AS BLOB)) <= 1048576),
    PRIMARY KEY (extension, scope, area)
) STRICT, WITHOUT ROWID;
CREATE TABLE extension_job_context (
    job_id TEXT PRIMARY KEY NOT NULL REFERENCES extension_job(id),
    originating_run_id TEXT NOT NULL CHECK (length(originating_run_id) = 36),
    context_json TEXT NOT NULL CHECK (COALESCE(
        json_valid(context_json)
        AND length(CAST(context_json AS BLOB)) <= 65536
        AND json_type(context_json) = 'object'
        AND json_type(context_json, '$.schema_version') = 'integer'
        AND json_extract(context_json, '$.schema_version') = 1
        AND json_type(context_json, '$.originating_run_id') = 'text'
        AND json_extract(context_json, '$.originating_run_id') = originating_run_id,
        0
    ))
) STRICT, WITHOUT ROWID;
CREATE TABLE extension_job_attempt (
    job_id TEXT NOT NULL REFERENCES extension_job(id),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 4294967295),
    fence INTEGER NOT NULL CHECK (fence = 2 * attempt - 1),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms >= 0),
    lease_until_ms INTEGER NOT NULL CHECK (lease_until_ms > started_at_ms),
    closed_at_ms INTEGER CHECK (closed_at_ms >= started_at_ms),
    outcome TEXT CHECK (outcome IN ('succeeded', 'failed', 'cancelled', 'interrupted')),
    result_json TEXT CHECK (result_json IS NULL OR
        (json_valid(result_json) AND length(CAST(result_json AS BLOB)) <= 1048576)),
    PRIMARY KEY (job_id, attempt),
    UNIQUE (job_id, fence),
    UNIQUE (job_id, attempt, fence),
    CHECK ((closed_at_ms IS NULL) = (outcome IS NULL)),
    CHECK (result_json IS NULL OR
        (outcome IS NOT NULL AND outcome IN ('succeeded', 'failed', 'cancelled')))
) STRICT, WITHOUT ROWID;
CREATE TABLE extension_job_observation (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed')),
    evidence_json TEXT CHECK (evidence_json IS NULL OR
        (json_valid(evidence_json) AND length(CAST(evidence_json AS BLOB)) <= 1048576)),
    FOREIGN KEY (job_id, attempt, fence)
        REFERENCES extension_job_attempt(job_id, attempt, fence)
) STRICT, WITHOUT ROWID;
CREATE TABLE extension_config (
    extension TEXT NOT NULL CHECK (length(CAST(extension AS BLOB)) BETWEEN 1 AND 128),
    scope TEXT NOT NULL CHECK (scope IN ('project', 'personal')),
    schema_version INTEGER NOT NULL CHECK (schema_version BETWEEN 1 AND 4294967295),
    revision INTEGER NOT NULL CHECK (revision > 0),
    values_json TEXT NOT NULL CHECK (COALESCE(
        json_valid(values_json)
        AND json_type(values_json) = 'object'
        AND length(CAST(values_json AS BLOB)) <= 1048576,
        0
    )),
    PRIMARY KEY (extension, scope)
) STRICT, WITHOUT ROWID;
CREATE TABLE extension_job_output (
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'tool_call', 'tool_result', 'usage')),
    at_ms INTEGER NOT NULL CHECK (at_ms >= 0),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)
        AND length(CAST(content_json AS BLOB)) <= 262144),
    PRIMARY KEY (job_id, attempt, seq),
    FOREIGN KEY (job_id, attempt, fence)
        REFERENCES extension_job_attempt(job_id, attempt, fence)
) STRICT, WITHOUT ROWID;
CREATE TABLE replaceable (
    target TEXT PRIMARY KEY,
    capability TEXT NOT NULL,
    bound INTEGER NOT NULL CHECK (bound IN (0, 1)),
    version INTEGER NOT NULL CHECK (version >= 1),
    -- JSON array of prop names.
    props TEXT NOT NULL,
    label TEXT NOT NULL
, view TEXT NOT NULL DEFAULT '[]') STRICT;
CREATE TABLE "task_link" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_uri TEXT NOT NULL,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from',
        'duplicates', 'supersedes', 'replies_to', 'about')),
    created_at TEXT NOT NULL,
    UNIQUE (from_item_id, to_uri, link_type)
);
CREATE TABLE change_function (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    -- `symbol:<path>/<id>`
    uri TEXT NOT NULL,
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
    PRIMARY KEY (change_id, path, uri)
);
CREATE TABLE "fact_subject" (
    id INTEGER PRIMARY KEY,
    uri TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL
);
CREATE TABLE "extension_job" (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL CHECK (length(CAST(contribution_id AS BLOB)) BETWEEN 1 AND 512),
    package_version TEXT,
    contract_version INTEGER CHECK (contract_version > 0 AND contract_version <= 4294967295),
    operation TEXT NOT NULL CHECK (length(CAST(operation AS BLOB)) BETWEEN 1 AND 256),
    invoker TEXT NOT NULL CHECK (length(CAST(invoker AS BLOB)) BETWEEN 1 AND 256),
    run_id TEXT NOT NULL,
    intent_key TEXT NOT NULL CHECK (length(CAST(intent_key AS BLOB)) BETWEEN 1 AND 128),
    replay_policy TEXT NOT NULL CHECK (replay_policy IN ('never', 'idempotent')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'interrupted', 'succeeded', 'failed', 'cancelled')),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    lease_until_ms INTEGER,
    cancellation_requested_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    revision INTEGER NOT NULL CHECK (revision > 0),
    request_json TEXT NOT NULL CHECK (
        json_valid(request_json) AND length(CAST(request_json AS BLOB)) <= 1048576
    ), deployment_hash TEXT
    CHECK (deployment_hash IS NULL OR length(CAST(deployment_hash AS BLOB)) BETWEEN 1 AND 256),
    UNIQUE (contribution_id, intent_key),
    CHECK (package_version IS NULL OR length(CAST(package_version AS BLOB)) BETWEEN 1 AND 256),
    CHECK (cancellation_requested_at_ms IS NULL OR
        cancellation_requested_at_ms BETWEEN created_at_ms AND updated_at_ms),
    CHECK (replay_policy != 'never' OR attempt <= 1),
    CHECK (
        (state = 'queued' AND lease_until_ms IS NULL
            AND cancellation_requested_at_ms IS NULL AND fence = 2 * attempt
            AND (attempt = 0 OR replay_policy = 'idempotent'))
        OR (state = 'running' AND attempt > 0 AND fence = 2 * attempt - 1
            AND lease_until_ms IS NOT NULL AND lease_until_ms > created_at_ms)
        OR (state = 'interrupted' AND attempt > 0 AND fence = 2 * attempt
            AND lease_until_ms IS NULL)
        OR (state IN ('succeeded', 'failed') AND attempt > 0
            AND fence = 2 * attempt - 1 AND lease_until_ms IS NULL)
        OR (state = 'cancelled' AND cancellation_requested_at_ms IS NOT NULL
            AND lease_until_ms IS NULL AND
            ((attempt = 0 AND fence = 0) OR (attempt > 0 AND fence = 2 * attempt - 1)))
    )
) STRICT, WITHOUT ROWID;
CREATE TABLE "snapshot_op" (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'effort_start', 'effort_end', 'startup',
                          'manual', 'git_refs', 'head_moved', 'run_measured')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id          INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL,
    provider           TEXT,
    contents           INTEGER NOT NULL DEFAULT 1,
    handle             TEXT
) STRICT;
CREATE TABLE metric_capture_exclusion (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('oversize', 'binary', 'unreadable')),
    PRIMARY KEY (capture_id, path)
) WITHOUT ROWID;
CREATE TABLE "extension_run" (
    run_id TEXT PRIMARY KEY,
    parent_run_id TEXT,
    extension TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL,
    version TEXT,
    generation TEXT,
    actor_kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    stream_id INTEGER,
    thread_id INTEGER,
    effort_id INTEGER,
    turn_id INTEGER,
    agent_session_id INTEGER,
    snapshot_id INTEGER,
    cause_event_id TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('pure', 'transactional', 'external')),
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    outcome TEXT NOT NULL
        CHECK (outcome IN ('ok', 'skipped', 'failed', 'denied', 'cancelled')),
    error TEXT,
    wall_ms INTEGER NOT NULL,
    cpu_ms INTEGER NOT NULL,
    peak_bytes INTEGER NOT NULL,
    ai_calls INTEGER NOT NULL,
    scopes_json TEXT NOT NULL,
    decisions_json TEXT NOT NULL
) STRICT;
CREATE TABLE extension_listing (
    name TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    description TEXT NOT NULL,
    path TEXT NOT NULL,
    origin TEXT NOT NULL,
    version TEXT,
    enabled INTEGER NOT NULL,
    purpose TEXT,
    errors_json TEXT NOT NULL,
    warnings_json TEXT NOT NULL
, deployment TEXT NOT NULL DEFAULT 'source', hash TEXT, permissions_json TEXT NOT NULL DEFAULT '{}', pending_json TEXT NOT NULL DEFAULT '[]', shared INTEGER NOT NULL DEFAULT 0) STRICT;
CREATE TABLE agent_session_binding (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_session_id INTEGER NOT NULL REFERENCES agent_session(id) ON DELETE CASCADE,
    native_id        TEXT NOT NULL,
    origin           TEXT NOT NULL CHECK (origin IN ('started', 'switched', 'forked', 'chosen')),
    bound_at         TEXT NOT NULL,
    unbound_at       TEXT
) STRICT;
CREATE TABLE "agent_session" (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id         INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('terminal', 'action')),
    -- A harness registry key: no CHECK on purpose.
    harness           TEXT NOT NULL,
    title             TEXT NOT NULL DEFAULT '',
    resume_session_id TEXT NOT NULL DEFAULT '',
    -- The host it runs on; NULL is the local machine.
    host              TEXT,
    opened_at         TEXT NOT NULL,
    closed_at         TEXT,
    closed_reason     TEXT CHECK (closed_reason IN ('closed', 'thread_closed', 'stream_archived')),
    updated_at        TEXT NOT NULL,
    pid               INTEGER,
    last_seen_at      TEXT,
    native_title      TEXT NOT NULL DEFAULT '',
    fork_next         INTEGER NOT NULL DEFAULT 0 CHECK (fork_next IN (0, 1))
) STRICT;
CREATE TABLE handoff (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    from_actor TEXT NOT NULL CHECK (from_actor IN ('user', 'agent', 'effect', 'job', 'system')),
    to_thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    work_item TEXT,
    body TEXT NOT NULL,
    sent_at TEXT NOT NULL,
    delivered_at TEXT,
    delivered_to_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'open'
        CHECK (state IN ('open', 'taken', 'declined', 'withdrawn')),
    settled_at TEXT,
    settled_by_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    note TEXT,
    CHECK (from_thread_id <> to_thread_id)
);
CREATE TABLE config_key (
    key TEXT PRIMARY KEY,
    doc TEXT NOT NULL,
    schema_json TEXT NOT NULL
) STRICT;
CREATE TABLE stream_head (
    stream_id INTEGER PRIMARY KEY,
    head_sha TEXT NOT NULL
);
CREATE TABLE extension_deployment (
    name TEXT NOT NULL,
    hash TEXT NOT NULL,
    deployed_at TEXT NOT NULL,
    source TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0, permissions_json TEXT NOT NULL DEFAULT '{}', build_run TEXT, "commit" TEXT, dirty INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (name, hash)
);
CREATE TABLE ui_preference (
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (scope, key)
) STRICT;
CREATE TABLE resource (
    resource TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    label TEXT NOT NULL,
    -- JSON object: param name -> { required, pattern }.
    params TEXT NOT NULL,
    additional INTEGER NOT NULL CHECK (additional IN (0, 1)),
    resolve TEXT,
    title TEXT,
    searchable TEXT,
    icon TEXT,
    -- JSON { prefix, param }, or NULL.
    wikilink TEXT
, description TEXT) STRICT;
CREATE TABLE "page_ref" (
  source_uri      TEXT NOT NULL,
  target_uri      TEXT NOT NULL,
  source_resource TEXT NOT NULL,
  target_resource TEXT NOT NULL,
  ref_type        TEXT NOT NULL,
  source_extra    TEXT,
  local_snapshot_id INTEGER,
  closest_vcs_rev TEXT,
  vcs_rev_exact   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (source_uri, target_uri, ref_type)
);
CREATE TABLE "comment" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    target_resource TEXT NOT NULL,
    target_uri TEXT NOT NULL,
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
    last_activity_at TEXT NOT NULL,
    resolved_at TEXT,
    context_chain_json TEXT NOT NULL DEFAULT '[]',
    referenced_json TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE "comment_message" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES "comment"(id) ON DELETE CASCADE,
    -- Free-form, e.g. 'user' or 'agent'.
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE "change" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working` | `turn` | `range`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working', 'turn', 'range')),
    -- The sha, the effort id, the turn id, `<base>..<head>` for a range,
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
CREATE TABLE terminal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE thread_context_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('page', 'text')),
    source_uri TEXT,
    label TEXT,
    text TEXT,
    added_at TEXT NOT NULL,
    delivered_at TEXT,
    delivered_session_id INTEGER,
    CHECK (kind = 'text' OR (source_uri IS NOT NULL AND text IS NULL)),
    CHECK (kind = 'page' OR text IS NOT NULL)
) STRICT;
CREATE TABLE page (
    uri TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    label TEXT,
    icon TEXT,
    category TEXT,
    keywords TEXT,
    params_json TEXT NOT NULL,
    renders_json TEXT NOT NULL,
    is_default INTEGER NOT NULL,
    pinned INTEGER NOT NULL,
    hidden INTEGER NOT NULL,
    tool TEXT,
    needs_json TEXT NOT NULL,
    source TEXT,
    deployment TEXT NOT NULL
, running_uri TEXT) STRICT;
CREATE TABLE page_answer (
    page_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    question TEXT NOT NULL,
    about TEXT,
    params_json TEXT NOT NULL,
    PRIMARY KEY (page_uri, position)
) STRICT;
CREATE TABLE page_param (
    page_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    role TEXT NOT NULL,
    label TEXT,
    default_json TEXT,
    options_json TEXT NOT NULL,
    PRIMARY KEY (page_uri, position)
) STRICT;
CREATE TABLE "command_audit" (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT    NOT NULL,
    command      TEXT    NOT NULL,
    actor_kind   TEXT    NOT NULL
                 CHECK (actor_kind IN ('human', 'agent', 'page', 'component', 'system', 'effect', 'job')),
    actor_id     TEXT,
    thread_id    INTEGER,
    input_json   TEXT    NOT NULL,
    outcome      TEXT    NOT NULL CHECK (outcome IN ('ok', 'denied', 'invalid', 'error')),
    error        TEXT,
    event_id     TEXT,
    inverse_json TEXT,
    undone_by    INTEGER,
    result_json  TEXT,
    scopes_json TEXT,
    session_id INTEGER,
    run_id TEXT,
    via TEXT
) STRICT;
CREATE UNIQUE INDEX idx_streams_one_primary ON streams(kind) WHERE kind = 'primary';
CREATE INDEX idx_streams_branch ON streams(branch);
CREATE INDEX idx_threads_stream_sort ON threads(stream_id, sort_index);
CREATE INDEX idx_task_thread_parent ON task(thread_id, parent_id, sort_index);
CREATE INDEX idx_task_thread_status ON task(thread_id, status, sort_index);
CREATE INDEX idx_task_thread_deleted ON task(thread_id, deleted_at, sort_index);
CREATE INDEX idx_task_backlog ON task(deleted_at, sort_index) WHERE thread_id IS NULL;
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
CREATE INDEX idx_snapshot_stream ON snapshot(stream_id, created_at DESC);
CREATE UNIQUE INDEX search_entry_identity
    ON search_entry (kind, uri, COALESCE(stream_id, ''));
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
CREATE INDEX idx_metric_capture_stream ON metric_capture(stream_id);
CREATE INDEX idx_metric_capture_captured_at ON metric_capture(captured_at);
CREATE INDEX idx_metric_capture_branch ON metric_capture(branch);
CREATE INDEX idx_metric_capture_version ON metric_capture(closest_vcs_rev);
CREATE INDEX idx_metric_capture_effort ON metric_capture(effort_id);
CREATE INDEX idx_metric_capture_producer ON metric_capture(producer, captured_at DESC);
CREATE INDEX idx_metric_capture_trigger ON metric_capture(trigger, captured_at);
CREATE INDEX idx_metric_capture_thread ON metric_capture(thread_id);
CREATE INDEX idx_metric_spec_scope ON metric_spec(scope);
CREATE INDEX idx_metric_spec_measure ON metric_spec(source_measure);
CREATE INDEX idx_metric_spec_language ON metric_spec(language, category);
CREATE UNIQUE INDEX idx_metric_capture_idempotency
    ON metric_capture(idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX idx_metric_capture_snapshot ON metric_capture(snapshot_id);
CREATE INDEX idx_metric_cube_capture ON metric_cube(capture_id);
CREATE INDEX idx_metric_live_fact_fact ON metric_live_fact(fact_id);
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
CREATE INDEX idx_effort_thread ON effort(thread_id, started_at DESC);
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
CREATE INDEX idx_work_item_parent ON work_item(parent_uri);
CREATE INDEX idx_symbol_file ON symbol(stream_id, path);
CREATE INDEX idx_symbol_name ON symbol(name);
CREATE INDEX idx_command_proposal_pending ON command_proposal(decision, key);
CREATE INDEX idx_metric_capture_turn ON metric_capture (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
CREATE INDEX effect_run_retry_due ON effect_run (retry_at) WHERE retry_at IS NOT NULL;
CREATE UNIQUE INDEX bookmark_owner
    ON bookmark (uri, scope, coalesce(thread_id, 0), coalesce(stream_id, 0));
CREATE INDEX idx_effort_work_item ON effort(work_item, started_at DESC)
    WHERE work_item IS NOT NULL;
CREATE UNIQUE INDEX idx_effort_open_per_thread ON effort(thread_id)
    WHERE ended_at IS NULL;
CREATE UNIQUE INDEX idx_once_mark ON once_mark(thread_id, coalesce(effort_id, 0), mark);
CREATE INDEX idx_claim_effort ON claim(effort_id);
CREATE INDEX idx_decision_effort ON decision(effort_id);
CREATE INDEX idx_task_note_task ON task_note(task_id, created_at);
CREATE INDEX idx_agent_turn_open_session ON agent_turn(agent_session_id)
    WHERE ended_at IS NULL;
CREATE INDEX idx_event_log_agent_session ON event_log(agent_session_id, seq)
    WHERE agent_session_id IS NOT NULL;
CREATE INDEX idx_fact_measure_capture ON fact(measure_id, capture_id);
CREATE INDEX idx_fact_subject ON fact(subject_id);
CREATE INDEX idx_fact_capture ON fact(capture_id);
CREATE INDEX idx_fact_chain_measure ON fact_chain(measure_id, capture_id);
CREATE INDEX idx_fact_path_hold_path ON fact_path_hold(measure_id, path_id, capture_id);
CREATE INDEX idx_fact_measure_path ON fact(measure_id, path_id, capture_id);
CREATE INDEX extension_job_observation_history
    ON extension_job_observation (job_id, observed_at_ms, id);
CREATE INDEX idx_task_link_from ON task_link (from_item_id);
CREATE INDEX idx_task_link_to ON task_link (to_uri);
CREATE INDEX extension_job_recovery ON extension_job (state, updated_at_ms, id);
CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_snapshot_op_effort ON snapshot_op (effort_id) WHERE effort_id IS NOT NULL;
CREATE INDEX extension_run_by_contribution ON extension_run (contribution, started_at);
CREATE INDEX extension_run_by_start ON extension_run (started_at);
CREATE INDEX idx_agent_session_binding_session
    ON agent_session_binding(agent_session_id, bound_at);
CREATE UNIQUE INDEX idx_agent_session_binding_open
    ON agent_session_binding(agent_session_id) WHERE unbound_at IS NULL;
CREATE INDEX idx_agent_session_thread ON agent_session(thread_id, opened_at DESC);
CREATE INDEX idx_agent_session_open ON agent_session(thread_id) WHERE closed_at IS NULL;
CREATE INDEX idx_agent_session_resume ON agent_session(resume_session_id)
    WHERE resume_session_id <> '';
CREATE INDEX idx_handoff_to ON handoff(to_thread_id, state, sent_at);
CREATE INDEX idx_handoff_from ON handoff(from_thread_id, sent_at);
CREATE INDEX idx_page_ref_target ON page_ref(target_uri);
CREATE INDEX idx_page_ref_source ON page_ref(source_uri);
CREATE INDEX idx_page_ref_target_resource ON page_ref(target_resource);
CREATE INDEX idx_page_ref_source_resource ON page_ref(source_resource);
CREATE INDEX idx_page_ref_snapshot ON page_ref(local_snapshot_id);
CREATE INDEX idx_comment_stream ON comment(stream_id, status, last_activity_at DESC);
CREATE INDEX idx_comment_thread ON comment(thread_id, last_activity_at DESC);
CREATE INDEX idx_comment_target ON comment(target_uri);
CREATE INDEX work_item_comment_uri ON work_item_comment (uri);
CREATE UNIQUE INDEX idx_symbol_uri ON symbol (stream_id, uri);
CREATE INDEX event_log_work_item_uri
    ON event_log (json_extract(payload, '$.item.uri'), seq);
CREATE INDEX terminal_stream ON terminal(stream_id, position);
CREATE INDEX thread_context_item_waiting
    ON thread_context_item (thread_id, delivered_at);
CREATE INDEX command_audit_refused ON command_audit (at)
    WHERE outcome IN ('invalid', 'denied');
CREATE TRIGGER metric_cube_version_ai AFTER INSERT ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER metric_cube_version_au AFTER UPDATE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER metric_cube_version_ad AFTER DELETE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
CREATE TRIGGER extension_job_context_origin
BEFORE INSERT ON extension_job_context
WHEN NEW.originating_run_id IS NOT (SELECT run_id FROM extension_job WHERE id = NEW.job_id)
BEGIN
    SELECT RAISE(ABORT, 'extension job context origin');
END;
CREATE TRIGGER extension_job_context_immutable
BEFORE UPDATE ON extension_job_context
BEGIN
    SELECT RAISE(ABORT, 'extension job context is immutable');
END;
CREATE TRIGGER extension_job_attempt_immutable
BEFORE UPDATE ON extension_job_attempt
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.attempt IS NOT OLD.attempt
    OR NEW.fence IS NOT OLD.fence OR NEW.started_at_ms IS NOT OLD.started_at_ms
    OR NEW.lease_until_ms < OLD.lease_until_ms
    OR OLD.closed_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'extension job closed or immutable attempt');
END;
CREATE TRIGGER extension_job_observation_immutable
BEFORE UPDATE ON extension_job_observation
BEGIN
    SELECT RAISE(ABORT, 'extension job observations are append-only');
END;
CREATE TRIGGER extension_config_revision
BEFORE UPDATE ON extension_config
WHEN NEW.extension IS NOT OLD.extension OR NEW.scope IS NOT OLD.scope
    OR NEW.revision != OLD.revision + 1
    OR NEW.schema_version < OLD.schema_version
BEGIN
    SELECT RAISE(ABORT, 'extension configuration namespace or revision');
END;
CREATE TRIGGER extension_job_output_append_only
BEFORE UPDATE ON extension_job_output
BEGIN
    SELECT RAISE(ABORT, 'extension job output is append-only');
END;
CREATE TRIGGER extension_job_output_open_attempt
BEFORE INSERT ON extension_job_output
WHEN NOT EXISTS (
    SELECT 1 FROM extension_job_attempt
     WHERE job_id = NEW.job_id AND attempt = NEW.attempt AND fence = NEW.fence
       AND closed_at_ms IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'extension job attempt is not open');
END;
CREATE TRIGGER extension_job_immutable
BEFORE UPDATE ON extension_job
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled')
    OR NEW.id IS NOT OLD.id
    OR NEW.contribution_id IS NOT OLD.contribution_id
    OR ((NEW.package_version IS NOT OLD.package_version
            OR NEW.deployment_hash IS NOT OLD.deployment_hash)
        AND NOT (OLD.state = 'queued' AND NEW.state = 'queued'
            AND NEW.attempt = OLD.attempt AND NEW.fence = OLD.fence))
    OR NEW.contract_version IS NOT OLD.contract_version
    OR NEW.operation IS NOT OLD.operation
    OR NEW.invoker IS NOT OLD.invoker
    OR NEW.run_id IS NOT OLD.run_id
    OR NEW.intent_key IS NOT OLD.intent_key
    OR NEW.replay_policy IS NOT OLD.replay_policy
    OR NEW.created_at_ms IS NOT OLD.created_at_ms
    OR NEW.request_json IS NOT OLD.request_json
    OR NEW.updated_at_ms < OLD.updated_at_ms
    OR NEW.revision != OLD.revision + 1
    OR (OLD.cancellation_requested_at_ms IS NOT NULL
        AND NEW.cancellation_requested_at_ms IS NOT OLD.cancellation_requested_at_ms)
BEGIN
    SELECT RAISE(ABORT, 'extension job immutable identity or revision');
END;
