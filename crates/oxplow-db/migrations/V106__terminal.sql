-- A terminal is a row: a shell in a stream's worktree, named by its URI
-- (`oxplow:oxplow/terminal?id=trm<n>`). Its process is the daemon's, keyed
-- by that URI; closing the row ends the shell, and a stream's rows go with
-- the stream. `position` orders a stream's terminals.
CREATE TABLE terminal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX terminal_stream ON terminal(stream_id, position);
