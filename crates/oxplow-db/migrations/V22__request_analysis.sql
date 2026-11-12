-- A request's stored analysis on one stream: what the metric engine computes
-- over its window (the span of its turns there, the files they changed,
-- the captures they made) that a page can't — its metric deltas and its
-- runs' observations (tests, diff coverage, static analysis). Keyed by
-- request and stream: a request can span streams. `sig` is what it was
-- computed from; one that differs from the inputs now is stale.
CREATE TABLE request_analysis (
    request_id INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    start_snapshot_id INTEGER,
    end_snapshot_id INTEGER,
    sig TEXT NOT NULL,
    refreshed_at TEXT NOT NULL,
    PRIMARY KEY (request_id, stream_id)
) WITHOUT ROWID;
CREATE TABLE request_metric_delta (
    request_id INTEGER NOT NULL,
    stream_id INTEGER NOT NULL,
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
    PRIMARY KEY (request_id, stream_id, key),
    FOREIGN KEY (request_id, stream_id) REFERENCES request_analysis(request_id, stream_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE request_observation (
    request_id INTEGER NOT NULL,
    stream_id INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source TEXT NOT NULL,
    metric_value REAL,
    payload_json TEXT,
    local_snapshot_id INTEGER,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, stream_id, seq),
    FOREIGN KEY (request_id, stream_id) REFERENCES request_analysis(request_id, stream_id) ON DELETE CASCADE
) WITHOUT ROWID;
