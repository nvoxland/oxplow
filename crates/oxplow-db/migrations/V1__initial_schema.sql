-- Initial oxplow schema (Rust rewrite).
--
-- This collapses the 50 incremental migrations from the prior TS
-- implementation into a single initial-state schema. The Electron
-- → Tauri rewrite is a clean break — there is no upgrade path from
-- the old SQLite DB. Users start fresh.
--
-- Naming note: the legacy schema used "batch" / "batches" for what
-- the domain model now calls "thread". The Rust rewrite uses the
-- domain name end-to-end, so tables are renamed accordingly.
--
-- Id note: every external entity id is a SQLite autoincrement INTEGER
-- (rowid). The 3-letter type prefix (`str`, `thr`, `tsk`, …) lives
-- only in the application layer (serde/Display), never in the DB — so
-- all id columns and the FKs that point at them are INTEGER.

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE streams (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('primary', 'worktree')),
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
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
);
CREATE UNIQUE INDEX idx_streams_one_primary ON streams(kind) WHERE kind = 'primary';
CREATE INDEX idx_streams_branch ON streams(branch);

CREATE TABLE runtime_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL
);
INSERT INTO runtime_state (id, current_stream_id) VALUES (1, NULL);

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
);
CREATE INDEX idx_threads_stream_sort ON threads(stream_id, sort_index);
-- At most one ACTIVE (writer) thread per stream — mirrors the TS
-- invariant. Other threads sit in the queued bucket.
CREATE UNIQUE INDEX idx_threads_one_active_per_stream
    ON threads(stream_id) WHERE status = 'active';

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
    acceptance_criteria TEXT,
    status TEXT NOT NULL CHECK (status IN ('ready', 'in_progress', 'blocked', 'done', 'canceled', 'archived')),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'urgent')),
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL CHECK (created_by IN ('user', 'agent', 'system')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    deleted_at TEXT,
    -- Semantic origin (vs. created_by which is the writer).
    author TEXT CHECK (author IN ('user', 'agent')),
    category TEXT,
    tags TEXT
);
CREATE INDEX idx_task_thread_parent ON task(thread_id, parent_id, sort_index);
CREATE INDEX idx_task_thread_status ON task(thread_id, status, sort_index);
CREATE INDEX idx_task_thread_deleted ON task(thread_id, deleted_at, sort_index);
CREATE INDEX idx_task_backlog ON task(deleted_at, sort_index) WHERE thread_id IS NULL;

CREATE TABLE task_link (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from', 'duplicates', 'supersedes', 'replies_to')),
    created_at TEXT NOT NULL,
    CHECK (from_item_id <> to_item_id)
);
CREATE INDEX idx_task_link_thread_from ON task_link(thread_id, from_item_id);
CREATE INDEX idx_task_link_thread_to ON task_link(thread_id, to_item_id);

-- Internal event log. `id` is a non-entity string id (not surfaced to
-- agents/users), so it stays TEXT; the FK to threads is INTEGER.
CREATE TABLE task_event (
    id TEXT PRIMARY KEY,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    item_id INTEGER REFERENCES task(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('user', 'agent', 'system')),
    actor_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_task_event_thread_item ON task_event(thread_id, item_id, created_at);

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
CREATE INDEX idx_task_note_task ON task_note(task_id, created_at);
CREATE INDEX idx_task_note_thread ON task_note(thread_id, created_at);

-- Wiki notes — durable, file-backed knowledge captured by agent
-- exploration. Body lives at .oxplow/notes/<slug>.md; this table
-- holds metadata + a search index.
CREATE TABLE wiki_note (
    slug TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body_path TEXT NOT NULL,
    body_excerpt TEXT NOT NULL DEFAULT '',
    body_size_bytes INTEGER NOT NULL DEFAULT 0,
    file_refs_json TEXT NOT NULL DEFAULT '[]',
    related_notes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_wiki_note_updated ON wiki_note(updated_at DESC);

-- FTS5 mirror of wiki_note.body for `mcp__oxplow__search_note_bodies`.
CREATE VIRTUAL TABLE wiki_note_fts USING fts5(slug UNINDEXED, title, body_excerpt);

-- Page-visit / usage telemetry — drives the "recent" rails.
CREATE TABLE page_visit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    page_kind TEXT NOT NULL,
    page_id TEXT NOT NULL,
    visited_at TEXT NOT NULL,
    duration_ms INTEGER
);
CREATE INDEX idx_page_visit_time ON page_visit(visited_at DESC);
CREATE INDEX idx_page_visit_kind_id ON page_visit(page_kind, page_id);

CREATE TABLE usage_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);
CREATE INDEX idx_usage_event_time ON usage_event(occurred_at DESC);

