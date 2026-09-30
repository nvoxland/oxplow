-- P5.C6: the symbols the running language servers report for each
-- stream's files — a current-tree index, restated per changed file by the
-- symbol collector at each snapshot (`ref` = symbol:<path>/<name>@snap:<id>,
-- the snapshot it was read at).
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
);
CREATE INDEX idx_symbol_file ON symbol(stream_id, path);
CREATE INDEX idx_symbol_name ON symbol(name);

-- One row per snapshot the collector handled: how many changed files it
-- asked about, and how many it skipped (over the bound, or no running
-- server for their language).
CREATE TABLE symbol_capture (
    snapshot_id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    files_collected INTEGER NOT NULL,
    files_over_budget INTEGER NOT NULL,
    files_without_server INTEGER NOT NULL,
    captured_at TEXT NOT NULL
);