-- Code-quality scan + finding tables.
CREATE TABLE code_quality_scan (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    started_at TEXT NOT NULL,
    ended_at TEXT,
    error TEXT
);
CREATE INDEX idx_code_quality_scan_started ON code_quality_scan(started_at DESC);

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
CREATE INDEX idx_code_quality_finding_scan ON code_quality_finding(scan_id, path);

-- File-snapshot store: captures content-addressed blob hashes per
-- (path, captured_at) tuple. Used for cross-turn diffs and the
-- snapshots panel.
CREATE TABLE file_snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    blob_hash TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT NOT NULL,
    oversize INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_file_snapshot_stream_path ON file_snapshot(stream_id, path, captured_at DESC);
CREATE INDEX idx_file_snapshot_path ON file_snapshot(path, captured_at DESC);

CREATE TABLE agent_turn (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
    prompt TEXT NOT NULL,
    answer TEXT,
    session_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT
);
CREATE INDEX idx_agent_turn_thread ON agent_turn(thread_id, started_at DESC);
CREATE INDEX idx_agent_turn_task ON agent_turn(task_id, started_at DESC);
CREATE INDEX idx_agent_turn_open ON agent_turn(thread_id) WHERE ended_at IS NULL;

-- Durable hook-event log. Surfaces in the HookEventsPage and feeds
-- the stop-hook pipeline / write-guard / filing-enforcement deciders.
CREATE TABLE hook_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    session_id TEXT,
    payload_json TEXT NOT NULL,
    received_at TEXT NOT NULL
);
CREATE INDEX idx_hook_event_thread_time ON hook_event(thread_id, received_at DESC);
CREATE INDEX idx_hook_event_kind_time ON hook_event(kind, received_at DESC);

-- Per-thread agent status snapshot. Updated on hook events and pane
-- lifecycle. One row per (thread_id, pane_target).
CREATE TABLE agent_status (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    pane_target TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('idle', 'running', 'awaiting_user', 'stopped', 'error')),
    detail TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, pane_target)
);
CREATE INDEX idx_agent_status_state ON agent_status(state, updated_at DESC);

-- Task commit attribution: which commits the agent produced
-- under a given task. Powers the "commits this task shipped" rail.
CREATE TABLE task_commit (
    task_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (task_id, commit_sha)
);
CREATE INDEX idx_task_commit_sha ON task_commit(commit_sha);

-- Task effort tracking. An "effort" is one continuous push of
-- agent work on a task — bounded by snapshots at start and end. The
-- effort_file rows attribute concrete file changes.
CREATE TABLE task_effort (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    start_snapshot_id INTEGER REFERENCES file_snapshot(id) ON DELETE SET NULL,
    end_snapshot_id INTEGER REFERENCES file_snapshot(id) ON DELETE SET NULL,
    summary TEXT
);
CREATE INDEX idx_task_effort_task ON task_effort(task_id, started_at DESC);
CREATE INDEX idx_task_effort_thread ON task_effort(thread_id, started_at DESC);

CREATE TABLE task_effort_file (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('created', 'updated', 'deleted')),
    PRIMARY KEY (effort_id, path)
);

CREATE TABLE task_effort_turn (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    turn_id INTEGER NOT NULL REFERENCES agent_turn(id) ON DELETE CASCADE,
    PRIMARY KEY (effort_id, turn_id)
);

-- Per-thread last-seen wiki-note timestamps for the freshness badge.
CREATE TABLE wiki_note_thread_update (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    slug TEXT NOT NULL REFERENCES wiki_note(slug) ON DELETE CASCADE,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, slug)
);
